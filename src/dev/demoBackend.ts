/**
 * Dev-only stand-in for the Tauri backend, used by #preview/app to render the
 * whole app in a normal browser with example data (README screenshots and
 * GIFs). Nothing here ships: it is only imported from src/dev/Preview.tsx.
 *
 * The data is made up. The backend keeps a little state so the demo behaves:
 * starting a recording streams audio levels, stopping it returns a transcript
 * and the feed picks it up from history.
 */

type Callback = (msg: { event: string; id: number; payload: unknown }) => void;

const HOUR = 3600_000;
const now = Date.now();
const at = (msAgo: number) => new Date(now - msAgo).toISOString();

let nextId = 100;
const history = [
    { text: "Can you move the design review to Thursday at 3? Friday is packed with the launch prep.", engine: "granite", model: "granite-speech-5-nc", ago: 0.2 * HOUR, dur: 5200, proc: 410, llm: true },
    { text: "Remind me to send Priya the updated pricing sheet before the call tomorrow.", engine: "granite", model: "granite-speech-5-nc", ago: 1.1 * HOUR, dur: 3900, proc: 380, llm: true },
    { text: "The onboarding flow should skip the hardware step when the machine has already been checked. Let's cache that result and only rerun it if the user changes GPUs.", engine: "qwen3", model: "qwen3-asr-0.6b", ago: 2.4 * HOUR, dur: 9800, proc: 920, llm: true },
    { text: "Hey team, quick update: the Windows build is green again. I'll cut a release candidate after lunch.", engine: "granite", model: "granite-speech-5-nc", ago: 5 * HOUR, dur: 6100, proc: 450, llm: true },
    { text: "git checkout -b fix/overlay-position", engine: "whisper", model: "whisper-base-en-q5_1", ago: 26 * HOUR, dur: 2600, proc: 310, llm: false },
    { text: "Grocery list: oat milk, two avocados, coffee beans, and something for dinner on Saturday.", engine: "granite", model: "granite-speech-5-nc", ago: 30 * HOUR, dur: 5600, proc: 400, llm: true },
].map((h, i) => ({
    id: 90 - i,
    created_at: at(h.ago),
    transcript: h.text,
    engine: h.engine,
    duration_ms: h.dur,
    grammar_llm_used: h.llm,
    processing_time_ms: h.proc,
    model_id: h.model,
    audio_source: "microphone",
    kind: "dictation",
}));

const turn = (speaker_id: string, speaker_name: string, start: number, end: number, channel: number, text: string) =>
    ({ speaker_id, speaker_name, start_ms: start * 1000, end_ms: end * 1000, channel, text, snippet_path: null, candidate_snippets: [], current_snippet_idx: 0 });

