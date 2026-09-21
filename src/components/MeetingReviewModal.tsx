import React, { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { MeetingDetailRecord, ActionItem } from "../types/meetings";
import { IconX, IconCheck, IconPlay, IconStop, IconSparkles, IconUsers, IconListCheck, IconTag } from "./Icons";
import { useSpeakerModelInstalled } from "../hooks/useSpeakerModelInstalled";
import "./MeetingReviewModal.css";

interface MeetingReviewModalProps {
    meetingId: number | null;
    onClose: () => void;
    onViewInCatalog?: (id: number) => void;
}

const CATEGORIES = ["Engineering", "Standup", "Planning", "1-on-1", "Client Sync", "General"];

export const MeetingReviewModal: React.FC<MeetingReviewModalProps> = ({
    meetingId,
    onClose,
    onViewInCatalog,
}) => {
    const [meeting, setMeeting] = useState<MeetingDetailRecord | null>(null);
    const [title, setTitle] = useState("");
    const [category, setCategory] = useState("General");
    const [actionItems, setActionItems] = useState<ActionItem[]>([]);
    const [speakerNames, setSpeakerNames] = useState<Record<string, string>>({});
    const [saveError, setSaveError] = useState<string | null>(null);
    const [isSaving, setIsSaving] = useState(false);
    const { installed: speakerModelInstalled } = useSpeakerModelInstalled(meetingId);
    const [playingSnippetSpeakerId, setPlayingSnippetSpeakerId] = useState<string | null>(null);

    const audioRef = useRef<HTMLAudioElement | null>(null);

    useEffect(() => {
        setSaveError(null);
        setMeeting(null);
        if (!meetingId) {
            stopAudio();
            return;
        }

        let active = true;
        const fetchDetail = async () => {
            try {
                const detail = await invoke<MeetingDetailRecord>("get_meeting_detail", {
                    meetingId,
                });
                if (!active) return;
                setMeeting(detail);
                setTitle(detail.title);
                setCategory(detail.category || "General");
                setActionItems(detail.action_items || []);

                // Collect distinct speaker names
                const names: Record<string, string> = {};
                for (const turn of detail.turns) {
                    names[turn.speaker_id] = turn.speaker_name;
                }
                setSpeakerNames(names);
            } catch (err) {
                console.error("Failed to load meeting detail for review:", err);
            }
        };

        fetchDetail();

        return () => {
            active = false;
            stopAudio();
        };
    }, [meetingId]);

    const stopAudio = () => {
        if (audioRef.current) {
            audioRef.current.pause();
            audioRef.current = null;
        }
        setPlayingSnippetSpeakerId(null);
    };

    const handlePlaySnippet = (speakerId: string, snippetPath?: string | null) => {
        if (playingSnippetSpeakerId === speakerId) {
            stopAudio();
            return;
        }
        stopAudio();
        if (!snippetPath) return;

        try {
            const assetUrl = convertFileSrc(snippetPath);
            const audio = new Audio(assetUrl);
            audioRef.current = audio;
            setPlayingSnippetSpeakerId(speakerId);

            audio.onended = () => {
                setPlayingSnippetSpeakerId(null);
                audioRef.current = null;
            };

            audio.onerror = () => {
                setPlayingSnippetSpeakerId(null);
                audioRef.current = null;
            };

            audio.play().catch((e) => {
                console.error("Failed to play snippet audio:", e);
                setPlayingSnippetSpeakerId(null);
            });
        } catch (e) {
            console.error("Failed to load snippet audio:", e);
        }
    };

    const toggleActionItem = (index: number) => {
        setActionItems((prev) =>
            prev.map((item, i) =>
                i === index
                    ? { ...item, status: item.status === "done" ? "todo" : "done" }
                    : item
            )
        );
    };

    const handleSpeakerNameChange = (speakerId: string, name: string) => {
        setSpeakerNames((prev) => ({ ...prev, [speakerId]: name }));
    };

    const dismiss = () => {
        if (!isSaving) onClose();
    };

    const handleSaveAndConfirm = async () => {
        if (!meeting || meeting.id !== meetingId || isSaving) return;

        setIsSaving(true);
        setSaveError(null);
        try {
            // Only changed names are sent; the backend saves these and the
            // meeting fields in one transaction.
            const originalNames: Record<string, string> = {};
            for (const turn of meeting.turns) {
                originalNames[turn.speaker_id] = turn.speaker_name;
            }
            const speakerRenames = Object.entries(speakerNames)
                .filter(([speakerId, newName]) => newName.trim() && newName.trim() !== originalNames[speakerId])
                .map(([speakerId, newName]) => ({ speakerId, newName: newName.trim() }));

            await invoke("save_meeting_review", {
                meetingId: meeting.id,
                title: title.trim() || meeting.title,
                category,
                summary: meeting.summary,
                actionItems,
                speakerRenames,
            });

            onClose();
            if (onViewInCatalog) {
                onViewInCatalog(meeting.id);
            }
        } catch (err) {
            console.error("Failed to save reviewed meeting:", err);
            setSaveError(`Could not save the meeting. Your edits are still here. ${String(err)}`);
        } finally {
            setIsSaving(false);
        }
    };

    if (!meetingId || !meeting || meeting.id !== meetingId) return null;

    // Filter distinct remote speakers with snippets
    const remoteSpeakers = Array.from(
        new Map(
            meeting.turns
                .filter((t) => t.channel === 1)
                .map((t) => [t.speaker_id, t])
        ).values()
    );

    return (
        <div className="meeting-review-overlay" onClick={dismiss}>
            <div className="meeting-review-modal" onClick={(e) => e.stopPropagation()}>
                <div className="review-header">
                    <div style={{ flex: 1, minWidth: 0 }}>
                        <div className="review-badge-row">
                            <span className="review-platform-badge">
                                <IconSparkles size={12} />
                                {meeting.platform || "Call"} Complete
                            </span>
                            <span className="review-time-badge">
                                {Math.round(meeting.duration_ms / 1000)}s call duration
                            </span>
                        </div>
                        <input
                            aria-label="Meeting title"
                            id="meeting-review-title-input"
                            data-testid="meeting-review-title-input"
                            type="text"
                            className="review-title-input"
                            value={title}
                            onChange={(e) => setTitle(e.target.value)}
                            placeholder="Meeting Title"
                        />
                    </div>
                    <button type="button" id="meeting-review-close-modal-btn" data-testid="meeting-review-close-modal-btn" className="review-close-btn" onClick={dismiss} aria-label="Close modal" disabled={isSaving}>
                        <IconX size={18} />
                    </button>
                </div>

                <div className="review-body">
                    {/* Category Selection */}
                    <div className="review-section">
                        <span className="review-section-title">
                            <IconTag size={13} />
                            Category
                        </span>
                        <div className="review-category-group">
                            {CATEGORIES.map((cat) => (
                                <button
                                    id={`meeting-review-category-${cat}`}
                                    data-testid={`meeting-review-category-${cat}`}
                                    key={cat}
                                    type="button"
                                    className={`review-category-pill ${category === cat ? "selected" : ""}`}
                                    onClick={() => setCategory(cat)}
                                >
                                    {cat}
                                </button>
                            ))}
                        </div>
                    </div>

                    {/* Remote Speakers & 3s Snippets */}
                    {remoteSpeakers.length > 0 && (
                        <div className="review-section">
                            <span className="review-section-title">
                                <IconUsers size={13} />
                                Who was on this call?
                            </span>
                            {speakerModelInstalled === false && (
                                <p id="meeting-review-speaker-model-note" data-testid="meeting-review-speaker-model-note" className="review-speaker-model-note" role="note">
                                    Names apply to this meeting only. Download Speaker Recognition in
                                    Settings → Models to recognise people by voice in future meetings.
                                </p>
                            )}
                            <div className="review-speakers-list">
                                {remoteSpeakers.map((spk) => {
                                    // `??`, not `||`: a field the user cleared must stay empty (with
                                    // `||` it refilled with the old name, so typing appended to it).
                                    const currentName = speakerNames[spk.speaker_id] ?? spk.speaker_name;
                                    const isPlaying = playingSnippetSpeakerId === spk.speaker_id;

                                    return (
                                        <div key={spk.speaker_id} className="review-speaker-row">
                                            <div className="review-speaker-left">
                                                <div className="review-speaker-avatar">
                                                    {(currentName || "S").slice(0, 2).toUpperCase()}
                                                </div>
                                                <input
                                                    aria-label="Speaker name"
                                                    id={`meeting-review-speaker-name-input-${spk.speaker_id}`}
                                                    data-testid={`meeting-review-speaker-name-input-${spk.speaker_id}`}
                                                    type="text"
                                                    className="review-speaker-name-input"
                                                    value={currentName}
                                                    onChange={(e) =>
                                                        handleSpeakerNameChange(spk.speaker_id, e.target.value)
                                                    }
                                                    placeholder="Speaker name"
                                                />
                                            </div>

                                            {spk.snippet_path && (
                                                <button
                                                    id={`meeting-review-play-snippet-btn-${spk.speaker_id}`}
                                                    data-testid={`meeting-review-play-snippet-btn-${spk.speaker_id}`}
                                                    type="button"
                                                    className={`review-play-snippet-btn ${isPlaying ? "playing" : ""}`}
                                                    onClick={() => handlePlaySnippet(spk.speaker_id, spk.snippet_path)}
                                                    title="Listen to 3-second sample to verify who spoke"
                                                >
                                                    {isPlaying ? <IconStop size={12} /> : <IconPlay size={12} />}
                                                    <span>{isPlaying ? "Playing..." : "▶ 3s Clip"}</span>
                                                </button>
                                            )}
                                        </div>
                                    );
                                })}
                            </div>
                        </div>
                    )}

                    {/* Action Items Checklist */}
                    {actionItems.length > 0 && (
                        <div className="review-section">
                            <span className="review-section-title">
                                <IconListCheck size={13} />
                                Action Items Detected ({actionItems.filter((a) => a.status === "done").length}/{actionItems.length})
                            </span>
                            <div className="review-actions-list">
                                {actionItems.map((item, idx) => {
                                    const isDone = item.status === "done";
                                    return (
                                        <div key={item.id || idx} className="review-action-item">
                                            <input
                                                aria-label="Done"
                                                id={`meeting-review-action-done-${item.id || idx}`}
                                                data-testid={`meeting-review-action-done-${item.id || idx}`}
                                                type="checkbox"
                                                className="review-action-check"
                                                checked={isDone}
                                                onChange={() => toggleActionItem(idx)}
                                            />
                                            <span className={`review-action-text ${isDone ? "done" : ""}`}>
                                                {item.task}
                                            </span>
                                            {item.assignee && (
                                                <span className="review-action-assignee">
                                                    @{item.assignee}
                                                </span>
                                            )}
                                        </div>
                                    );
                                })}
                            </div>
                        </div>
                    )}
                </div>

                <div className="review-footer">
                    {saveError && <div id="meeting-review-save-error" data-testid="meeting-review-save-error" className="review-save-error" role="alert">{saveError}</div>}
                    <div className="review-footer-left">
                        <button id="meeting-review-dismiss-btn" data-testid="meeting-review-dismiss-btn" type="button" className="review-secondary-btn" onClick={dismiss} disabled={isSaving}>
                            Dismiss
                        </button>
                    </div>
                    <button id="meeting-review-save-btn" data-testid="meeting-review-save-btn" type="button" className="review-save-btn" onClick={handleSaveAndConfirm} disabled={isSaving}>
                        <IconCheck size={15} />
                        <span>{isSaving ? "Saving…" : "Save & View Notes"}</span>
                    </button>
                </div>
            </div>
        </div>
    );
};
