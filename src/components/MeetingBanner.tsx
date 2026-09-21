import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import { MEETING_KEYS, DEFAULT_AUTORECORD_DELAY } from "./settings/types";
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
    meeting: MeetingInfo | null;
    isRecording: boolean;
    onStartDualRecording: () => void;
    suppressBanner?: boolean;
}

export function MeetingBanner({ meeting, isRecording, onStartDualRecording, suppressBanner }: MeetingBannerProps) {
    const [dismissedMeetingKey, setDismissedMeetingKey] = useState<string | null>(null);
    const [autoRecordCountdown, setAutoRecordCountdown] = useState<number | null>(null);
    const [bannerEnabled, setBannerEnabled] = useState(true);
    const countdownTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

    const isRecordingRef = useRef(isRecording);
    isRecordingRef.current = isRecording;
    const onStartDualRecordingRef = useRef(onStartDualRecording);
    onStartDualRecordingRef.current = onStartDualRecording;

    const cancelAutoRecord = () => {
        if (countdownTimerRef.current) {
            clearInterval(countdownTimerRef.current);
            countdownTimerRef.current = null;
        }
        setAutoRecordCountdown(null);
    };

    const meetingKey = meeting ? `${meeting.pid}:${meeting.title}` : "";
    const isDismissed = Boolean(meetingKey && dismissedMeetingKey === meetingKey);

    useEffect(() => {
        if (!meeting) {
            setDismissedMeetingKey(null);
            return;
        }
        // Settings → Meetings → "Show meeting banner"; re-read per meeting.
        Store.load("settings.json")
            .then((s) => s.get<boolean>(MEETING_KEYS.showBanner))
            .then((v) => setBannerEnabled(v !== false))
            .catch(() => {});
    }, [meetingKey]);

    useEffect(() => {
        if (!meeting || isDismissed) {
            cancelAutoRecord();
            return;
        }

        Promise.all([
            invoke<boolean>("get_auto_record_meetings"),
            Store.load("settings.json").then((s) => s.get<number>(MEETING_KEYS.autoRecordDelay)).catch(() => undefined),
        ])
            .then(([autoRecord, savedDelay]) => {
                if (autoRecord && !isRecordingRef.current) {
                    cancelAutoRecord();
                    let timeLeft = savedDelay ?? DEFAULT_AUTORECORD_DELAY;
                    if (timeLeft <= 0) {
                        onStartDualRecordingRef.current();
                        return;
                    }
                    setAutoRecordCountdown(timeLeft);
                    countdownTimerRef.current = setInterval(() => {
                        timeLeft -= 1;
                        if (timeLeft <= 0) {
                            cancelAutoRecord();
                            if (!isRecordingRef.current) {
                                onStartDualRecordingRef.current();
                            }
                        } else {
                            setAutoRecordCountdown(timeLeft);
                        }
                    }, 1000);
                }
            })
            .catch(() => {});

        return () => {
            cancelAutoRecord();
        };
    }, [meetingKey, isDismissed]);

    // The meetings view shows its own header pill; the auto-record countdown
    // above keeps running either way.
    // Auto-record still counts down when the banner is hidden.
    if (!meeting || isRecording || suppressBanner || (!bannerEnabled && autoRecordCountdown === null)) {
        return null;
    }

    const platform = (meeting?.platform || "").toLowerCase();
    const platformDisplayName =
        platform === "zoom"
            ? "Zoom"
            : platform === "teams"
            ? "Microsoft Teams"
            : platform === "meet"
            ? "Google Meet"
            : platform === "slack"
            ? "Slack Huddle"
            : platform === "discord"
            ? "Discord"
            : platform === "webex"
            ? "Cisco Webex"
            : meeting?.app_name || "Meeting";

    const titleStr = (meeting?.title || "").trim();
    const displayTitle = titleStr || `${platformDisplayName} Call`;

    return (
        <div
            id="meeting-banner"
            data-testid="meeting-banner"
            className="meeting-banner-container"
            style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                background: "linear-gradient(135deg, #1e293b, #0f172a)",
                border: "1px solid #3b82f6",
                borderRadius: "8px",
                padding: "10px 16px",
                margin: "8px 0 12px 0",
                color: "#f8fafc"
            }}
            role="alert"
        >
            <div className="meeting-banner-left">
                <div className="meeting-banner-icon-badge">
                    <span style={{ fontSize: 16 }}>📹</span>
                    <span className="meeting-banner-pulse" />
                </div>
                <div className="meeting-banner-text">
                    <div className="meeting-banner-title">
                        <span className="meeting-platform-tag">{platformDisplayName}</span>
                        <span className="meeting-title-label">{displayTitle}</span>
                    </div>
                    <div className="meeting-banner-sub">
                        {autoRecordCountdown !== null
                            ? `Auto-recording in ${autoRecordCountdown}s • press ✕ to cancel`
                            : "Active Call Detected • Ready for Dual-Channel Recording (Mic + Call Audio)"}
                    </div>
                </div>
            </div>

            <div className="meeting-banner-actions">
                <button
                    type="button"
                    data-testid="meeting-banner-record-btn"
                    id="meeting-banner-record-btn"
                    className="meeting-banner-btn-record"
                    onClick={() => {
                        cancelAutoRecord();
                        onStartDualRecording();
                    }}
                    title="Record your voice on Channel 1 and call participants on Channel 2"
                >
                    <span>Record Call</span>
                </button>

                <button
                    type="button"
                    id="meeting-banner-dismiss-btn"
                    data-testid="meeting-banner-dismiss-btn"
                    className="meeting-banner-btn-dismiss"
                    onClick={() => {
                        cancelAutoRecord();
                        setDismissedMeetingKey(meetingKey);
                    }}
                    title="Dismiss notification"
                    aria-label="Dismiss meeting notification"
                >
                    ✕
                </button>
            </div>
        </div>
    );
}