const meetings = [
    {
        id: 12, session_id: "demo-12", title: "Weekly product sync", platform: "meet", app_name: "Google Chrome",
        url: "https://meet.google.com/abc-defg-hij", created_at: at(3 * HOUR), duration_ms: 31 * 60_000, category: "work",
        summary: [
            "Launch moves to October 14 so the Windows installer can ship with it.",
            "Meeting detection works in Meet, Zoom and Teams; Webex still needs testing.",
            "Speaker names carry over between meetings once someone is named once.",
        ],
        action_items: [
            { id: "a1", task: "Test meeting detection in the Webex desktop app", assignee: "Marcus", status: "todo" },
            { id: "a2", task: "Write release notes for 0.3", assignee: "You", status: "done" },
            { id: "a3", task: "Send the beta build to the design group", assignee: "Priya", status: "todo" },
        ],
        turns: [
            turn("you", "You", 4, 19, 0, "Okay, let's start with the launch date. I think we should push to the fourteenth so Windows ships together with macOS."),
            turn("spk_priya", "Priya", 20, 33, 1, "Agreed. The installer is almost there, it just needs the signing step, and I'd rather not ship Mac alone."),
            turn("spk_marcus", "Marcus", 34, 51, 1, "Detection is solid in Meet, Zoom and Teams. I haven't tried Webex yet, so I'll take that this week."),
            turn("you", "You", 52, 63, 0, "Great. And the speaker names, once you name someone, they stick for the next call?"),
            turn("spk_priya", "Priya", 64, 78, 1, "Yes, it matches on the voiceprint. I named Marcus once on Monday and he was recognized in every call since."),
        ],
    },
    {
        id: 11, session_id: "demo-11", title: "Pricing review", platform: "zoom", app_name: "zoom.us",
        url: "", created_at: at(27 * HOUR), duration_ms: 18 * 60_000, category: "work",
        summary: ["Keep the app free and open source.", "Offer paid support for teams later."],
        action_items: [{ id: "b1", task: "Draft the support plan", assignee: "Dana", status: "todo" }],
        turns: [
            turn("spk_dana", "Dana", 2, 14, 1, "I don't think we should charge for the app itself. The whole point is that it runs on your own machine."),
            turn("you", "You", 15, 24, 0, "Same. If anything, teams would pay for setup help and priority fixes."),
        ],
    },
    {
        id: 10, session_id: "demo-10", title: "Standup", platform: "teams", app_name: "Microsoft Teams",
        url: "", created_at: at(50 * HOUR), duration_ms: 9 * 60_000, category: "work",
        summary: ["Overlay bug on external monitors is fixed.", "Qwen3 model download is faster with the new mirror."],
        action_items: [],
        turns: [turn("spk_marcus", "Marcus", 1, 9, 1, "The overlay was showing up on the wrong screen with two monitors. That's fixed now.")],
    },
    {
        id: 9, session_id: "demo-9", title: "Interview: Sam Okafor", platform: "slack", app_name: "Slack",
        url: "", created_at: at(75 * HOUR), duration_ms: 42 * 60_000, category: "interview",
        summary: ["Sam dictates most of their writing and wants it to work offline on flights."],
        action_items: [{ id: "c1", task: "Send Sam the beta link", assignee: "You", status: "done" }],
        turns: [turn("spk_sam", "Sam", 3, 16, 1, "Most of my writing is dictated now, but nothing I use works on a plane.")],
    },
];

const vault = [
    { id: "spk_priya", name: "Priya", created_at: at(200 * HOUR), sample_count: 6, meeting_count: 5, last_seen: at(3 * HOUR), snippet_path: null },
    { id: "spk_marcus", name: "Marcus", created_at: at(180 * HOUR), sample_count: 4, meeting_count: 4, last_seen: at(3 * HOUR), snippet_path: null },
    { id: "spk_dana", name: "Dana", created_at: at(90 * HOUR), sample_count: 2, meeting_count: 1, last_seen: at(27 * HOUR), snippet_path: null },
    { id: "spk_sam", name: "Sam", created_at: at(75 * HOUR), sample_count: 1, meeting_count: 1, last_seen: at(75 * HOUR), snippet_path: null },
];

const settings: Record<string, unknown> = {
    setup_complete: true,
    active_engine: "granite",
    granite_model: "granite-speech-5-nc",
    enable_grammar_lm: true,
    enable_overlay: true,
    transcription_style: "Clean",
};

const INSTALLED = new Set([
    "granite-speech-5-nc", "qwen3-asr-0.6b", "whisper-base-en-q5_1", "whisper-small-en-q5_1",
    "flowscribe-qwen3.5-0.8b-v3", "speaker-campplus-en", "diarization-nemotron3", "whisper-base-en-coreml",
]);

// ── Events ──────────────────────────────────────────────────────────────────
const callbacks = new Map<number, Callback>();
const listeners = new Map<string, Set<number>>();
let cbSeq = 1;

export function emitDemo(event: string, payload: unknown) {
    for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
}

// ── Recording simulation ────────────────────────────────────────────────────
export const DEMO_DICTATION = "Let's ship the beta on Friday. I'll write the release notes tonight and send them to Priya first.";
let levelTimer: ReturnType<typeof setInterval> | null = null;
let recStart = 0;

