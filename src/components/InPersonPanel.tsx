import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { IconCheck, IconMic, IconPause, IconPlay, IconRecord, IconStop, IconUsers, IconWarning, IconX } from "./Icons";
import "./InPersonPanel.css";

/** What `get_my_voice` / `record_my_voice` report (commands/my_voice.rs). */
export interface MyVoiceStatus {
    enrolled: boolean;
    speech_seconds: number | null;
    created_at: string | null;
    speaker_model_installed: boolean;
}

interface CommandResult<T> { ok: boolean; data?: T; error?: { code: string; message: string } }

/** Read aloud for the voice setup: about 20 s, with varied sounds. */
export const VOICE_PASSAGE =
    "I'm setting up my voice so Taurscribe can tell when I'm the one speaking in a meeting. " +
    "On Thursday afternoon we moved the budget review to the large room upstairs, and everyone agreed " +
    "to bring their numbers, questions and quick sketches. If the projector fails again, we'll share " +
    "screens from a laptop and keep going.";

/** Settings key: the voice setup was offered once (set up or skipped). */
export const VOICE_PROMPTED_KEY = "in_person_voice_prompted";

interface Props {
    isRecording: boolean;
    /** The recording running is an in-person meeting (not a dictation or call). */
    isInPersonRecording: boolean;
    isPaused: boolean;
    isProcessing: boolean;
    /** Speaker separation (Nemotron) is downloaded. */
    separatorInstalled: boolean;
    onStart: () => void;
    onStop: () => void;
    onPause: () => void;
    onResume: () => void;
    onOpenModelSettings: () => void;
    /** Closes the section (when not recording). */
    onClose: () => void;
    /** Recording time so far without pauses, and the live transcript so far:
     * the section can be closed and reopened (another tab) mid-recording. */
    getRecordingMs: () => number;
    getLiveTranscript: () => string;
    /** settings.json, for remembering that the voice setup was offered. */
    getSetting: (key: string) => Promise<unknown>;
    setSetting: (key: string, value: unknown) => Promise<void>;
}

