import { claimAudioPlayback } from "../utils/audioPlayback";
import { createPortal } from "react-dom";
import React, { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { SpeakerVaultRecord } from "../types/meetings";
import { IconX, IconPlay, IconStop, IconTrash, IconEdit, IconCheck, IconUsers, IconRefresh, IconSearch } from "./Icons";
import { useSpeakerModelInstalled } from "../hooks/useSpeakerModelInstalled";
import { VoiceBankPanel } from "./VoiceBankPanel";
import "./SpeakerVaultModal.css";

interface SpeakerVaultModalProps {
    isOpen: boolean;
    onClose: () => void;
    onSpeakerUpdated?: () => void;
    /** Opens Settings → Models (where the speaker recognition model is downloaded). */
    onOpenModelSettings?: () => void;
}

export const SpeakerVaultModal: React.FC<SpeakerVaultModalProps> = ({
    isOpen,
    onClose,
    onSpeakerUpdated,
    onOpenModelSettings,
}) => {
    // The vault is voiceprints: it does not work without the speaker model.
    const { installed: speakerModelInstalled } = useSpeakerModelInstalled(isOpen);
    const [speakers, setSpeakers] = useState<SpeakerVaultRecord[]>([]);
    const [loading, setLoading] = useState(false);
    const [editingSpeakerId, setEditingSpeakerId] = useState<string | null>(null);
    const [newName, setNewName] = useState("");
    const [playingSnippetId, setPlayingSnippetId] = useState<string | null>(null);
    /** The person whose Delete was clicked once and now awaits confirmation. */
    const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
    const [actionError, setActionError] = useState("");
    /** The person whose voice-bank panel is open: find more samples, or their samples. */
    const [voiceBank, setVoiceBank] = useState<{ personId: string; mode: "find" | "samples" } | null>(null);

    const audioRef = useRef<HTMLAudioElement | null>(null);

    const loadVault = async () => {
        setLoading(true);
        try {
            const list = await invoke<SpeakerVaultRecord[]>("list_speaker_vault");
            setSpeakers(list);
        } catch (err) {
            console.error("Failed to load speaker vault:", err);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        if (isOpen) {
            loadVault();
        } else {
            stopAudio();
        }
    }, [isOpen]);

    const stopAudio = () => {
        if (audioRef.current) {
            audioRef.current.pause();
            audioRef.current = null;
        }
        setPlayingSnippetId(null);
    };

    const handlePlaySnippet = (speaker: SpeakerVaultRecord) => {
        if (playingSnippetId === speaker.id) {
            stopAudio();
            return;
        }
        stopAudio();
        if (!speaker.snippet_path) return;

        try {
            const assetUrl = convertFileSrc(speaker.snippet_path);
            const audio = new Audio(assetUrl);
            audioRef.current = audio;
            setPlayingSnippetId(speaker.id);

            audio.onended = () => {
                if (audioRef.current !== audio) return;
                setPlayingSnippetId(null);
                audioRef.current = null;
            };

            audio.onerror = () => {
                if (audioRef.current !== audio) return;
                setPlayingSnippetId(null);
                audioRef.current = null;
            };

            claimAudioPlayback(audio, () => { if (audioRef.current === audio) { audioRef.current = null; setPlayingSnippetId(null); } });
            audio.play().catch((e) => {
                if (audioRef.current !== audio) return;
                console.error("Failed to play audio snippet:", e);
                setPlayingSnippetId(null);
            });
        } catch (e) {
            console.error("Failed to load snippet:", e);
        }
    };

    const handleStartRename = (speaker: SpeakerVaultRecord) => {
        setEditingSpeakerId(speaker.id);
        setNewName(speaker.name);
    };

    const handleSaveRename = async (speaker: SpeakerVaultRecord) => {
        if (!newName.trim() || newName.trim() === speaker.name) {
            setEditingSpeakerId(null);
            return;
        }

        try {
            await invoke("rename_vault_speaker", {
                speakerId: speaker.id,
                newName: newName.trim(),
            });
            setEditingSpeakerId(null);
            setActionError("");
            loadVault();
            onSpeakerUpdated?.();
        } catch (err) {
            console.error("Failed to rename speaker:", err);
            setActionError(`Couldn't rename ${speaker.name}: ${err}`);
        }
    };

    /** First click asks for confirmation; the second deletes. */
    const handleDeleteSpeaker = async (speaker: SpeakerVaultRecord) => {
        if (confirmDeleteId !== speaker.id) {
            setConfirmDeleteId(speaker.id);
            return;
        }
        setConfirmDeleteId(null);
        if (playingSnippetId === speaker.id) stopAudio();
        try {
            await invoke("delete_speaker_from_vault", { speakerId: speaker.id });
            setActionError("");
            loadVault();
            onSpeakerUpdated?.();
        } catch (err) {
            console.error("Failed to delete speaker from vault:", err);
            setActionError(`Couldn't delete ${speaker.name}: ${err}`);
        }
    };

    const handleCycleVaultSnippet = async (speaker: SpeakerVaultRecord) => {
        try {
            const [nextPath] = await invoke<[string, number]>("cycle_vault_speaker_snippet", {
                speakerId: speaker.id,
            });
            setSpeakers((prev) =>
                prev.map((s) => {
                    if (s.id === speaker.id) {
                        return {
                            ...s,
                            snippet_path: nextPath,
                        };
                    }
                    return s;
                })
            );
            if (playingSnippetId === speaker.id) {
                stopAudio();
                handlePlaySnippet({ ...speaker, snippet_path: nextPath });
            }
            setActionError("");
            onSpeakerUpdated?.();
        } catch (err) {
            console.error("Failed to cycle vault speaker snippet:", err);
            setActionError(String(err));
        }
    };

    if (!isOpen) return null;

    const content = (
        <div className="speaker-vault-overlay" onClick={onClose}>
            <div className="speaker-vault-modal" onClick={(e) => e.stopPropagation()}>
                <div className="vault-header">
                    <div className="vault-title-wrap">
                        <IconUsers size={20} style={{ color: "#3b82f6" }} />
                        <div>
                            <h2>Speaker Voiceprint Vault</h2>
                            <p className="vault-subtitle">
                                Recognized meeting participants and custom voice profiles
                            </p>
                        </div>
                    </div>
                    <button type="button" id="speaker-vault-close-modal-btn" data-testid="speaker-vault-close-modal-btn" className="vault-close-btn" onClick={onClose} aria-label="Close modal">
                        <IconX size={18} />
                    </button>
                </div>

                <div className="vault-body">
                    {speakerModelInstalled === false ? (
                        <div id="speaker-vault-empty" data-testid="speaker-vault-empty" className="vault-empty vault-locked" role="status" aria-label="Speaker recognition model required">
                            <IconUsers size={32} style={{ opacity: 0.3 }} />
                            <p>Meeting speakers download required</p>
                            <span>
                                The Speaker Vault recognises people by their voice across meetings. It needs the
                                Meeting speakers download (227 MB), which runs on this computer. Until it is
                                downloaded, speaker names apply to a single meeting only.
                            </span>
                            {onOpenModelSettings && (
                                <button
                                    id="speaker-vault-download-btn"
                                    data-testid="speaker-vault-download-btn"
                                    type="button"
                                    className="vault-download-btn"
                                    onClick={() => {
                                        onClose();
                                        onOpenModelSettings();
                                    }}
                                >
                                    Open Settings → Models
                                </button>
                            )}
                        </div>
                    ) : loading && speakers.length === 0 ? (
                        <div className="vault-empty">Loading voiceprints...</div>
                    ) : speakers.length === 0 ? (
                        <div className="vault-empty">
                            <IconUsers size={32} style={{ opacity: 0.3 }} />
                            <p>No people saved yet.</p>
                            <span>Name a participant in a meeting to add them here. After that they are recognised by voice in future meetings.</span>
                        </div>
                    ) : (
                        speakers.map((s) => {
                            const isEditing = editingSpeakerId === s.id;
                            const isPlaying = playingSnippetId === s.id;
                            const initials = (s.name || "S").slice(0, 2).toUpperCase();

                            const panel = voiceBank?.personId === s.id ? voiceBank.mode : null;
                            return (
                                <React.Fragment key={s.id}>
                                <div className="vault-item">
                                    <div className="vault-item-left">
                                        <div className="vault-avatar">{initials}</div>
                                        <div className="vault-speaker-info">
                                            <div className="vault-speaker-name-row">
                                                {isEditing ? (
                                                    <input
                                                        aria-label="Speaker name"
                                                        id={`speaker-vault-rename-input-${s.id}`}
                                                        data-testid={`speaker-vault-rename-input-${s.id}`}
                                                        type="text"
                                                        className="vault-rename-input"
                                                        value={newName}
                                                        onChange={(e) => setNewName(e.target.value)}
                                                        onKeyDown={(e) => {
                                                            if (e.key === "Enter") handleSaveRename(s);
                                                            if (e.key === "Escape") setEditingSpeakerId(null);
                                                        }}
                                                        autoFocus
                                                    />
                                                ) : (
                                                    <span className="vault-speaker-name">{s.name}</span>
                                                )}
                                            </div>
                                            <div className="vault-speaker-meta">
                                                <span>{s.meeting_count} {s.meeting_count === 1 ? "call" : "calls"}</span>
                                                <span>•</span>
                                                <span>Last seen {new Date(s.last_seen).toLocaleDateString()}</span>
                                                <span>•</span>
                                                <button
                                                    type="button"
                                                    className="vault-samples-link"
                                                    id={`speaker-vault-samples-${s.id}`}
                                                    data-testid={`speaker-vault-samples-${s.id}`}
                                                    onClick={() => setVoiceBank(panel === "samples" ? null : { personId: s.id, mode: "samples" })}
                                                    aria-expanded={panel === "samples"}
                                                >
                                                    {`${s.sample_count} voice ${s.sample_count === 1 ? "sample" : "samples"}`}
                                                </button>
                                            </div>
                                        </div>
                                    </div>

                                    <div className="vault-item-actions">
                                        {s.clip_missing && !s.snippet_path && (
                                            <span className="vault-clip-missing" title="The voice clip file is gone; this person is still recognised by voice">Clip missing</span>
                                        )}
                                        {s.snippet_path && (
                                            <button
                                                type="button"
                                                id={`speaker-vault-play-${s.id}`}
                                                data-testid={`speaker-vault-play-${s.id}`}
                                                className={`vault-action-btn ${isPlaying ? "active" : ""}`}
                                                onClick={() => handlePlaySnippet(s)}
                                                title="Play 3-second voice snippet"
                                            >
                                                {isPlaying ? <IconStop size={13} /> : <IconPlay size={13} />}
                                                <span>{isPlaying ? "Playing" : "3s Clip"}</span>
                                            </button>
                                        )}

                                        {s.candidate_snippets && s.candidate_snippets.length > 1 && (
                                            <button
                                                type="button"
                                                id={`speaker-vault-cycle-sample-${s.id}`}
                                                data-testid={`speaker-vault-cycle-sample-${s.id}`}
                                                className="vault-action-btn"
                                                onClick={() => handleCycleVaultSnippet(s)}
                                                title="Preview another existing clip (does not add a voiceprint)"
                                            >
                                                <IconRefresh size={13} />
                                                <span>
                                                    Clip {Math.max(1, (s.candidate_snippets.indexOf(s.snippet_path || "") + 1))}/{s.candidate_snippets.length}
                                                </span>
                                            </button>
                                        )}

                                        <button
                                            type="button"
                                            id={`speaker-vault-find-more-${s.id}`}
                                            data-testid={`speaker-vault-find-more-${s.id}`}
                                            className={`vault-action-btn ${panel === "find" ? "active" : ""}`}
                                            onClick={() => setVoiceBank(panel === "find" ? null : { personId: s.id, mode: "find" })}
                                            title={`Find more voice samples of ${s.name} in past meetings`}
                                            aria-expanded={panel === "find"}
                                        >
                                            <IconSearch size={13} />
                                            <span>Find more</span>
                                        </button>

                                        {isEditing ? (
                                            <button
                                                type="button"
                                                id={`speaker-vault-save-name-${s.id}`}
                                                data-testid={`speaker-vault-save-name-${s.id}`}
                                                className="vault-action-btn active"
                                                onClick={() => handleSaveRename(s)}
                                                title="Save name"
                                            >
                                                <IconCheck size={14} />
                                                <span>Save</span>
                                            </button>
                                        ) : (
                                            <button
                                                type="button"
                                                id={`speaker-vault-rename-${s.id}`}
                                                data-testid={`speaker-vault-rename-${s.id}`}
                                                className="vault-action-btn"
                                                onClick={() => handleStartRename(s)}
                                                title="Rename speaker"
                                            >
                                                <IconEdit size={13} />
                                                <span>Rename</span>
                                            </button>
                                        )}

                                        {confirmDeleteId === s.id ? (
                                            <>
                                                <button
                                                    type="button"
                                                    id={`speaker-vault-delete-confirm-${s.id}`}
                                                    data-testid={`speaker-vault-delete-confirm-${s.id}`}
                                                    className="vault-action-btn danger active"
                                                    onClick={() => handleDeleteSpeaker(s)}
                                                    title={`Forget ${s.name}'s voice`}
                                                >
                                                    <IconTrash size={13} />
                                                    <span>Delete {s.name}?</span>
                                                </button>
                                                <button
                                                    type="button"
                                                    id={`speaker-vault-delete-cancel-${s.id}`}
                                                    data-testid={`speaker-vault-delete-cancel-${s.id}`}
                                                    className="vault-action-btn"
                                                    onClick={() => setConfirmDeleteId(null)}
                                                >
                                                    <span>Cancel</span>
                                                </button>
                                            </>
                                        ) : (
                                            <button
                                                type="button"
                                                id={`speaker-vault-delete-${s.id}`}
                                                data-testid={`speaker-vault-delete-${s.id}`}
                                                className="vault-action-btn danger"
                                                onClick={() => handleDeleteSpeaker(s)}
                                                title="Delete from vault"
                                            >
                                                <IconTrash size={13} />
                                            </button>
                                        )}
                                    </div>
                                </div>
                                {panel && (
                                    <VoiceBankPanel
                                        personId={s.id}
                                        personName={s.name}
                                        mode={panel}
                                        onChanged={() => { loadVault(); onSpeakerUpdated?.(); }}
                                        onClose={() => setVoiceBank(null)}
                                    />
                                )}
                                </React.Fragment>
                            );
                        })
                    )}
                </div>

                {actionError && (
                    <p id="speaker-vault-error" data-testid="speaker-vault-error" className="vault-error" role="alert">
                        {actionError}
                    </p>
                )}

                <div className="vault-footer">
                    <button type="button" id="speaker-vault-done-btn" data-testid="speaker-vault-done-btn" className="vault-done-btn" onClick={onClose}>
                        Done
                    </button>
                </div>
            </div>
        </div>
    );
    return typeof document !== "undefined" && document.body ? createPortal(content, document.body) : content;
};
