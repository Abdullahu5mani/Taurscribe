import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MeetingHeaderPill, MeetingInfo } from "./MeetingHeaderPill";
import { Logo } from "./Logo";

function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const p = navigator.platform ?? "";
  const ua = navigator.userAgent ?? "";
  return p.includes("Mac") || ua.includes("Mac");
}

interface TitleBarProps {
  meeting?: MeetingInfo | null;
  isRecording?: boolean;
  isDualChannelRecording?: boolean;
  onStartDualRecording?: () => void;
}

/**
 * Custom title bar. A three-column grid (controls | name | extras) keeps the
 * name centred on the window, not on the space left beside the controls.
 * The whole bar moves the window except its buttons; double-click maximizes.
 */
export function TitleBar({
  meeting = null,
  isRecording = false,
  isDualChannelRecording = false,
  onStartDualRecording = () => {},
}: TitleBarProps) {
  const appWindow = getCurrentWindow();
  const mac = isMac();
  const [logoReplay, setLogoReplay] = useState(0);

  // Tauri's own drag region only reacts to the exact element carrying the
  // attribute, so text and padding inside the bar did not move the window.
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, a, input, [data-no-drag]")) return;
    e.preventDefault();
    if (e.detail === 2) {
      void appWindow.toggleMaximize();
    } else {
      void appWindow.startDragging();
    }
  };

  const macControls = (
    <div
      id="titlebar-controls-mac"
      data-testid="titlebar-controls-mac"
      className="titlebar-controls titlebar-controls--mac"
      role="group"
      aria-label="Window management"
    >
      <button type="button" id="titlebar-close-btn" data-testid="titlebar-close-btn" className="titlebar-btn titlebar-btn--close" onClick={() => appWindow.close()} aria-label="Close" />
      <button type="button" id="titlebar-minimize-btn" data-testid="titlebar-minimize-btn" className="titlebar-btn titlebar-btn--minimize" onClick={() => appWindow.minimize()} aria-label="Minimize" />
      <button type="button" id="titlebar-maximize-btn" data-testid="titlebar-maximize-btn" className="titlebar-btn titlebar-btn--maximize" onClick={() => appWindow.toggleMaximize()} aria-label="Maximize" />
    </div>
  );

  const winControls = (
    <div
      id="titlebar-controls-win"
      data-testid="titlebar-controls-win"
      className="titlebar-controls titlebar-controls--win"
      role="group"
      aria-label="Window management"
    >
      <button type="button" id="titlebar-minimize-btn-win" data-testid="titlebar-minimize-btn-win" className="titlebar-btn titlebar-btn--minimize" onClick={() => appWindow.minimize()} aria-label="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10"><line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" strokeWidth="1.2" /></svg>
      </button>
      <button type="button" id="titlebar-maximize-btn-win" data-testid="titlebar-maximize-btn-win" className="titlebar-btn titlebar-btn--maximize" onClick={() => appWindow.toggleMaximize()} aria-label="Maximize">
        <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1.2" /></svg>
      </button>
      <button type="button" id="titlebar-close-btn-win" data-testid="titlebar-close-btn-win" className="titlebar-btn titlebar-btn--close" onClick={() => appWindow.close()} aria-label="Close">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" /></svg>
      </button>
    </div>
  );

  const extras = meeting ? (
    <MeetingHeaderPill
      meeting={meeting}
      isRecording={isRecording}
      isDualChannelRecording={isDualChannelRecording}
      onStartDualRecording={onStartDualRecording}
    />
  ) : null;

  return (
    <header
      id="titlebar-header"
      data-testid="titlebar-header"
      className={`titlebar titlebar--${mac ? "mac" : "win"}`}
      role="banner"
      onMouseDown={onMouseDown}
    >
      <div className="titlebar-side titlebar-side--start">{mac ? macControls : extras}</div>
      <div className="titlebar-brand">
        <button
          type="button"
          id="titlebar-logo-btn"
          data-testid="titlebar-logo-btn"
          className="titlebar-logo-btn"
          onClick={() => setLogoReplay((n) => n + 1)}
          aria-label="Taurscribe"
          title="Taurscribe"
        >
          <Logo size={16} variant="small" animate={logoReplay > 0} replayKey={logoReplay} />
        </button>
        <span className="titlebar-title">Taurscribe</span>
      </div>
      <div className="titlebar-side titlebar-side--end">{mac ? extras : winControls}</div>
    </header>
  );
}
