import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { emitTo } from '@tauri-apps/api/event';

type CloseBehavior = 'tray' | 'quit';
type OverlayStyle = 'minimal' | 'full';

interface GeneralTabProps {
    closeBehavior: CloseBehavior;
    setCloseBehavior: (val: CloseBehavior) => void;
    overlayStyle: OverlayStyle;
    setOverlayStyle: (val: OverlayStyle) => void;
}

export function GeneralTab({ closeBehavior, setCloseBehavior, overlayStyle, setOverlayStyle }: GeneralTabProps) {
    const handleChange = async (val: CloseBehavior) => {
        setCloseBehavior(val);
        // Persist to settings.json so it survives restarts
        const store = await Store.load('settings.json');
        await store.set('close_behavior', val);
        await store.save();
        // Apply immediately to the running process
        await invoke('set_close_behavior', { behavior: val });
    };

    const handleOverlayStyleChange = async (val: OverlayStyle) => {
        setOverlayStyle(val);
        const store = await Store.load('settings.json');
        await store.set('overlay_style', val);
        await store.save();
        // Notify the overlay window immediately so it switches without a restart
        emitTo('overlay', 'overlay-style-changed', val).catch(() => {});
    };

    return (
        <div className="general-tab">
            <h3 className="settings-section-title">General</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label">Close button behavior</span>
                </div>
                <p className="setting-card-desc">
                    What happens when you click the window's close (×) button.
                </p>
                <div className="close-behavior-options">
                    <label className={`close-behavior-option${closeBehavior === 'tray' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="close_behavior"
                            value="tray"
                            checked={closeBehavior === 'tray'}
                            onChange={() => handleChange('tray')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Minimise to tray</span>
                            <span className="close-behavior-option-desc">
                                The app stays running in the system tray. Click the tray icon to reopen.
                            </span>
                        </div>
                    </label>
                    <label className={`close-behavior-option${closeBehavior === 'quit' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="close_behavior"
                            value="quit"
                            checked={closeBehavior === 'quit'}
                            onChange={() => handleChange('quit')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Quit app</span>
                            <span className="close-behavior-option-desc">
                                The process exits completely. Use the tray menu to reopen.
                            </span>
                        </div>
                    </label>
                </div>
            </div>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label">Recording overlay style</span>
                </div>
                <p className="setting-card-desc">
                    How the overlay HUD looks while recording.
                </p>
                <div className="close-behavior-options">
                    <label className={`close-behavior-option${overlayStyle === 'full' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="overlay_style"
                            value="full"
                            checked={overlayStyle === 'full'}
                            onChange={() => handleOverlayStyleChange('full')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Full HUD</span>
                            <span className="close-behavior-option-desc">
                                Interactive card with live transcript, waveform, and pause/cancel controls.
                            </span>
                        </div>
                    </label>
                    <label className={`close-behavior-option${overlayStyle === 'minimal' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="overlay_style"
                            value="minimal"
                            checked={overlayStyle === 'minimal'}
                            onChange={() => handleOverlayStyleChange('minimal')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Minimal</span>
                            <span className="close-behavior-option-desc">
                                Compact status pill — engine name and phase only. Stays out of the way.
                            </span>
                        </div>
                    </label>
                </div>
            </div>
        </div>
    );
}

