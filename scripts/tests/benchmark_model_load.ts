/**
 * Native app benchmark; run against a disposable app with WebView2 CDP enabled.
 * bun scripts/tests/benchmark_model_load.ts --audio C:\\path\\speech.wav \
 *   --engine granite --model granite-speech-5-nc --device cuda:GPU_NAME \
 *   --rounds 3 --output results.json
 *
 * Calls the real file importer without saving history or recording audio.
 * "Unloaded" means model weights are absent from the app, not a cold OS cache.
 */
const args = process.argv.slice(2);
const options: Record<string, string> = {};
for (let i = 0; i < args.length; i += 2) {
    if (!args[i].startsWith("--") || !args[i + 1]) throw Error("Expected --name value pairs");
    options[args[i].slice(2)] = args[i + 1];
}
const engine = options.engine ?? "granite";
const rounds = Number(options.rounds ?? 3);
if (!["granite", "whisper", "qwen3"].includes(engine)) throw Error("Unknown engine");
if (!Number.isInteger(rounds) || rounds < 1 || rounds > 20) throw Error("rounds must be 1–20");
if (!options.audio || !options.model || !options.output) throw Error("--audio, --model and --output are required");
const config = { engine, model: options.model, audio: options.audio, device: options.device, rounds };
const pages = await (await fetch(`${options.url ?? "http://127.0.0.1:9333"}/json`)).json();
const page = pages.find((p: any) => p.type === "page" && p.url.includes("tauri.localhost") && !p.url.includes("#overlay"));
if (!page) throw Error("No Taurscribe main WebView found");