function startLevels() {
    recStart = Date.now();
    let t = 0;
    levelTimer = setInterval(() => {
        t += 1;
        // Speech-like envelope: syllable bursts with short pauses.
        const burst = Math.abs(Math.sin(t * 0.55)) * (0.5 + 0.5 * Math.abs(Math.sin(t * 0.13)));
        emitDemo("audio-level", Math.min(1, 0.08 + burst * 0.8));
    }, 50);
}

function stopLevels() {
    if (levelTimer) clearInterval(levelTimer);
    levelTimer = null;
    emitDemo("audio-level", 0);
}

const ok = <T,>(data: T) => ({ ok: true, data, error: null });

function handle(cmd: string, args: Record<string, unknown> = {}): unknown {
    switch (cmd) {
        // Tauri plugins
        case "plugin:event|listen": {
            const ev = String(args.event);
            if (!listeners.has(ev)) listeners.set(ev, new Set());
            listeners.get(ev)!.add(args.handler as number);
            return args.handler;
        }
        case "plugin:event|unlisten": return null;
        case "plugin:store|load": return 1;
        case "plugin:store|get": {
            const k = String(args.key);
            return [settings[k] ?? null, k in settings];
        }
        case "plugin:store|set": settings[String(args.key)] = args.value; return null;
        case "plugin:autostart|is_enabled": return false;
        case "plugin:window|is_maximized": return false;
        case "plugin:window|is_fullscreen": return false;

        // System
        case "get_backend_info": return "Metal (Apple M4)";
        case "get_platform": return "macos";
        case "is_apple_silicon": return true;
        case "get_system_info": return { cpu_name: "Apple M4", cpu_cores: 10, ram_total_gb: 16, gpu_name: "Apple M4", cuda_available: false, vram_gb: null, backend_hint: "Metal" };
        case "check_microphone_permission": return "granted";
        case "check_accessibility_permission":
        case "check_input_monitoring_permission": return true;
        case "list_input_devices": return ["MacBook Air Microphone", "AirPods Pro"];
        case "get_active_input_device": return "MacBook Air Microphone";
        case "get_hotkey": return { keys: ["ControlLeft", "MetaLeft"], mode: "hold" };
        case "get_close_behavior": return "tray";

        // Models
        case "get_download_status":
            return (args.modelIds as string[]).map((id) => ({ id, downloaded: INSTALLED.has(id), verified: INSTALLED.has(id), size_on_disk: 0 }));
        case "list_models": return [
            { id: "base.en-q5_1", display_name: "Base English (Q5_1)", file_name: "ggml-base.en-q5_1.bin", size_mb: 57, has_coreml: true },
            { id: "small.en-q5_1", display_name: "Small English (Q5_1)", file_name: "ggml-small.en-q5_1.bin", size_mb: 181, has_coreml: false },
        ];
        case "list_granite_models": return [{ id: "granite-speech-5-nc", display_name: "Granite Speech 5", model_type: "gguf", size_mb: 948 }];
        case "list_qwen3_models": return [{ id: "qwen3-asr-0.6b", display_name: "Qwen3-ASR 0.6B", size_mb: 1600 }];
        case "get_engine_selection_state":
            return { active_engine: "granite", selected_model_id: "granite-speech-5-nc", loaded_engine: "granite", loaded_model_id: "granite-speech-5-nc", backend: "Metal", engine_loading: false };
        case "get_granite_status": return { loaded: true, model_id: "granite-speech-5-nc", model_type: "gguf", backend: "Metal" };
        case "get_qwen3_status": return { loaded: false, model_id: null };
        case "get_current_model": return null;
        case "get_auto_unload_status": return { timeout_seconds: 1800, remaining_seconds: 1500, is_loaded: true, last_activity_epoch: Math.floor(now / 1000) };
        case "check_grammar_llm_available": return true;
        case "init_llm": return "FlowScribe V3 (beta) ready";
        case "check_llm_status": return true;
        case "init_granite":
        case "init_qwen3":
        case "switch_model": return ok("Granite Speech 5 loaded");
        case "unload_current_model": return ok(null);
        case "type_text": return ok(null);

        // Recording
        case "start_recording": startLevels(); return ok("/tmp/demo.wav");
        case "stop_recording": {
            stopLevels();
            const text = DEMO_DICTATION;
            return ok(text);
        }
        case "cancel_recording": stopLevels(); return ok(null);
        case "correct_text": return args.text;
        case "save_transcript_history": {
            history.unshift({
                id: nextId++, created_at: new Date().toISOString(), transcript: String(args.transcript ?? DEMO_DICTATION),
                engine: "granite", duration_ms: Date.now() - recStart, grammar_llm_used: false, processing_time_ms: 390,
                model_id: "granite-speech-5-nc", audio_source: "microphone", kind: "dictation",
            });
            return ok(null);
        }
        case "list_transcript_history": return history;

        // Meetings
        case "get_meeting_detection_status": return { is_watching: true, active_meetings_count: 0, active_meetings: [] };
        case "list_meetings":
            return meetings.map((m) => ({
                id: m.id, session_id: m.session_id, title: m.title, platform: m.platform, app_name: m.app_name, url: m.url,
                created_at: m.created_at, duration_ms: m.duration_ms, category: m.category,
                speaker_count: new Set(m.turns.map((t) => t.speaker_id)).size,
                snippet_preview: m.turns[0]?.text ?? "", action_item_count: m.action_items.length, has_audio: true,
            }));
        case "get_meeting_platform_counts": {
            const counts = new Map<string, number>();
            meetings.forEach((m) => counts.set(m.platform, (counts.get(m.platform) ?? 0) + 1));
            return [...counts].map(([platform, count]) => ({ platform, count }));
        }
        case "get_meeting_detail": {
            const m = meetings.find((x) => x.id === args.id || x.id === args.meetingId) ?? meetings[0];
            return {
                ...m, audio_path: null, transcript_raw: m.turns.map((t) => `${t.speaker_name}: ${t.text}`).join("\n"),
                speaker_count: new Set(m.turns.map((t) => t.speaker_id)).size,
            };
        }
        case "list_speaker_vault": return vault;
        case "get_audio_source_mode": return "mic";
        case "get_auto_record_meetings": return false;
        case "get_speaker_match_threshold": return 0.6;
        case "get_meeting_continue_minutes": return 10;
        case "get_mcp_setup": return { command: "/Applications/Taurscribe.app/Contents/MacOS/taurscribe", args: ["mcp"] };
        case "get_storage_locations": return [
            { area: "models", path: "/Users/me/Library/Application Support/Taurscribe/models", default_path: "/Users/me/Library/Application Support/Taurscribe/models", is_custom: false, is_other_drive: false, is_removable: false, available: true, used_bytes: 4.2e9, free_bytes: 212e9, drive_name: "Macintosh HD" },
            { area: "recordings", path: "/Users/me/Library/Application Support/Taurscribe/meetings", default_path: "/Users/me/Library/Application Support/Taurscribe/meetings", is_custom: false, is_other_drive: false, is_removable: false, available: true, used_bytes: 184e6, free_bytes: 212e9, drive_name: "Macintosh HD" },
        ];
        default: return null;
    }
}

export function installDemoBackend() {
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (_event: string, id: number) => { callbacks.delete(id); },
    };
    w.__TAURI_INTERNALS__ = {
        invoke: async (cmd: string, args?: Record<string, unknown>) => handle(cmd, args),
        transformCallback: (cb: Callback) => { const id = cbSeq++; callbacks.set(id, cb); return id; },
        unregisterCallback: (id: number) => { callbacks.delete(id); },
        convertFileSrc: (p: string) => p,
        metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    };
    (window as unknown as { demo: unknown }).demo = { emit: emitDemo };
}
