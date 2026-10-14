import { claimAudioPlayback } from "../utils/audioPlayback";
import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { VoiceSampleSuggestion, Voiceprint } from "../types/meetings";
import { open } from "@tauri-apps/plugin-dialog";
import { IconCheck, IconPlay, IconStop, IconTrash, IconX } from "./Icons";

/** How many suggestions "Find more samples" shows per step. */
export const SUGGESTION_PAGE = 10;

interface Props {
    personId: string;
    personName: string;
    /** "find": speakers in past meetings who may be this person. "samples": their stored voice samples. */
    mode: "find" | "samples";
    /** Something changed in the vault (a sample added or removed). */
    onChanged?: () => void;
    onClose: () => void;
}

const formatDate = (iso: string) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
};
const seconds = (ms: number) => `${Math.max(1, Math.round(ms / 1000))}s of speech`;

/** One shared player: starting a clip stops the previous one. */
function usePlayer() {
    const audio = useRef<HTMLAudioElement | null>(null);
    const [playing, setPlaying] = useState<string | null>(null);
    const stop = () => {
        audio.current?.pause();
        audio.current = null;
        setPlaying(null);
    };
    const toggle = (key: string, path: string | null) => {
        if (playing === key) return stop();
        stop();
        if (!path) return;
        const a = new Audio(convertFileSrc(path));
        audio.current = a;
        setPlaying(key);
        const done = () => { if (audio.current === a) { audio.current = null; setPlaying(null); } };
        a.onended = done;
        a.onerror = done;
        claimAudioPlayback(a, done);
        a.play().catch(done);
    };
    useEffect(() => () => { audio.current?.pause(); }, []);
    return { playing, toggle, stop };
}