function formatClock(ms: number) {
    const s = Math.floor(ms / 1000);
    const h = Math.floor(s / 3600);
    const mm = String(Math.floor((s % 3600) / 60)).padStart(2, "0");
    const ss = String(s % 60).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/**
 * In-person recording, at the top of the Meetings tab: records a meeting in a
 * room on one microphone. On Stop it closes; the backend tells the speakers
 * apart and the meeting appears in the list like a call (Meetings shows the
 * processing and opens the review).
 */
export function InPersonPanel({
    isRecording, isInPersonRecording, isPaused, isProcessing, separatorInstalled,
    onStart, onStop, onPause, onResume, onOpenModelSettings, onClose, getRecordingMs, getLiveTranscript, getSetting, setSetting,
}: Props) {
    const [voice, setVoice] = useState<MyVoiceStatus | null>(null);
    const [setupOpen, setSetupOpen] = useState(false);
    const [lines, setLines] = useState<string[]>([]);
    const [level, setLevel] = useState(0);
    const [elapsedMs, setElapsedMs] = useState(0);
    const feedRef = useRef<HTMLDivElement>(null);
    const recording = isRecording && isInPersonRecording;
    const busyElsewhere = isRecording && !isInPersonRecording;

    useEffect(() => {
        invoke<MyVoiceStatus>("get_my_voice").then(setVoice).catch(() => setVoice(null));
    }, []);

    // Live transcript and level of this recording only.
    useEffect(() => {
        if (!recording) return;
        const sofar = getLiveTranscript().trim();
        setLines(sofar ? [sofar] : []);
        const offChunk = listen<{ text: string }>("transcription-chunk", (e) => {
            const text = e.payload.text?.trim();
            if (text) setLines((prev) => [...prev, text]);
        });
        const offLevel = listen<number>("audio-level", (e) => setLevel(e.payload));
        return () => {
            offChunk.then((f) => f());
            offLevel.then((f) => f());
            setLevel(0);
        };
    }, [recording]);

    // Recording time, not counting pauses.
    useEffect(() => {
        if (!recording) return;
        setElapsedMs(getRecordingMs());
        const id = setInterval(() => setElapsedMs(getRecordingMs()), 250);
        return () => clearInterval(id);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [recording]);

    useEffect(() => {
        feedRef.current?.scrollTo({ top: feedRef.current.scrollHeight, behavior: "smooth" });
    }, [lines]);

    const stop = () => {
        onStop();
        onClose();
    };
    const closeSetup = async (startAfter: boolean) => {
        setSetupOpen(false);
        await setSetting(VOICE_PROMPTED_KEY, true);
        if (startAfter) onStart();
    };
    // The voice setup is offered once, the first time this records.
    const start = async () => {
        if (voice && !voice.enrolled && !(await getSetting(VOICE_PROMPTED_KEY))) {
            setSetupOpen(true);
            return;
        }
        onStart();
    };

    if (setupOpen) {
        return (
            <div className="inperson-panel" data-testid="inperson-panel">
                <VoiceSetup
                    voice={voice}
                    onSaved={(v) => setVoice(v)}
                    onDone={(startAfter) => void closeSetup(startAfter)}
                    onOpenModelSettings={onOpenModelSettings}
                />
            </div>
        );
    }

    return (
        <div className="inperson-panel" data-testid="inperson-panel">
            {!separatorInstalled && (
                <div className="inperson-note inperson-note--warn" data-testid="inperson-separator-missing">
                    <IconWarning size={14} />
                    <span>Telling speakers apart needs the Meeting speakers download. Without it, voices are grouped roughly.</span>
                    <button id="inperson-separator-download-btn" data-testid="inperson-separator-download-btn" type="button" className="inperson-link" onClick={onOpenModelSettings}>Download</button>
                </div>
            )}

            <div className={`inperson-stage${recording ? " inperson-stage--live" : ""}`}>
                <div className="inperson-stage-head">
                    <div className="inperson-title">
                        <IconUsers size={16} />
                        <span>{recording ? (isPaused ? "Paused" : "Recording in person") : "In-person meeting"}</span>
                        {recording && <span className="inperson-clock" data-testid="inperson-clock">{formatClock(elapsedMs)}</span>}
                    </div>
                    <div className="inperson-actions">
                        {recording ? (
                            <>
                                <button id="inperson-pause-btn" data-testid="inperson-pause-btn" type="button" className="inperson-btn" onClick={isPaused ? onResume : onPause}>
                                    {isPaused ? <IconPlay size={13} /> : <IconPause size={13} />}
                                    <span>{isPaused ? "Resume" : "Pause"}</span>
                                </button>
                                <button id="inperson-stop-btn" data-testid="inperson-stop-btn" type="button" className="inperson-btn inperson-btn--stop" onClick={stop}>
                                    <IconStop size={13} />
                                    <span>Stop</span>
                                </button>
                            </>
                        ) : (
                            <button id="inperson-record-btn" data-testid="inperson-record-btn"
                                type="button"
                                className="inperson-btn inperson-btn--record"

                                onClick={() => void start()}
                                disabled={busyElsewhere || isProcessing}
                                title={busyElsewhere ? "Stop the current recording first" : undefined}
                            >
                                <IconRecord size={13} />
                                <span>Start recording</span>
                            </button>
                        )}
                        {!recording && (
                            <button id="inperson-close-btn" data-testid="inperson-close-btn" type="button" className="inperson-icon-btn" onClick={onClose} aria-label="Close">
                                <IconX size={14} />
                            </button>
                        )}
                    </div>
                </div>

                {recording && (
                    <div className="inperson-meter" aria-label="Microphone level">
                        <IconMic size={12} />
                        <span className="inperson-meter-track">
                            <span className="inperson-meter-fill" style={{ width: `${Math.round(Math.min(1, level) * 100)}%` }} />
                        </span>
                        <span className="inperson-meter-hint">
                            {isPaused ? "Paused" : level < 0.08 ? "Quiet — put the laptop near the middle of the table" : "Hearing the room"}
                        </span>
                    </div>
                )}

                {recording ? (
                    <div className="inperson-feed" ref={feedRef} data-testid="inperson-feed">
                        {lines.length === 0 && <p className="inperson-placeholder">Listening… the transcript appears here as people talk.</p>}
                        {lines.map((line, i) => <p key={i} className="inperson-line">{line}</p>)}
                    </div>
                ) : (
                    <p className="inperson-intro">
                        Records everyone in the room on this computer's microphone; put it where everyone can be heard.
                        When you stop, Taurscribe works out who said what (up to about 5 people work best) and the meeting appears below.
                    </p>
                )}
            </div>

            {!recording && voice && (
                <div className="inperson-voice-row" data-testid="inperson-voice-row">
                    <span>
                        {voice.enrolled ? "Your voice is set up: you're labelled “You” in the transcript." : "Your voice isn't set up: everyone is labelled Speaker 1, 2, …"}
                    </span>
                    <button id="inperson-voice-setup-open-btn" data-testid="inperson-voice-setup-open-btn" type="button" className="inperson-link" onClick={() => setSetupOpen(true)} disabled={isRecording}>
                        {voice.enrolled ? "Redo" : "Set up"}
                    </button>
                </div>
            )}
        </div>
    );
}

/** One-time "read this aloud" setup that saves the user's voiceprint. */
function VoiceSetup({ voice, onSaved, onDone, onOpenModelSettings }: {
    voice: MyVoiceStatus | null;
    onSaved: (v: MyVoiceStatus) => void;
    onDone: (startAfter: boolean) => void;
    onOpenModelSettings: () => void;
}) {
    const [phase, setPhase] = useState<"idle" | "recording" | "done" | "error">("idle");
    const [progress, setProgress] = useState({ elapsed: 0, total: 1, level: 0 });
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        if (phase !== "recording") return;
        const off = listen<{ elapsed_ms: number; total_ms: number; level: number }>("my-voice-progress", (e) =>
            setProgress({ elapsed: e.payload.elapsed_ms, total: e.payload.total_ms, level: e.payload.level }),
        );
        return () => { off.then((f) => f()); };
    }, [phase]);

    const record = async () => {
        setError(null);
        setProgress({ elapsed: 0, total: 1, level: 0 });
        setPhase("recording");
        try {
            const res = await invoke<CommandResult<MyVoiceStatus>>("record_my_voice");
            if (res.ok && res.data) {
                onSaved(res.data);
                setPhase("done");
            } else {
                setError(res.error?.message ?? "Couldn't save your voice");
                setPhase("error");
            }
        } catch (e) {
            setError(String(e));
            setPhase("error");
        }
    };

    const modelMissing = voice != null && !voice.speaker_model_installed;
    return (
        <div className="inperson-setup" data-testid="inperson-voice-setup">
            <h3 className="inperson-setup-title">Set up your voice</h3>
            <p className="inperson-setup-desc">
                Everyone in the room is on one microphone. Read this once, and Taurscribe labels you “You” in every in-person meeting.
                It takes about 20 seconds and stays on this computer.
            </p>
            {modelMissing ? (
                <div className="inperson-note inperson-note--warn">
                    <IconWarning size={14} />
                    <span>This needs the speaker recognition model.</span>
                    <button id="inperson-speaker-model-download-btn" data-testid="inperson-speaker-model-download-btn" type="button" className="inperson-link" onClick={onOpenModelSettings}>Download it</button>
                </div>
            ) : (
                <blockquote className={`inperson-passage${phase === "recording" ? " inperson-passage--live" : ""}`}>{VOICE_PASSAGE}</blockquote>
            )}
            {phase === "recording" && (
                <div className="inperson-meter">
                    <IconMic size={12} />
                    <span className="inperson-meter-track">
                        <span className="inperson-meter-fill inperson-meter-fill--progress" style={{ width: `${Math.min(100, (progress.elapsed / progress.total) * 100)}%` }} />
                    </span>
                    <span className="inperson-meter-hint">{Math.max(0, Math.ceil((progress.total - progress.elapsed) / 1000))} s</span>
                </div>
            )}
            {phase === "done" && (
                <div className="inperson-saved"><IconCheck size={14} /><span>Your voice is saved.</span></div>
            )}
            {phase === "error" && error && (
                <div className="inperson-note inperson-note--warn"><IconWarning size={14} /><span>{error}</span></div>
            )}
            <div className="inperson-setup-actions">
                {phase === "done" ? (
                    <button id="inperson-voice-continue" data-testid="inperson-voice-continue" type="button" className="inperson-btn inperson-btn--record" onClick={() => onDone(true)}>
                        <IconRecord size={13} /><span>Start recording</span>
                    </button>
                ) : (
                    <>
                        <button id="inperson-voice-record" data-testid="inperson-voice-record"
                            type="button"
                            className="inperson-btn inperson-btn--record"

                            onClick={() => void record()}
                            disabled={phase === "recording" || modelMissing}
                        >
                            <IconMic size={13} /><span>{phase === "recording" ? "Listening…" : phase === "error" ? "Try again" : "Start reading"}</span>
                        </button>
                        <button id="inperson-voice-skip" data-testid="inperson-voice-skip" type="button" className="inperson-btn" onClick={() => onDone(false)} disabled={phase === "recording"}>
                            Skip
                        </button>
                    </>
                )}
            </div>
        </div>
    );
}
