import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { SpeakerNameSuggestion } from "../types/meetings";
import { IconCheck, IconPlay, IconStop, IconX } from "./Icons";

interface Props {
    meetingId: number;
    /** A speaker was labelled: reload the meeting. */
    onLabelled: () => void;
}

/**
 * "Speaker 2 sounds like Dana — is it?" for a meeting's unnamed speakers whose
 * voice is close to someone in the Speaker Vault but not close enough to be
 * labelled automatically. Yes labels them and keeps their voice as another
 * sample of the person; No is remembered.
 */
export function SpeakerSuggestions({ meetingId, onLabelled }: Props) {
    const [items, setItems] = useState<SpeakerNameSuggestion[]>([]);
    const [busy, setBusy] = useState<string | null>(null);
    const [error, setError] = useState("");
    const [playing, setPlaying] = useState<string | null>(null);
    const audio = useRef<HTMLAudioElement | null>(null);

    const stop = () => {
        audio.current?.pause();
        audio.current = null;
        setPlaying(null);
    };

    useEffect(() => {
        let current = true;
        setItems([]);
        setError("");
        invoke<SpeakerNameSuggestion[]>("suggest_speaker_names", { meetingId })
            .then((list) => { if (current) setItems(list); })
            .catch(() => { if (current) setItems([]); });
        return () => { current = false; stop(); };
    }, [meetingId]);

    const play = (s: SpeakerNameSuggestion) => {
        if (playing === s.speaker_id) return stop();
        stop();
        if (!s.clip_path) return;
        const a = new Audio(convertFileSrc(s.clip_path));
        audio.current = a;
        setPlaying(s.speaker_id);
        const done = () => { if (audio.current === a) { audio.current = null; setPlaying(null); } };
        a.onended = done;
        a.onerror = done;
        a.play().catch(done);
    };

    const answer = async (s: SpeakerNameSuggestion, yes: boolean) => {
        if (busy) return;
        setBusy(s.speaker_id);
        if (playing === s.speaker_id) stop();
        try {
            await invoke(yes ? "accept_voice_sample" : "reject_voice_sample", {
                personId: s.person_id, meetingId, speakerId: s.speaker_id,
            });
            setItems((prev) => prev.filter((x) => x.speaker_id !== s.speaker_id));
            setError("");
            if (yes) onLabelled();
        } catch (err) {
            setError(String(err));
        } finally {
            setBusy(null);
        }
    };

    if (items.length === 0 && !error) return null;
    return (
        <div className="speaker-suggestions" data-testid="speaker-suggestions" role="region" aria-label="Who might these speakers be">
            {items.map((s) => (
                <div key={s.speaker_id} className="speaker-suggestion" data-testid={`speaker-suggestion-${s.speaker_id}`}>
                    <button
                        type="button"
                        className="speaker-suggestion-play"
                        onClick={() => play(s)}
                        disabled={!s.clip_path}
                        aria-label={playing === s.speaker_id ? "Stop" : `Play ${s.speaker_name}`}
                    >
                        {playing === s.speaker_id ? <IconStop size={11} /> : <IconPlay size={11} />}
                    </button>
                    <span className="speaker-suggestion-text">
                        <strong>{s.speaker_name}</strong> sounds like <strong>{s.person_name}</strong>
                        <span className="speaker-suggestion-pct"> · {Math.round(s.similarity * 100)}% match</span>
                    </span>
                    <button
                        type="button"
                        className="detail-tool-btn speaker-suggestion-yes"
                        onClick={() => answer(s, true)}
                        disabled={busy !== null}
                        data-testid={`speaker-suggestion-yes-${s.speaker_id}`}
                    >
                        <IconCheck size={11} /><span>It's {s.person_name}</span>
                    </button>
                    <button
                        type="button"
                        className="detail-tool-btn"
                        onClick={() => answer(s, false)}
                        disabled={busy !== null}
                        data-testid={`speaker-suggestion-no-${s.speaker_id}`}
                    >
                        <IconX size={11} /><span>Not {s.person_name}</span>
                    </button>
                </div>
            ))}
            {error && <p className="detail-toolbar-error" role="alert">{error}</p>}
        </div>
    );
}