export function VoiceBankPanel({ personId, personName, mode, onChanged, onClose }: Props) {
    const player = usePlayer();
    const [suggestions, setSuggestions] = useState<VoiceSampleSuggestion[]>([]);
    const [samples, setSamples] = useState<Voiceprint[]>([]);
    const [limit, setLimit] = useState(SUGGESTION_PAGE);
    /** The last fetch filled its page, so the server may have more. Answers shrink the list without refetching. */
    const [hasMore, setHasMore] = useState(false);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState("");
    /** Key of the row whose Add / Not them / Remove is in flight. */
    const [busy, setBusy] = useState<string | null>(null);
    const generatedCandidates = useRef(new Set<string>());
    const sampleMutation = useRef(false);
    const sampleGeneration = useRef(0);
    const [candidates, setCandidates] = useState<string[]>([]);
    const [confirmRemove, setConfirmRemove] = useState<number | null>(null);
    const request = useRef(0);

    const load = async (nextLimit = limit) => {
        const id = ++request.current;
        setLoading(true);
        try {
            if (mode === "find") {
                const found = await invoke<VoiceSampleSuggestion[]>("find_voice_samples", { personId, limit: nextLimit });
                if (id === request.current) { setSuggestions(found); setHasMore(found.length >= nextLimit); }
            } else {
                const list = await invoke<Voiceprint[]>("list_voiceprints", { personId });
                if (id === request.current) setSamples(list);
            }
            if (id === request.current) setError("");
        } catch (err) {
            if (id === request.current) setError(String(err));
        } finally {
            if (id === request.current) setLoading(false);
        }
    };

    const discardPreview = (path?: string) => {
        const paths = path ? [path] : [...generatedCandidates.current];
        for (const file of paths) if (generatedCandidates.current.delete(file)) {
            void invoke("discard_generated_voice_sample", { path: file }).catch(console.warn);
        }
    };
    useEffect(() => {
        setCandidates([]); sampleGeneration.current++; void load();
        return () => { sampleGeneration.current++; discardPreview(); };
    }, [personId, mode]); // eslint-disable-line react-hooks/exhaustive-deps

    const keyOf = (s: VoiceSampleSuggestion) => `${s.meeting_id}:${s.speaker_id}`;

    const answer = async (s: VoiceSampleSuggestion, add: boolean) => {
        const key = keyOf(s);
        if (busy) return;
        setBusy(key);
        if (player.playing === key) player.stop();
        try {
            await invoke(add ? "accept_voice_sample" : "reject_voice_sample", {
                personId, meetingId: s.meeting_id, speakerId: s.speaker_id,
            });
            setSuggestions((prev) => prev.filter((x) => keyOf(x) !== key));
            setError("");
            if (add) onChanged?.();
        } catch (err) {
            setError(String(err));
        } finally {
            setBusy(null);
        }
    };

    const remove = async (v: Voiceprint) => {
        if (confirmRemove !== v.id) { setConfirmRemove(v.id); return; }
        setConfirmRemove(null);
        setBusy(String(v.id));
        if (player.playing === `v${v.id}`) player.stop();
        try {
            await invoke("delete_voiceprint", { personId, voiceprintId: v.id });
            setSamples((prev) => prev.filter((x) => x.id !== v.id));
            setError("");
            onChanged?.();
        } catch (err) {
            setError(String(err));
        } finally {
            setBusy(null);
        }
    };

    const prepareSample = async (generate: boolean) => {
        if (sampleMutation.current || busy || candidates.length >= 3) return;
        sampleMutation.current = true;
        const generation = sampleGeneration.current;
        setBusy("prepare"); setError(""); player.stop();
        try {
            const path = generate ? await invoke<string>("generate_vault_voice_sample", { personId, excludedPaths: candidates })
                : await open({ multiple: false, filters: [{ name: "Voice recording (5–60 seconds)", extensions: ["wav", "mp3", "m4a", "flac", "ogg"] }] });
            if (generation !== sampleGeneration.current) {
                if (generate && typeof path === "string") void invoke("discard_generated_voice_sample", { path }).catch(console.warn);
                return;
            }
            if (typeof path === "string") { if (generate) generatedCandidates.current.add(path); setCandidates(previous => previous.includes(path) ? previous : [...previous, path]); }
        } catch (reason) { setError(String(reason)); }
        finally { sampleMutation.current = false; setBusy(null); }
    };
    const addSample = async (candidate: string) => {
        if (!candidate || busy || sampleMutation.current) return;
        sampleMutation.current = true;
        setBusy("add"); player.stop();
        try {
            await invoke("add_vault_voice_sample", { personId, path: candidate });
            generatedCandidates.current.delete(candidate); setCandidates(previous => previous.filter(path => path !== candidate)); setError(""); await load(); onChanged?.();
        } catch (reason) { setError(String(reason)); }
        finally { sampleMutation.current = false; setBusy(null); }
    };

    const showMore = () => {
        const next = limit + SUGGESTION_PAGE;
        setLimit(next);
        void load(next);
    };

    return (
        <div className="voicebank-panel" data-testid={`voicebank-panel-${personId}`} role="region" aria-label={mode === "find" ? `More voice samples for ${personName}` : `Voice samples of ${personName}`}>
            <div className="voicebank-head">
                <span className="voicebank-title">
                    {mode === "find" ? `Is this ${personName}?` : `${personName}'s voice samples`}
                </span>
                <button type="button" className="vault-action-btn" data-testid="voicebank-close-btn" onClick={() => { player.stop(); onClose(); }} aria-label="Close">
                    <IconX size={12} />
                </button>
            </div>
            {mode === "find" && (
                <p className="voicebank-hint">
                    Speakers from other meetings who sound like {personName}, closest first. Listen, then add the ones that are
                    really them: each one becomes another sample of their voice. "Not them" is remembered.
                </p>
            )}

            {mode === "samples" && <div className="voicebank-new-sample">
                <p className="voicebank-hint">Generate a new clip from this person's saved meeting speech, or choose a 5–60 second recording. Listen and confirm it contains only {personName} before adding it to their bank.</p>
                <button type="button" className="vault-action-btn" data-testid="voicebank-generate-sample" disabled={busy !== null || candidates.length >= 3} onClick={() => prepareSample(true)}>{candidates.length ? "Generate another sample" : "Generate sample"}</button>
                <button type="button" className="vault-action-btn" data-testid="voicebank-choose-sample" disabled={busy !== null || candidates.length >= 3} onClick={() => prepareSample(false)}>Choose recording</button>
                {candidates.map((candidate, index) => <div key={candidate} className="voicebank-candidate" role="group" aria-label={`Preview sample ${index + 1}`}>
                    <strong>Sample {index + 1} · not added yet</strong>
                    <div className="voicebank-candidate-actions">
                    <button type="button" className="vault-action-btn" data-testid="voicebank-preview-sample" onClick={() => player.toggle(`candidate:${candidate}`, candidate)}>{player.playing === `candidate:${candidate}` ? "Stop preview" : "Listen"}</button>
                    <button type="button" className="vault-action-btn" data-testid="voicebank-confirm-sample" disabled={busy !== null} onClick={() => addSample(candidate)}>This is {personName} — add sample</button>
                    <button type="button" className="vault-action-btn" disabled={busy !== null} onClick={() => { player.stop(); discardPreview(candidate); setCandidates(previous => previous.filter(path => path !== candidate)); }}>Discard</button>
                    </div>
                </div>)}
            </div>}
            {error && <p className="voicebank-error" role="alert" data-testid="voicebank-error">{error}</p>}
            {loading && (mode === "find" ? suggestions.length === 0 : samples.length === 0) && (
                <p className="voicebank-empty">{mode === "find" ? "Listening through past meetings…" : "Loading…"}</p>
            )}

            {mode === "find" && !loading && !error && suggestions.length === 0 && !hasMore && (
                <p className="voicebank-empty" data-testid="voicebank-no-suggestions">No other speakers in your meetings sound like {personName}.</p>
            )}

            {mode === "find" && suggestions.map((s) => {
                const key = keyOf(s);
                const pct = Math.round(s.similarity * 100);
                return (
                    <div key={key} className="voicebank-row" data-testid={`voicebank-suggestion-${s.meeting_id}-${s.speaker_id}`}>
                        <button
                            type="button"
                            className={`vault-action-btn ${player.playing === key ? "active" : ""}`}
                            onClick={() => player.toggle(key, s.clip_path)}
                            disabled={!s.clip_path}
                            aria-label={player.playing === key ? "Stop" : `Play ${s.speaker_name} from ${s.meeting_title}`}
                            data-testid={`voicebank-play-${s.meeting_id}-${s.speaker_id}`}
                        >
                            {player.playing === key ? <IconStop size={12} /> : <IconPlay size={12} />}
                        </button>
                        <div className="voicebank-info">
                            <div className="voicebank-line">
                                <span className="voicebank-meeting">{s.meeting_title}</span>
                                <span className="voicebank-meta">{formatDate(s.created_at)} · {s.speaker_name} · {seconds(s.speech_ms)}</span>
                            </div>
                            {s.preview && <div className="voicebank-preview">“{s.preview}”</div>}
                            <div className="voicebank-match" title={`${pct}% voice match`}>
                                <span className="voicebank-match-track"><span className="voicebank-match-fill" style={{ width: `${pct}%` }} /></span>
                                <span>{pct}% match</span>
                            </div>
                        </div>
                        <div className="voicebank-actions">
                            <button
                                type="button"
                                className="vault-action-btn voicebank-add"
                                onClick={() => answer(s, true)}
                                disabled={busy !== null}
                                data-testid={`voicebank-add-${s.meeting_id}-${s.speaker_id}`}
                            >
                                <IconCheck size={12} /><span>Add</span>
                            </button>
                            <button
                                type="button"
                                className="vault-action-btn"
                                onClick={() => answer(s, false)}
                                disabled={busy !== null}
                                data-testid={`voicebank-reject-${s.meeting_id}-${s.speaker_id}`}
                            >
                                <IconX size={12} /><span>Not them</span>
                            </button>
                        </div>
                    </div>
                );
            })}
            {mode === "find" && hasMore && (
                <button type="button" className="vault-action-btn voicebank-more" onClick={showMore} disabled={loading} data-testid="voicebank-show-more">
                    Show more
                </button>
            )}

            {mode === "samples" && samples.map((v, i) => (
                <div key={v.id} className="voicebank-row" data-testid={`voicebank-sample-${v.id}`}>
                    <button
                        type="button"
                        className={`vault-action-btn ${player.playing === `v${v.id}` ? "active" : ""}`}
                        onClick={() => player.toggle(`v${v.id}`, v.clip_path)}
                        disabled={!v.clip_path}
                        aria-label={v.clip_path ? `Play sample ${i + 1}` : "Clip missing"}
                    >
                        {player.playing === `v${v.id}` ? <IconStop size={12} /> : <IconPlay size={12} />}
                    </button>
                    <div className="voicebank-info">
                        <div className="voicebank-line">
                            <span className="voicebank-meeting">{v.meeting_title ?? "Voice recording"}</span>
                            <span className="voicebank-meta">{formatDate(v.created_at)}{v.clip_path ? "" : " · clip missing"}</span>
                        </div>
                    </div>
                    <div className="voicebank-actions">
                        <button
                            type="button"
                            className={`vault-action-btn danger ${confirmRemove === v.id ? "active" : ""}`}
                            onClick={() => remove(v)}
                            disabled={busy !== null || samples.length <= 1}
                            title={samples.length <= 1 ? "Their only sample: delete the person instead" : "Remove this sample"}
                            data-testid={`voicebank-remove-${v.id}`}
                        >
                            <IconTrash size={12} />{confirmRemove === v.id && <span>Remove?</span>}
                        </button>
                    </div>
                </div>
            ))}
        </div>
    );
}
