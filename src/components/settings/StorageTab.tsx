import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

type Area = 'models' | 'recordings';

interface AreaInfo {
    area: Area;
    path: string;
    default_path: string;
    is_custom: boolean;
    is_other_drive: boolean;
    is_removable: boolean;
    available: boolean;
    used_bytes: number;
    free_bytes: number | null;
    drive_name: string | null;
}

interface Speed {
    path: string;
    write_mb_s: number;
    read_mb_s: number;
}

const COPY: Record<Area, { title: string; desc: string }> = {
    models: {
        title: 'Models',
        desc: 'Downloaded speech, speaker and writing models. These are the biggest files and are read every time a model loads.',
    },
    recordings: {
        title: 'Recordings',
        desc: 'Meeting audio, speaker voice clips and paused-call buffers. Transcripts and settings always stay on this computer.',
    },
};

/** A 4.1 GB model (Qwen3-ASR 1.7B) is the largest one people usually load. */
const REFERENCE_MODEL_GB = 4.1;

function formatBytes(bytes: number) {
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
    if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
    return `${Math.round(bytes / 1e3)} KB`;
}

function formatSpeed(mbs: number) {
    return mbs >= 1000 ? `${(mbs / 1024).toFixed(1)} GB/s` : `${Math.round(mbs)} MB/s`;
}

function loadSeconds(readMbs: number) {
    return (REFERENCE_MODEL_GB * 1024) / Math.max(readMbs, 1);
}

function verdict(readMbs: number): { tone: 'good' | 'ok' | 'slow'; text: string } {
    if (readMbs >= 1000) return { tone: 'good', text: 'Fast. Same as a built-in SSD.' };
    if (readMbs >= 350) return { tone: 'good', text: 'Good. Models load a little slower than from a built-in SSD.' };
    if (readMbs >= 120) return { tone: 'ok', text: 'OK. Expect a few extra seconds each time a model loads.' };
    return { tone: 'slow', text: 'Slow. Loading models will take a long time; a USB SSD or the built-in drive is better.' };
}

function SpeedResult({ area, speed, baseline }: { area: Area; speed: Speed; baseline: Speed | null }) {
    const v = verdict(speed.read_mb_s);
    const max = Math.max(speed.read_mb_s, speed.write_mb_s, baseline?.read_mb_s ?? 0, 1);
    const bar = (label: string, value: number, muted = false) => (
        <div className="storage-bar-row">
            <span className="storage-bar-label">{label}</span>
            <span className="storage-bar-track">
                <span className={`storage-bar-fill${muted ? ' storage-bar-fill--muted' : ''}`} style={{ width: `${Math.max(2, (value / max) * 100)}%` }} />
            </span>
            <span className="storage-bar-value">{formatSpeed(value)}</span>
        </div>
    );
    return (
        <div className="storage-speed" data-testid={`storage-speed-result-${area}`}>
            {bar('Read', speed.read_mb_s)}
            {bar('Write', speed.write_mb_s)}
            {baseline && bar('Built-in drive (read)', baseline.read_mb_s, true)}
            <p className={`storage-verdict storage-verdict--${v.tone}`}>{v.text}</p>
            {area === 'models' && (
                <p className="storage-estimate">
                    Loading a {REFERENCE_MODEL_GB} GB model takes about <strong>{loadSeconds(speed.read_mb_s).toFixed(1)} s</strong> from here
                    {baseline ? <> vs {loadSeconds(baseline.read_mb_s).toFixed(1)} s from the built-in drive</> : null}.
                </p>
            )}
        </div>
    );
}

