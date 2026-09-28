import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface CliInstallStatus {
    installed: boolean;
    location: string;
    note: string | null;
}

const EXAMPLES = [
    ['taurscribe start', 'start dictating'],
    ['taurscribe stop --print', 'stop, paste, and print the text'],
    ['taurscribe transcribe call.m4a', 'transcribe a file'],
    ['taurscribe search budget', 'search your transcripts'],
];

export function CliSection() {
    const [status, setStatus] = useState<CliInstallStatus | null>(null);
    const [busy, setBusy] = useState(false);
    const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

    const refresh = () => invoke<CliInstallStatus>('cli_install_status').then(setStatus).catch(() => setStatus(null));
    useEffect(() => { void refresh(); }, []);

    const run = async (command: 'install_cli' | 'uninstall_cli') => {
        setBusy(true);
        setMessage(null);
        try {
            setMessage({ text: await invoke<string>(command), error: false });
        } catch (e) {
            setMessage({ text: String(e), error: true });
        } finally {
            setBusy(false);
            void refresh();
        }
    };

    if (!status) return null;

    return (
        <>
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Command Line</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: status.installed ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>taurscribe command</span>
                        <span className="setting-card-meta">{status.installed ? 'installed' : 'not installed'}</span>
                    </div>
                    <button
                        type="button"
                        id="cli-install-btn"
                        data-testid="cli-install-btn"
                        className="about-open-btn"
                        disabled={busy}
                        onClick={() => run(status.installed ? 'uninstall_cli' : 'install_cli')}
                    >{busy ? 'Working…' : status.installed ? 'Uninstall' : 'Install'}</button>
                </div>
                <p className="setting-card-desc">
                    Dictate, transcribe files and search your history from a terminal or a script.
                    Dictation and file transcription use this app, and start it if it isn't running.
                </p>
                <div className="info-row" style={{ marginTop: '12px' }}>
                    <span className="info-row-label">Location</span>
                    <code className="info-row-value" style={{ wordBreak: 'break-all', fontSize: '11px' }}>{status.location}</code>
                </div>
                {status.installed && (
                    <div style={{ marginTop: '12px', display: 'grid', gap: '4px' }}>
                        {EXAMPLES.map(([cmd, what]) => (
                            <div key={cmd} className="info-row">
                                <code className="info-row-label" style={{ fontSize: '11px' }}>{cmd}</code>
                                <span className="info-row-value">{what}</span>
                            </div>
                        ))}
                    </div>
                )}
                {status.note && <p className="setting-card-desc" style={{ marginTop: '8px' }}>{status.note}</p>}
                {message && (
                    <p
                        className="setting-card-desc"
                        role={message.error ? 'alert' : 'status'}
                        style={{ marginTop: '8px', color: message.error ? 'var(--error)' : undefined }}
                    >{message.text}</p>
                )}
            </div>
        </>
    );
}