async function benchmark(config: any) {
    const internals = (window as any).__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    const initial = await invoke("get_engine_selection_state");
    if (initial.engine_loading) throw Error("Wait for startup model loading to finish");
    const gpu = await invoke("get_gpu_report");
    const timeout = await invoke("get_auto_unload_status");
    const device = config.device ?? gpu.preferred_device;
    if (!gpu.compute_devices.some((d: any) => d.usable && d.id === device) && device !== "auto") {
        throw Error(`Device unavailable: ${device}`);
    }
    const useGpu = device !== "cpu";
    const check = (result: any) => {
        if (!result?.ok) throw Error(JSON.stringify(result));
        return result;
    };
    const load = async (family: string, model: string, gpuEnabled: boolean) => check(await invoke(
        family === "whisper" ? "switch_model" : family === "granite" ? "init_granite" : "init_qwen3",
        { modelId: model, useGpu: gpuEnabled },
    ));
    const runs: any[] = [];
    let current: any = null;
    const event = "file-transcription-progress";
    const callback = internals.transformCallback((e: any) => {
        if (current && e.payload.job_id === current.job_id && e.payload.path === config.audio) {
            current.events.push({ ms: performance.now() - current.started, ...e.payload });
        }
    });
    const listener = await invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler: callback });
    let preload: any;
    let failure: string | null = null;
    const restoreErrors: string[] = [];
    try {
        await invoke("set_auto_unload_timeout", { seconds: 0 });
        check(await invoke("unload_current_model"));
        await invoke("set_compute_device", { id: device });
        const started = performance.now();
        await load(config.engine, config.model, useGpu);
        preload = { ms: performance.now() - started, state: await invoke("get_engine_selection_state") };
        for (let round = 1; round <= config.rounds; round++) {
            check(await invoke("unload_current_model"));
            for (const mode of ["unloaded", "loaded"]) {
                const before = await invoke("get_engine_selection_state");
                if (before.engine_loading || before.active_engine !== config.engine) throw Error("Unexpected engine state");
                if (mode === "unloaded" && before.loaded_engine !== null) throw Error("Cold run still has a loaded model");
                if (mode === "loaded" && before.loaded_model_id !== config.model) throw Error("Warm run model is absent/different");
                current = { round, mode, job_id: `load-timing-${Date.now()}-${round}-${mode}`, events: [], started: performance.now() };
                const result = await invoke("transcribe_file", {
                    path: config.audio, expectedEngine: config.engine, expectedModelId: config.model,
                    jobId: current.job_id, deferAutoUnload: true, useGpu,
                });
                const wall = performance.now() - current.started;
                const after = await invoke("get_engine_selection_state");
                if (after.loaded_model_id !== config.model || after.engine_loading) throw Error("Model not resident after run");
                if (device.startsWith("cuda:") && !after.backend.toLowerCase().includes("cuda")) throw Error("Expected CUDA backend");
                if (device.startsWith("vulkan:") && !after.backend.toLowerCase().includes("vulkan")) throw Error("Expected Vulkan backend");
                if (!result.transcript?.trim()) throw Error("Empty transcript; this is not a valid speech timing test");
                const loading = current.events.find((e: any) => e.status === "loading model");
                const decoding = current.events.find((e: any) => e.status === "decoding" && e.percent === 5);
                if (mode === "unloaded" && (!loading || !decoding)) throw Error("Missing model-loading progress events");
                if (mode === "loaded" && loading) throw Error("Loaded run reloaded the model");
                const loadMs = loading ? decoding.ms - loading.ms : 0;
                const record = {
                    round, mode, before, after, wall_ms: wall, load_interval_ms: loadMs,
                    post_load_ms: wall - loadMs, ...result, events: current.events,
                };
                runs.push(record);
                console.log("LOAD_TIMING_PROGRESS " + JSON.stringify({ engine: config.engine, round, mode, wall_ms: wall, load_interval_ms: loadMs }));
                current = null;
            }
        }
    } catch (e) {
        failure = String(e);
    } finally {
        current = null;
        try {
            (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, listener);
            await invoke("plugin:event|unlisten", { event, eventId: listener });
            internals.unregisterCallback(callback);
        } catch (e) { restoreErrors.push(`Listener: ${e}`); }
        try {
            check(await invoke("unload_current_model"));
            await invoke("set_compute_device", { id: gpu.preferred_device });
            if (initial.loaded_engine && initial.loaded_model_id) {
                await load(initial.loaded_engine, initial.loaded_model_id, gpu.preferred_device !== "cpu");
            }
        } catch (e) { restoreErrors.push(`Model/device: ${e}`); }
        try { await invoke("set_auto_unload_timeout", { seconds: timeout.timeout_seconds }); }
        catch (e) { restoreErrors.push(`Auto unload: ${e}`); }
    }
    const sameText = runs.length > 0 && runs.every(r => r.transcript === runs[0].transcript);
    const median = (values: number[]) => {
        const sorted = [...values].sort((a, b) => a - b);
        const i = Math.floor(sorted.length / 2);
        return sorted.length % 2 ? sorted[i] : (sorted[i - 1] + sorted[i]) / 2;
    };
    const cold = runs.filter(r => r.mode === "unloaded");
    const warm = runs.filter(r => r.mode === "loaded");
    const summary = cold.length && warm.length ? {
        unloaded_total_ms: median(cold.map(r => r.wall_ms)),
        loaded_total_ms: median(warm.map(r => r.wall_ms)),
        load_interval_ms: median(cold.map(r => r.load_interval_ms)),
        paired_extra_ms: median(cold.map((r, i) => r.wall_ms - warm[i].wall_ms)),
        transcripts_identical: sameText,
    } : null;
    return {
        created_at: new Date().toISOString(), config: { ...config, device }, gpu, initial, preload,
        methodology: "Same file importer; model explicitly unloaded before each cold run; immediately repeated while resident. OS file cache is not flushed. Loading interval uses app progress events; post-load time includes audio decoding, preprocessing and inference. Preload includes engine kernel warm-up. Does not record audio or save history.",
        runs, summary, failure, restore_errors: restoreErrors,
    };
}

const result: any = await new Promise((resolve, reject) => {
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    const timer = setTimeout(() => { ws.close(); reject(Error("Benchmark exceeded 15 minutes")); }, 900000);
    const finish = (error?: Error, value?: any) => {
        clearTimeout(timer); ws.close(); error ? reject(error) : resolve(value);
    };
    ws.onerror = () => finish(Error("CDP connection failed"));
    ws.onopen = () => {
        ws.send(JSON.stringify({ id: 1, method: "Runtime.enable" }));
        ws.send(JSON.stringify({ id: 2, method: "Runtime.evaluate", params: {
            expression: `(${benchmark.toString()})(${JSON.stringify(config)})`, awaitPromise: true, returnByValue: true,
        } }));
    };
    ws.onmessage = e => {
        const message = JSON.parse(String(e.data));
        if (message.method === "Runtime.consoleAPICalled") {
            const line = message.params.args?.[0]?.value;
            if (typeof line === "string" && line.startsWith("LOAD_TIMING_PROGRESS ")) console.log(line);
        }
        if (message.id === 2) {
            if (message.error || message.result?.exceptionDetails) finish(Error(JSON.stringify(message.error ?? message.result.exceptionDetails)));
            else finish(undefined, message.result.result.value);
        }
    };
});
await Bun.write(options.output, JSON.stringify(result, null, 2) + "\n");
console.log(JSON.stringify(result.summary, null, 2));
if (result.failure || result.restore_errors.length) {
    console.error(result.failure, result.restore_errors);
    process.exitCode = 1;
}
