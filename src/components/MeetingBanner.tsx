import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
import { MEETING_KEYS, DEFAULT_AUTORECORD_DELAY } from "./settings/types";
import { scheduleAutoRecord } from "../utils/autoRecord";
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
    const [autoRecordRevision, setAutoRecordRevision] = useState(0);
    const [autoRecordReady, setAutoRecordReady] = useState(false);
    const cancelAutoRecordRef = useRef<(() => void) | null>(null);
    const consumedMeetingKeyRef = useRef<string | null>(null);

    const isRecordingRef = useRef(isRecording);
    isRecordingRef.current = isRecording;
    const onStartDualRecordingRef = useRef(onStartDualRecording);
    onStartDualRecordingRef.current = onStartDualRecording;

    const cancelAutoRecord = () => {
        cancelAutoRecordRef.current?.();
        cancelAutoRecordRef.current = null;
        setAutoRecordCountdown(null);
    };

    // Identifies the call by its process, not its window title: titles change
    // mid-call (browser tab badges like "(1) Meet – …", Zoom renaming its window),
    // and a title change must not undo a dismissal or restart the auto-record
    // countdown. A new call clears the dismissal when `meeting` goes null.
    const meetingKey = meeting ? `${meeting.pid}:${meeting.platform}` : "";
    const isDismissed = Boolean(meetingKey && dismissedMeetingKey === meetingKey);

    useEffect(() => {
        if (!meeting) {
            setDismissedMeetingKey(null);
            consumedMeetingKeyRef.current = null;
        }
    }, [meetingKey]);

    useEffect(() => {
        let disposed = false;
        const subscription = listen<boolean>("auto-record-meetings-changed", () => {
            if (disposed) return;
            // Cancel synchronously, including an outstanding settings read.
            cancelAutoRecord();
            setAutoRecordRevision((v) => v + 1);
        }).catch(() => () => {});
        // Subscribe before reading the setting so startup hydration cannot
        // enable auto-record between the first read and listener registration.
        void subscription.then(() => { if (!disposed) setAutoRecordReady(true); });
        return () => {
            disposed = true;
            void subscription.then((unlisten) => unlisten()).catch(() => {});
        };
    }, []);

    useEffect(() => {
        let disposed = false;
        let bannerRevision = 0;
        let unlisten: (() => void) | undefined;
        void (async () => {
            const store = await Store.load("settings.json");
            if (disposed) return;
            unlisten = await store.onChange((key, value) => {
                if (disposed) return;
                if (key === MEETING_KEYS.showBanner) {
                    bannerRevision++;
                    setBannerEnabled(value !== false);
                } else if (key === MEETING_KEYS.autoRecordDelay) {
                    cancelAutoRecord();
                    setAutoRecordRevision((v) => v + 1);
                }
            });
            if (disposed) { unlisten(); return; }
            const revision = bannerRevision;
            const value = await store.get<boolean>(MEETING_KEYS.showBanner);
            if (!disposed && revision === bannerRevision) setBannerEnabled(value !== false);
        })().catch(() => {});
        return () => { disposed = true; unlisten?.(); };
    }, []);

    useEffect(() => {
        // A call in an app the detector doesn't know (should_record false) is
        // only offered: a DAW or a voice tool can look like one.
        if (!autoRecordReady || !meeting || isDismissed || !meeting.should_record || consumedMeetingKeyRef.current === meetingKey) {
            cancelAutoRecord();
            return;
        }

        // Cancelling also covers the settings read still in flight, so a
        // dismissal or a call that ends meanwhile can't start a recording.
        const cancel = scheduleAutoRecord({
            loadSettings: () =>
                Promise.all([
                    invoke<boolean>("get_auto_record_meetings"),
                    Store.load("settings.json").then((s) => s.get<number>(MEETING_KEYS.autoRecordDelay)).catch(() => undefined),
                ]).then(([autoRecord, delay]) => [autoRecord, delay ?? undefined] as [boolean, number | undefined]),
            defaultDelay: DEFAULT_AUTORECORD_DELAY,
            isRecording: () => isRecordingRef.current,
            start: () => {
                consumedMeetingKeyRef.current = meetingKey;
                onStartDualRecordingRef.current();
            },
            onCountdown: setAutoRecordCountdown,
        });
        cancelAutoRecordRef.current = cancel;

        return () => {
            cancel();
            if (cancelAutoRecordRef.current === cancel) cancelAutoRecordRef.current = null;
        };
        // should_record: the same process can go from an offered call to a known one.
    }, [meetingKey, isDismissed, meeting?.should_record, autoRecordRevision, autoRecordReady]);

    // A manual/hotkey recording consumes this countdown, even if it stops
    // before the original timer would have expired. Stopping doesn't re-arm it.
    useEffect(() => {
        if (isRecording) {
            consumedMeetingKeyRef.current = meetingKey;
            cancelAutoRecord();
        }
    }, [isRecording, meetingKey]);

    // The meetings view shows its own header pill; the auto-record countdown
    // above keeps running either way.
    // Auto-record still counts down when the banner is hidden.
    if (!meeting || isDismissed || isRecording || suppressBanner || (!bannerEnabled && autoRecordCountdown === null)) {
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
                        consumedMeetingKeyRef.current = meetingKey;
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
