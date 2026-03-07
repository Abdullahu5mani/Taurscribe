import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

type CloseBehavior = 'tray' | 'quit';

interface GeneralTabProps {
    closeBehavior: CloseBehavior;
    setCloseBehavior: (val: CloseBehavior) => void;
}

export function GeneralTab({ closeBehavior, setCloseBehavior }: GeneralTabProps) {
    const handleChange = async (val: CloseBehavior) => {
        setCloseBehavior(val);
        // Persist to settings.json so it survives restarts
        const store = await Store.load('settings.json');
        await store.set('close_behavior', val);
        await store.save();
        // Apply immediately to the running process
        await invoke('set_close_behavior', { behavior: val });
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
        </div>
    );
}