function AreaCard({ info, onChanged }: { info: AreaInfo; onChanged: (next: AreaInfo) => void }) {
    const { area } = info;
    const [speed, setSpeed] = useState<Speed | null>(null);
    const [baseline, setBaseline] = useState<Speed | null>(null);
    const [measuring, setMeasuring] = useState(false);
    const [pending, setPending] = useState<{ path: string; speed: Speed | null } | null>(null);
    const [moving, setMoving] = useState<{ done: number; total: number } | null>(null);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const un = listen<{ area: Area; done_bytes: number; total_bytes: number }>('storage-move-progress', (e) => {
            if (e.payload.area === area) setMoving({ done: e.payload.done_bytes, total: e.payload.total_bytes });
        });
        return () => { un.then((f) => f()); };
    }, [area]);

    const measure = async () => {
        setMeasuring(true);
        setError(null);
        try {
            const s = await invoke<Speed>('measure_storage_speed', { area, path: null });
            setSpeed(s);
            // Compare against the built-in drive when the folder lives elsewhere.
            if (info.is_other_drive || info.is_custom) {
                setBaseline(await invoke<Speed>('measure_storage_speed', { area, path: info.default_path }).catch(() => null));
            } else {
                setBaseline(null);
            }
        } catch (e) {
            setError(String(e));
        } finally {
            setMeasuring(false);
        }
    };

    const choose = async () => {
        setError(null);
        const picked = await open({ directory: true, multiple: false, title: `Choose a folder for ${COPY[area].title.toLowerCase()}` }).catch(() => null);
        if (!picked || typeof picked !== 'string') return;
        setPending({ path: picked, speed: null });
        setMeasuring(true);
        try {
            const s = await invoke<Speed>('measure_storage_speed', { area, path: picked });
            setPending({ path: picked, speed: s });
        } catch (e) {
            setPending(null);
            setError(String(e));
        } finally {
            setMeasuring(false);
        }
    };

    const apply = async (path: string | null, moveFiles: boolean) => {
        setError(null);
        setMoving(moveFiles ? { done: 0, total: info.used_bytes } : null);
        try {
            const next = await invoke<AreaInfo>('set_storage_location', { area, path, moveFiles });
            onChanged(next);
            setPending(null);
            setSpeed(null);
            setBaseline(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setMoving(null);
        }
    };

    const driveLabel = !info.available
        ? 'Drive not connected'
        : info.is_other_drive
            ? `${info.is_removable ? 'External drive' : 'Other drive'}${info.drive_name ? ` · ${info.drive_name}` : ''}`
            : 'Built-in drive';

    return (
        <div className="setting-card storage-card" id={`storage-${area}`} data-testid={`storage-card-${area}`}>
            <div className="setting-card-header">
                <span className="setting-card-label">{COPY[area].title}</span>
                <span className={`storage-chip${!info.available ? ' storage-chip--error' : info.is_other_drive ? ' storage-chip--warn' : ''}`}>
                    {driveLabel}
                </span>
            </div>
            <p className="setting-card-desc">{COPY[area].desc}</p>

            <div className="storage-path-row">
                <code className="storage-path" title={info.path}>{`\u200E${info.path}\u200E`}</code>
                <div className="storage-actions">
                    <button id={`storage-change-${area}`} type="button" className="ghost-btn" data-testid={`storage-change-${area}`} onClick={choose} disabled={!!moving || measuring}>Change…</button>
                    <button id={`storage-open-${area}`} type="button" className="ghost-btn" data-testid={`storage-open-${area}`} onClick={() => invoke('open_storage_location', { area }).catch((e) => setError(String(e)))} disabled={!info.available}>Open</button>
                    {info.is_custom && (
                        <button id={`storage-reset-${area}`} type="button" className="ghost-btn" data-testid={`storage-reset-${area}`} onClick={() => setPending({ path: info.default_path, speed: null })} disabled={!!moving}>
                            Use default
                        </button>
                    )}
                </div>
            </div>

            <div className="storage-meta">
                <span>{formatBytes(info.used_bytes)} used</span>
                {info.free_bytes != null && <span>{formatBytes(info.free_bytes)} free on this drive</span>}
            </div>

            {info.is_other_drive && info.available && area === 'models' && (
                <p id={`storage-slow-drive-note-${area}`} data-testid={`storage-slow-drive-note-${area}`} className="storage-warning" role="note">
                    Models load from this drive each time you start dictating after they were unloaded. External drives can make that noticeably slower, and
                    Taurscribe can't transcribe while the drive is unplugged. Use <em>Measure speed</em> to see how long a load takes.
                </p>
            )}
            {!info.available && (
                <p id={`storage-unavailable-${area}`} data-testid={`storage-unavailable-${area}`} className="storage-warning storage-warning--error" role="alert">
                    {info.path} isn't available. Connect the drive, or switch back to the default folder.
                </p>
            )}

            {pending && (
                <div className="storage-confirm" data-testid={`storage-confirm-${area}`}>
                    <p className="storage-confirm-title">
                        {pending.path === info.default_path ? 'Move back to the default folder?' : 'Use this folder?'}
                    </p>
                    <code className="storage-path" title={pending.path}>{`\u200E${pending.path}\u200E`}</code>
                    {measuring && !pending.speed && <p className="storage-estimate storage-estimate--busy"><span className="storage-spinner" /> Testing the folder's speed…</p>}
                    {pending.speed && <SpeedResult area={area} speed={pending.speed} baseline={null} />}
                    <div className="storage-confirm-actions">
                        {info.used_bytes > 0 && (
                            <button id={`storage-move-${area}`} type="button" className="ghost-btn storage-primary" data-testid={`storage-move-${area}`} onClick={() => apply(pending.path, true)} disabled={!!moving}>
                                Move {formatBytes(info.used_bytes)} of files
                            </button>
                        )}
                        <button
                            id={`storage-use-${area}`}
                            data-testid={`storage-use-${area}`}
                            type="button"
                            className={`ghost-btn${info.used_bytes > 0 ? '' : ' storage-primary'}`}
                            onClick={() => apply(pending.path, false)}
                            disabled={!!moving}
                            title="Start using the folder as it is. Files already there are used; nothing is moved."
                        >
                            {info.used_bytes > 0 ? 'Switch without moving' : 'Use this folder'}
                        </button>
                        <button id={`storage-cancel-${area}`} data-testid={`storage-cancel-${area}`} type="button" className="ghost-btn" onClick={() => setPending(null)} disabled={!!moving}>Cancel</button>
                    </div>
                </div>
            )}

            {moving && (
                <div id={`storage-progress-${area}`} data-testid={`storage-progress-${area}`} className="storage-progress" role="progressbar" aria-valuemin={0} aria-valuemax={moving.total} aria-valuenow={moving.done}>
                    <span className="storage-bar-track">
                        <span className="storage-bar-fill" style={{ width: `${moving.total ? (moving.done / moving.total) * 100 : 0}%` }} />
                    </span>
                    <span className="storage-bar-value">{formatBytes(moving.done)} / {formatBytes(moving.total)}</span>
                </div>
            )}

            {!pending && (
                <div className="storage-speed-block">
                    <button id={`storage-measure-${area}`} type="button" className="ghost-btn" data-testid={`storage-measure-${area}`} onClick={measure} disabled={measuring || !info.available || !!moving}>
                        {measuring ? <><span className="storage-spinner" /> Measuring…</> : 'Measure speed'}
                    </button>
                    {!speed && !measuring && <span className="storage-hint">Writes and reads a 256 MB test file, then deletes it.</span>}
                    {speed && <SpeedResult area={area} speed={speed} baseline={baseline} />}
                </div>
            )}

            {error && <p id={`storage-error-${area}`} data-testid={`storage-error-${area}`} className="storage-warning storage-warning--error" role="alert">{error}</p>}
        </div>
    );
}

export function StorageTab() {
    const [areas, setAreas] = useState<AreaInfo[] | null>(null);
    const [loadError, setLoadError] = useState<string | null>(null);

    const load = useCallback(() => {
        invoke<AreaInfo[]>('get_storage_locations').then(setAreas).catch((e) => setLoadError(String(e)));
    }, []);
    useEffect(load, [load]);

    if (loadError) return <p className="storage-warning storage-warning--error">{loadError}</p>;
    if (!areas) return <p className="storage-estimate storage-estimate--busy"><span className="storage-spinner" /> Checking folders…</p>;

    return (
        <div className="storage-tab">
            <h3 className="settings-section-title">Where files are kept</h3>
            {areas.map((info) => (
                <AreaCard
                    key={info.area}
                    info={info}
                    onChanged={(next) => setAreas((prev) => prev?.map((a) => (a.area === next.area ? next : a)) ?? null)}
                />
            ))}
        </div>
    );
}
