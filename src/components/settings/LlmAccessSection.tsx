import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

/** settings.json key the MCP server (`taurscribe mcp`) checks before answering. */
const MCP_ENABLED_KEY = 'mcp_enabled';

interface McpSetup {
    command: string;
    args: string[];
}

function configJson(setup: McpSetup): string {
    return JSON.stringify({ mcpServers: { taurscribe: { command: setup.command, args: setup.args } } }, null, 2);
}

/** A prompt the user pastes into any LLM app so it adds the server to its own config. */
function setupPrompt(setup: McpSetup): string {
    const quoted = `"${setup.command}"`;
    return `Please connect yourself to my Taurscribe transcripts. Taurscribe is a speech-to-text app on this computer with a built-in, read-only MCP server (stdio transport) for my dictations, file transcripts and meetings.

Add this MCP server to the configuration of the app you are running in:
- name: taurscribe
- command: ${setup.command}
- args: ${JSON.stringify(setup.args)}

Depending on where you are:
- Claude Code: run  claude mcp add --scope user taurscribe -- ${quoted} ${setup.args.join(' ')}
- Claude Desktop: add it under "mcpServers" in claude_desktop_config.json (Settings → Developer → Edit Config)
- Cursor: add it under "mcpServers" in ~/.cursor/mcp.json
- Any other MCP-capable app: add the same command and args in its MCP server settings

As JSON:
${configJson(setup)}

Keep any MCP servers that are already configured. If you can't edit the configuration yourself, tell me exactly which file to open and what to paste. Then tell me whether the app needs a restart, and once connected, check it works by calling the taurscribe list_meetings tool.`;
}

export function LlmAccessSection() {
    const [enabled, setEnabled] = useState(false);
    const [setup, setSetup] = useState<McpSetup | null>(null);
    const [copied, setCopied] = useState<'prompt' | 'json' | null>(null);

    useEffect(() => {
        (async () => {
            try {
                const store = await Store.load('settings.json');
                setEnabled((await store.get<boolean>(MCP_ENABLED_KEY)) === true);
            } catch { /* defaults to off */ }
            setSetup(await invoke<McpSetup>('get_mcp_setup').catch(() => null));
        })();
    }, []);

    const toggle = async (on: boolean) => {
        setEnabled(on);
        try {
            const store = await Store.load('settings.json');
            await store.set(MCP_ENABLED_KEY, on);
            await store.save();
        } catch (e) {
            console.error('Failed to save LLM access setting:', e);
        }
    };

    const copy = (what: 'prompt' | 'json') => {
        if (!setup) return;
        navigator.clipboard.writeText(what === 'prompt' ? setupPrompt(setup) : configJson(setup)).catch(() => {});
        setCopied(what);
        setTimeout(() => setCopied(null), 1800);
    };

    return (
        <>
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>LLM Access</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: enabled ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Let LLM apps read my transcripts</span>
                        <span className="setting-card-meta">MCP · read-only</span>
                    </div>
                    <label className="switch" htmlFor="mcp-enabled-toggle">
                        <input
                            id="mcp-enabled-toggle"
                            data-testid="mcp-enabled-toggle"
                            role="switch"
                            aria-checked={enabled}
                            aria-label="Let LLM apps read my transcripts"
                            type="checkbox"
                            checked={enabled}
                            onChange={e => toggle(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Claude, ChatGPT, Cursor, LM Studio and other apps that support MCP can search and read your
                    dictations, file transcripts, meetings and Speaker Vault names. They can't change or delete
                    anything. Whatever an app reads may be sent to its model provider, the same as anything you
                    paste into it.
                </p>

                {enabled && setup && (
                    <>
                        <p className="setting-card-desc" style={{ marginTop: '12px' }}>
                            Copy this prompt into the LLM app you want to connect; it will add Taurscribe to its own settings.
                        </p>
                        <div style={{ display: 'flex', gap: '8px', flexWrap: 'wrap', marginTop: '8px' }}>
                            <button
                                type="button"
                                id="mcp-copy-prompt-btn"
                                data-testid="mcp-copy-prompt-btn"
                                className="about-open-btn"
                                onClick={() => copy('prompt')}
                            >{copied === 'prompt' ? 'Copied ✓' : 'Copy setup prompt'}</button>
                            <button
                                type="button"
                                id="mcp-copy-json-btn"
                                data-testid="mcp-copy-json-btn"
                                className="about-open-btn"
                                onClick={() => copy('json')}
                            >{copied === 'json' ? 'Copied ✓' : 'Copy config JSON'}</button>
                        </div>
                        <div className="info-row" style={{ marginTop: '12px' }}>
                            <span className="info-row-label">Command</span>
                            <code className="info-row-value" style={{ wordBreak: 'break-all', fontSize: '11px' }}>
                                {setup.command} {setup.args.join(' ')}
                            </code>
                        </div>
                    </>
                )}
            </div>
        </>
    );
}
