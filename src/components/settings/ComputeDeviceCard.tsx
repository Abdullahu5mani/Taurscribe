import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
    deviceChoices,
    deviceLabel,
    fastest,
    loadDeviceReport,
    selectedDeviceId,
    type ComputeDevice,
    type DeviceReport,
    type DeviceSpeed,
    type SpeedReport,
} from '../../utils/computeDevices';

interface Props {
    asrBackend: 'gpu' | 'cpu';
    computeDevice: string;
    onSelect: (backend: 'gpu' | 'cpu', device?: string, label?: string) => Promise<void>;
    /** Recording or a model loading: switching would be refused. */
    blocked: boolean;
    llmBackend?: 'gpu' | 'cpu';
    llmComputeDevice?: string;
    onSelectLlmDevice?: (id: string) => void;
    enableGrammarLM?: boolean;
    transcriptionStyle?: string;
}

interface CommandResult<T> { ok: boolean; data?: T; error?: { code: string; message: string } }

function formatRealtime(x: number) {
    return x >= 10 ? `${Math.round(x)}×` : `${x.toFixed(1)}×`;
}

/**
 * Settings → Models: which device the speech models run on, and a speed test
 * across them. Hidden where there is nothing to choose (Macs, CPU builds).
 */
export function ComputeDeviceCard({ asrBackend, computeDevice, onSelect, blocked, llmBackend = 'gpu', llmComputeDevice = 'same', onSelectLlmDevice, enableGrammarLM = false, transcriptionStyle = 'Clean' }: Props) {
    const [report, setReport] = useState<DeviceReport | null>(null);
    const [speeds, setSpeeds] = useState<SpeedReport | null>(null);
    const [measuring, setMeasuring] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [includeFlowScribe, setIncludeFlowScribe] = useState(enableGrammarLM);
    const [flowAvailable, setFlowAvailable] = useState(false);

    useEffect(() => {
        let alive = true;
        loadDeviceReport().then((r) => alive && setReport(r));
        const refresh = () => { void invoke<boolean>('check_grammar_llm_available').then((available) => alive && setFlowAvailable(available)).catch(() => {}); };
        refresh();
        const off = listen('models-changed', refresh);
        return () => { alive = false; off.then(f => f()).catch(() => {}); };
    }, []);

    useEffect(() => {
        if (measuring === null) return;
        const off = listen<{ index: number; total: number; device: string; round?: number; rounds?: number }>('compute-benchmark-progress', (e) => {
            setMeasuring(`Testing ${e.payload.device} (${e.payload.index + 1} of ${e.payload.total})${e.payload.round ? ` · run ${e.payload.round}/${e.payload.rounds}` : ''}…`);
        });
        return () => { off.then((f) => f()); };
    }, [measuring !== null]); // eslint-disable-line react-hooks/exhaustive-deps

    if (!report || report.build_flavor === 'apple') return null;
    const { gpus, npus } = deviceChoices(report);
    if (gpus.length === 0) return null;

    const cpu: ComputeDevice = report.compute_devices.find((d) => d.kind === 'cpu') ?? { id: 'cpu', name: 'CPU', backend: 'CPU', kind: 'cpu', memory_gb: null, usable: true, note: null };
    const options = [...gpus, cpu];
    const selected = selectedDeviceId(asrBackend, computeDevice, gpus);
    const best = speeds ? fastest(speeds.results) : null;
    const max = Math.max(...(speeds?.results.map((r) => r.realtime) ?? [1]), 1);
    const speedFor = (id: string): DeviceSpeed | undefined => speeds?.results.find((r) => r.device_id === id);

    const choose = (d: ComputeDevice) => {
        if (blocked || d.id === selected) return;
        void onSelect(d.kind === 'cpu' ? 'cpu' : 'gpu', d.kind === 'cpu' ? undefined : d.id, deviceLabel(d));
    };

    const measure = async () => {
        setError(null);
        setMeasuring('Starting…');
        try {
            const res = await invoke<CommandResult<SpeedReport>>('benchmark_compute_devices', {
                includeFlowscribe: includeFlowScribe,
                flowscribeDeviceId: llmComputeDevice,
                flowscribeUseGpu: llmBackend === 'gpu',
                style: transcriptionStyle,
            });
            if (res.ok && res.data) { setSpeeds(res.data); if (res.data.restore_error) setError(`Test finished, but the previous model could not be restored: ${res.data.restore_error}`); }
            else setError(res.error?.message ?? 'The speed test failed');
        } catch (e) {
            setError(String(e));
        } finally {
            setMeasuring(null);
        }
    };

    return (
        <div className="setting-card device-card" id="models-compute-device" data-testid="models-compute-device">
            <div className="setting-card-header">
                <span className="setting-card-label">Speech runs on</span>
            </div>
            <div className="device-list" role="radiogroup" aria-label="Speech model device">
                {options.map((d) => {
                    const speed = speedFor(d.id);
                    const isBest = best?.device_id === d.id && (speeds?.results.length ?? 0) > 1;
                    return (
                        <button
                            key={d.id}
                            type="button"
                            role="radio"
                            aria-checked={d.id === selected}
                            className={`device-row${d.id === selected ? ' device-row--active' : ''}`}
                            data-testid={`device-row-${d.kind === 'cpu' ? 'cpu' : d.id}`}
                            onClick={() => choose(d)}
                            disabled={blocked || measuring !== null}
                        >
                            <span className="device-radio" aria-hidden />
                            <span className="device-name">
                                {d.kind === 'cpu' ? 'CPU' : d.name}
                                <span className="device-meta">
                                    {d.kind === 'cpu' ? 'Works everywhere, slowest' : `${d.backend}${d.kind === 'igpu' ? ' · integrated' : ''}${d.memory_gb ? ` · ${d.memory_gb.toFixed(0)} GB` : ''}`}
                                </span>
                            </span>
                            {speed && (
                                speed.error
                                    ? <span className="device-speed device-speed--error" title={speed.error}>Failed</span>
                                    : <span className="device-speed">
                                        <span className="storage-bar-track"><span className="storage-bar-fill" style={{ width: `${Math.max(3, (speed.realtime / max) * 100)}%` }} /></span>
                                        <span className="storage-bar-value">{formatRealtime(speed.realtime)}</span>
                                        <span className="device-timing">Unloaded {speed.unloaded_total_s.toFixed(2)} s · Loaded {speed.loaded_total_s.toFixed(2)} s</span>
                                        <span className="device-timing">Speech load {speed.load_s.toFixed(2)} s · Transcribe {speed.transcribe_s.toFixed(2)} s</span>
                                        {speeds?.include_flowscribe && <span className="device-timing">Cleanup load {speed.flowscribe_load_s.toFixed(2)} s · Clean up {speed.flowscribe_s.toFixed(2)} s</span>}
                                        {isBest && <span className="device-best">Fastest</span>}
                                    </span>
                            )}
                        </button>
                    );
                })}
                {npus.map((d) => (
                    <div key={d.id} className="device-row device-row--unavailable" title={d.note ?? undefined}>
                        <span className="device-radio" aria-hidden />
                        <span className="device-name">
                            {d.name}
                            <span className="device-meta">NPU · not supported by the speech models yet</span>
                        </span>
                    </div>
                ))}
            </div>
            {onSelectLlmDevice && (
                <label className="device-cleanup-select">
                    <span>Word cleanup runs on</span>
                    <select data-testid="flowscribe-compute-device" value={llmBackend === 'cpu' ? 'cpu' : llmComputeDevice === 'cpu' ? 'same' : llmComputeDevice} onChange={(e) => { onSelectLlmDevice(e.target.value); setSpeeds(null); }} disabled={blocked || measuring !== null}>
                        <option value="same">Follow speech GPU</option>
                        {gpus.map((d) => <option key={d.id} value={d.id}>{deviceLabel(d)}{d.kind === 'igpu' ? ' · integrated' : ''}</option>)}
                        <option value="cpu">CPU</option>
                    </select>
                    <span className="storage-hint">FlowScribe V3 can use the same GPU, another GPU, or the CPU.</span>
                </label>
            )}
            <div className="storage-speed-block">
                <label className="device-benchmark-toggle">
                    <input type="checkbox" data-testid="benchmark-include-flowscribe" checked={includeFlowScribe} onChange={(e) => { setIncludeFlowScribe(e.target.checked); setSpeeds(null); }} disabled={blocked || measuring !== null || (!flowAvailable && !includeFlowScribe)} />
                    Include FlowScribe V3 cleanup
                </label>
                {!flowAvailable && <span className="storage-hint">Download FlowScribe V3 in Models to include cleanup.</span>}
                <button type="button" className="ghost-btn" data-testid="device-measure-speed" onClick={measure} disabled={blocked || measuring !== null}>
                    {measuring !== null ? <><span className="storage-spinner" /> {measuring}</> : 'Measure speed'}
                </button>
                {speeds && measuring === null && (
                    <span className="storage-hint">
                        {speeds.model} · {speeds.clip_seconds} s of speech · median of {speeds.rounds} runs · FlowScribe {speeds.include_flowscribe ? 'included' : 'off'}. Loaded totals include {speeds.include_flowscribe ? 'speech and cleanup' : 'speech recognition'}. Unloaded totals also include model loading. Disk caches are not cleared.
                    </span>
                )}
                {error && <span className="storage-hint device-error">{error}</span>}
            </div>
        </div>
    );
}
