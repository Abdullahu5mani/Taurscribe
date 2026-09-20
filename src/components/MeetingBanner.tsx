import { useEffect, useState, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { IconVideo, IconX, IconRecord } from "./Icons";
import "./MeetingBanner.css";

export interface MeetingInfo {
    pid: number;
    app_name: string;
    title: string;
    url: string;
    platform: string;
    confidence: number;
    should_record: boolean;
    is_using_mic: boolean;
    is_playing_audio: boolean;
}

interface MeetingBannerProps {
    isRecording: boolean;
    onStartDualRecording: () => void;
}

export function MeetingBanner({ isRecording, onStartDualRecording }: MeetingBannerProps) {
    const [meeting, setMeeting] = useState<MeetingInfo | null>(null);
    const [dismissedPid, setDismissedPid] = useState<number | null>(null);

    const isRecordingRef = useRef(isRecording);
    isRecordingRef.current = isRecording;
    const onStartDualRecordingRef = useRef(onStartDualRecording);
    onStartDualRecordingRef.current = onStartDualRecording;

    const checkAutoRecord = () => {
        invoke<boolean>("get_auto_record_meetings")
            .then((autoRecord) => {
                if (autoRecord && !isRecordingRef.current) {
                    onStartDualRecordingRef.current();
                }
            })
            .catch(() => {});
    };

    useEffect(() => {
        // 1. Initial scan on mount
        invoke<MeetingInfo[]>("scan_active_meetings")
            .then((meetings) => {
                const live = meetings.find((m) => m.should_record);
                if (live && live.pid !== dismissedPid) {
                    setMeeting(live);
                    checkAutoRecord();
                }
            })
            .catch(() => {});

        // 2. Listen for real-time meeting detection events
        const unlistenDetectedPromise = listen<MeetingInfo>("meeting-detected", (event) => {
            const m = event.payload;
            if (m.pid !== dismissedPid) {
                setMeeting(m);
                checkAutoRecord();
            }
        });

        const unlistenEndedPromise = listen<MeetingInfo>("meeting-ended", (event) => {
            const m = event.payload;
            setMeeting((curr) => (curr?.pid === m.pid ? null : curr));
        });

        return () => {
            unlistenDetectedPromise.then((unlisten) => unlisten());
            unlistenEndedPromise.then((unlisten) => unlisten());
        };
    }, [dismissedPid]);

    if (!meeting || isRecording) {
        return null;
    }

    const platformDisplayName =
        meeting.platform === "zoom"
            ? "Zoom"
            : meeting.platform === "teams"
            ? "Microsoft Teams"
            : meeting.platform === "meet"
            ? "Google Meet"
            : meeting.platform === "slack"
            ? "Slack Huddle"
            : meeting.platform === "discord"
            ? "Discord"
            : meeting.platform === "webex"
            ? "Cisco Webex"
            : meeting.app_name || "Meeting";

    const displayTitle = meeting.title.trim()
        ? meeting.title
        : `${platformDisplayName} Call`;

    return (
        <div className="meeting-banner-container" role="alert">
            <div className="meeting-banner-left">
                <div className="meeting-banner-icon-badge">
                    <IconVideo size={16} />
                    <span className="meeting-banner-pulse" />
                </div>
                <div className="meeting-banner-text">
                    <div className="meeting-banner-title">
                        <span className="meeting-platform-tag">{platformDisplayName}</span>
                        <span className="meeting-title-label">{displayTitle}</span>
                    </div>
                    <div className="meeting-banner-sub">
                        Active Call Detected • Ready for Dual-Channel Recording (Mic + Call Audio)
                    </div>
                </div>
            </div>

            <div className="meeting-banner-actions">
                <button
                    className="meeting-banner-btn-record"
                    onClick={() => {
                        onStartDualRecording();
                    }}
                    title="Record your voice on Channel 1 and call participants on Channel 2"
                >
                    <IconRecord size={14} />
                    <span>Record Call</span>
                </button>

                <button
                    className="meeting-banner-btn-dismiss"
                    onClick={() => {
                        setDismissedPid(meeting.pid);
                        setMeeting(null);
                    }}
                    title="Dismiss notification"
                    aria-label="Dismiss meeting notification"
                >
                    <IconX size={14} />
                </button>
            </div>
        </div>
    );
}
