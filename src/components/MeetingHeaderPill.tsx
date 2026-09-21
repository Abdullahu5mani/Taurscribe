import { useState, useRef, useEffect } from "react";
import { IconVideo, IconRecord } from "./Icons";
import "./MeetingHeaderPill.css";

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

interface MeetingHeaderPillProps {
  meeting: MeetingInfo | null;
  isRecording: boolean;
  isDualChannelRecording: boolean;
  onStartDualRecording: () => void;
}

export function MeetingHeaderPill({
  meeting,
  isRecording,
  isDualChannelRecording,
  onStartDualRecording,
}: MeetingHeaderPillProps) {
  const [showTooltip, setShowTooltip] = useState(false);
  const hideTimerRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, []);

  if (!meeting) return null;

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

  // Short app name for badge
  const shortAppName = meeting.app_name
    .replace(/^Google\s+/i, "")
    .replace(/\.app$/i, "")
    .replace(/\.exe$/i, "");

  const isRecordingCall = isRecording && (isDualChannelRecording || Boolean(meeting));

  return (
    <div
      className="meeting-header-pill-wrapper"
      onMouseEnter={() => {
        if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        setShowTooltip(true);
      }}
      onMouseLeave={() => {
        hideTimerRef.current = setTimeout(() => setShowTooltip(false), 200);
      }}
    >
      <button
        type="button"
        id="meeting-header-pill-btn"
        data-testid="meeting-header-pill-btn"
        className={`meeting-header-pill ${
          isRecordingCall
            ? "meeting-header-pill--recording"
            : "meeting-header-pill--detected"
        }`}
        onMouseEnter={() => {
          if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
          setShowTooltip(true);
        }}
        onMouseLeave={() => {
          hideTimerRef.current = setTimeout(() => setShowTooltip(false), 200);
        }}
        onClick={() => {
          if (!isRecording) {
            onStartDualRecording();
          }
        }}
        aria-label={
          isRecordingCall
            ? `Recording ${platformDisplayName} call. Process ${meeting.app_name}, PID ${meeting.pid}`
            : `Active call detected: ${platformDisplayName}. Click to start dual-channel recording.`
        }
        title={isRecordingCall ? undefined : "Click to record call (Dual-Channel)"}
      >
        <span
          className={`meeting-header-pill__dot ${
            isRecordingCall
              ? "meeting-header-pill__dot--recording"
              : "meeting-header-pill__dot--active"
          }`}
        />

        {isRecordingCall ? (
          <>
            <IconRecord size={12} className="meeting-header-pill__icon meeting-header-pill__icon--rec" />
            <span className="meeting-header-pill__name">REC: {platformDisplayName}</span>
            <span className="meeting-header-pill__proc">Ch1: Mic • Ch2: Call</span>
          </>
        ) : (
          <>
            <IconVideo size={12} className="meeting-header-pill__icon" />
            <span className="meeting-header-pill__name">{platformDisplayName}</span>
            <span className="meeting-header-pill__proc">
              {shortAppName} • {meeting.pid}
            </span>
            <span className="meeting-header-pill__action-hint">Record Call</span>
          </>
        )}
      </button>

      {showTooltip && (
        <div
          id="meeting-header-pill-tooltip"
          className="meeting-header-pill__tooltip"
          role="tooltip"
          data-testid="meeting-header-pill-tooltip"
        >
          <div className="meeting-tooltip__header">
            <span
              className={`meeting-tooltip__status-dot ${
                isRecordingCall
                  ? "meeting-tooltip__status-dot--rec"
                  : "meeting-tooltip__status-dot--active"
              }`}
            />
            <span className="meeting-tooltip__title">
              {isRecordingCall
                ? "Dual-Channel Recording Active"
                : "Active Meeting Detected"}
            </span>
          </div>

          <div className="meeting-tooltip__content">
            <div className="meeting-tooltip__row">
              <span className="meeting-tooltip__label">Platform:</span>
              <span className="meeting-tooltip__value meeting-tooltip__platform">
                {platformDisplayName}
              </span>
            </div>
            <div className="meeting-tooltip__row">
              <span className="meeting-tooltip__label">Process:</span>
              <span className="meeting-tooltip__value">
                {meeting.app_name} <code className="meeting-tooltip__code">PID {meeting.pid}</code>
              </span>
            </div>
            {displayTitle && (
              <div className="meeting-tooltip__row">
                <span className="meeting-tooltip__label">Title:</span>
                <span className="meeting-tooltip__value meeting-tooltip__truncate">
                  {displayTitle}
                </span>
              </div>
            )}
            <div className="meeting-tooltip__row">
              <span className="meeting-tooltip__label">Audio I/O:</span>
              <span className="meeting-tooltip__value">
                Mic: {meeting.is_using_mic ? "Active" : "Idle"} • Speakers:{" "}
                {meeting.is_playing_audio ? "Active" : "Idle"} ({meeting.confidence}% conf)
              </span>
            </div>
            <div className="meeting-tooltip__row">
              <span className="meeting-tooltip__label">Audio Tap:</span>
              <span className="meeting-tooltip__value meeting-tooltip__routing">
                Ch 1 (Left): Your Voice • Ch 2 (Right): Meeting Audio
              </span>
            </div>
          </div>

          <div className="meeting-tooltip__footer">
            {isRecordingCall ? (
              <span className="meeting-tooltip__rec-tip">
                Recording both speakers into isolated stereo tracks. Click central STOP when finished.
              </span>
            ) : (
              <span className="meeting-tooltip__click-tip">
                ⚡ Click this pill to start isolated Dual-Channel Recording
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
