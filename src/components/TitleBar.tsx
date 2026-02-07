import { getCurrentWindow } from "@tauri-apps/api/window";

function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const p = navigator.platform ?? "";
  const ua = navigator.userAgent ?? "";
  return p.includes("Mac") || ua.includes("Mac");
}

export function TitleBar() {
  const appWindow = getCurrentWindow();
  const mac = isMac();

  const handleMinimize = () => appWindow.minimize();
  const handleMaximize = () => appWindow.toggleMaximize();
  const handleClose = () => appWindow.close();

  return (
    <header className={`titlebar titlebar--${mac ? "mac" : "win"}`}>
      {mac ? (
        <>
          <div className="titlebar-controls titlebar-controls--mac">
            <button type="button" className="titlebar-btn titlebar-btn--close" onClick={handleClose} aria-label="Close" />
            <button type="button" className="titlebar-btn titlebar-btn--minimize" onClick={handleMinimize} aria-label="Minimize" />
            <button type="button" className="titlebar-btn titlebar-btn--maximize" onClick={handleMaximize} aria-label="Maximize" />
          </div>
          <div className="titlebar-drag titlebar-drag--mac" data-tauri-drag-region>
            <span className="titlebar-title">Taurscribe</span>
          </div>
        </>
      ) : (
        <>
          <div className="titlebar-drag titlebar-drag--win" data-tauri-drag-region>
            <span className="titlebar-title">Taurscribe</span>
          </div>
          <div className="titlebar-controls titlebar-controls--win">
            <button type="button" className="titlebar-btn titlebar-btn--minimize" onClick={handleMinimize} aria-label="Minimize">
              <svg width="10" height="10" viewBox="0 0 10 10"><line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" strokeWidth="1.2" /></svg>
            </button>
            <button type="button" className="titlebar-btn titlebar-btn--maximize" onClick={handleMaximize} aria-label="Maximize">
              <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1.2" /></svg>
            </button>
            <button type="button" className="titlebar-btn titlebar-btn--close" onClick={handleClose} aria-label="Close">
              <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" /></svg>
            </button>
          </div>
        </>
      )}
    </header>
  );
}
