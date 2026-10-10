import { claimAudioPlayback } from "../utils/audioPlayback";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import "./SpeakerVaultModal.css";

/** These are previews of labelled audio, not separated sources or enrolled voiceprints. */
export function SpeakerPreviewModal({ meetingId, speakerId, speakerName, initialPath, onClose }: {
    meetingId: number; speakerId: string; speakerName: string; initialPath?: string | null; onClose: () => void;
}) {
    const [paths, setPaths] = useState<string[]>(initialPath ? [initialPath] : []);
    const [separationReady, setSeparationReady] = useState(false);
    const [separated, setSeparated] = useState<string[]>([]);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState("");
    const owned = useRef(new Set<string>());
    const alive = useRef(true);
    const generating = useRef(false);
    const players = useRef(new Set<HTMLAudioElement>());
    useEffect(() => {
        alive.current = true;
        void invoke<boolean>("voice_separation_available").then(ready => { if (alive.current) setSeparationReady(ready); }).catch(console.warn);
        return () => {
            alive.current = false;
            for (const player of players.current) { player.pause(); player.removeAttribute("src"); player.load(); }
            for (const path of owned.current) void invoke("discard_generated_voice_sample", { path }).catch(console.warn);
            owned.current.clear();
        };
    }, []);
    const generate = async () => {
        if (generating.current || paths.length >= 3) return;
        generating.current = true; setBusy(true); setError("");
        try {
            const path = await invoke<string>("generate_meeting_speaker_preview", { meetingId, speakerId, excludedPaths: paths });
            if (!alive.current) { void invoke("discard_generated_voice_sample", { path }).catch(console.warn); return; }
            owned.current.add(path);
            setPaths(previous => previous.includes(path) ? previous : [...previous, path]);
        } catch (err) { if (alive.current) setError(String(err)); }
        finally { generating.current = false; if (alive.current) setBusy(false); }
    };
    const isolate = async (path: string) => {
        if (generating.current) return;
        generating.current = true; setBusy(true); setError("");
        for (const player of players.current) player.pause();
        try {
            const results = await invoke<string[]>("separate_meeting_sample", { path });
            if (!alive.current) { for (const path of results) void invoke("discard_generated_voice_sample", { path }).catch(console.warn); return; }
            for (const path of results) owned.current.add(path);
            setSeparated(results);
        } catch (err) { if (alive.current) setError(String(err)); }
        finally { generating.current = false; if (alive.current) setBusy(false); }
    };
    const downloadSeparation = async () => {
        if (generating.current) return;
        generating.current = true; setBusy(true); setError("");
        try { await invoke("download_model", { modelId: "voice-separation" }); if (alive.current) setSeparationReady(true); }
        catch (err) { if (alive.current) setError(String(err)); }
        finally { generating.current = false; if (alive.current) setBusy(false); }
    };
    const content = <div className="speaker-vault-overlay" onClick={onClose}>
        <section className="speaker-vault-modal" role="dialog" aria-modal="true" aria-label={`Voice previews for ${speakerName}`} onClick={e => e.stopPropagation()}>
            <header className="vault-header"><h2 className="vault-title">{speakerName} · Voice previews</h2><button type="button" className="vault-action-btn" onClick={onClose} aria-label="Close voice previews">Close</button></header>
            <div className="vault-body speaker-preview-body">
                <p>Compare another section of this speaker’s labelled audio. Overlapping voices or echo may still be audible. Samples are added to a person’s bank from the Speaker Vault.</p>
                {paths.map((path, index) => <div className="speaker-preview-card" key={path}><strong>Sample {index + 1}</strong>
                    <audio controls src={convertFileSrc(path)} ref={node => { if (node) players.current.add(node); }}
                        onPlay={event => { claimAudioPlayback(event.currentTarget); for (const player of players.current) if (player !== event.currentTarget) player.pause(); }} />
                    {separationReady && <button type="button" className="turn-clip-btn" disabled={busy} onClick={() => isolate(path)}>Isolate overlapping voices</button>}
                </div>)}
                <button type="button" className="turn-clip-btn" onClick={generate} disabled={busy || paths.length >= 3}>{busy ? "Preparing sample…" : "Generate another sample"}</button>
                {!separationReady && <button type="button" className="turn-clip-btn" disabled={busy} onClick={downloadSeparation}>{busy ? "Downloading/preparing…" : "Download optional voice separation · 29 MB"}</button>}
                <small>Experimental two-voice separation. Listen to both results to identify the speaker; it may introduce artifacts. <a href="https://huggingface.co/dellusional/sepformer-wsj02mix-int8-onnx" target="_blank" rel="noreferrer">Model and credits (Apache 2.0)</a></small>
                {separated.map((path, index) => <div className="speaker-preview-card" key={path}><strong>Isolated voice {index + 1}</strong>
                    <audio controls src={convertFileSrc(path)} ref={node => { if (node) players.current.add(node); }} onPlay={event => claimAudioPlayback(event.currentTarget)} />
                </div>)}
                {error && <p role="alert">{error}</p>}
            </div>
        </section>
    </div>;
    return typeof document !== "undefined" && document.body ? createPortal(content, document.body) : content;
}
