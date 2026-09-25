/**
 * Dev-only design preview (http://localhost:1420/#preview/<view>), for checking
 * UI in a normal browser without the Tauri backend. Views: settings, overlay,
 * wizard, and app (the whole app on example data, see demoBackend.ts). A tiny stand-in for the Tauri IPC answers the calls these screens
 * make with plausible data, so they render.
 */
import { lazy, Suspense, useState } from "react";
import { SettingsModal } from "../components/SettingsModal";
import { SetupWizard } from "../components/SetupWizard";
import { MODELS } from "../components/settings/types";

const FAKE: Record<string, unknown> = {
  get_meeting_detection_status: { is_watching: true, active_meetings_count: 0, active_meetings: [] },
  get_audio_source_mode: "mic",
  get_auto_record_meetings: false,
  get_speaker_match_threshold: 0.6,
  get_meeting_continue_minutes: 10,
  get_mcp_setup: { command: "/Applications/Taurscribe.app/Contents/MacOS/taurscribe", args: ["mcp"] },
  get_hotkey: { keys: ["ControlLeft", "AltLeft"], mode: "hold" },
  list_input_devices: ["MacBook Pro Microphone", "AirPods Pro"],
  get_platform: "macos",
  is_apple_silicon: true,
  get_system_info: { cpu_name: "Apple M4", cpu_cores: 10, ram_total_gb: 16, gpu_name: "Apple M4", cuda_available: false, vram_gb: null, backend_hint: "Metal" },
  get_close_behavior: "tray",
  get_storage_locations: [
    { area: "models", path: "/Volumes/ExternalSSD/TaurscribeData/models", default_path: "/Users/me/Library/Application Support/Taurscribe/models", is_custom: true, is_other_drive: true, is_removable: true, available: true, used_bytes: 7.1e9, free_bytes: 412e9, drive_name: "ExternalSSD" },
    { area: "recordings", path: "/Users/me/Library/Application Support/Taurscribe/meetings", default_path: "/Users/me/Library/Application Support/Taurscribe/meetings", is_custom: false, is_other_drive: false, is_removable: false, available: true, used_bytes: 184e6, free_bytes: 96e9, drive_name: "Macintosh HD" },
  ],
  measure_storage_speed: { path: "", write_mb_s: 742, read_mb_s: 868 },
  "plugin:store|load": 1,
  "plugin:store|get": [null, false],
};

function installFakeTauri() {
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown; __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown };
  if (w.__TAURI_INTERNALS__) return;
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  w.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string) => (cmd in FAKE ? FAKE[cmd] : null),
    transformCallback: () => 0,
    convertFileSrc: (p: string) => p,
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
}


const OverlayGallery = lazy(() => import("./OverlayGallery"));
const OverlaySequence = lazy(() => import("./OverlayGallery").then((m) => ({ default: m.OverlaySequence })));
const DemoApp = lazy(() => import("./demoBackend").then(({ installDemoBackend }) => {
  installDemoBackend();
  return import("../App");
}));

export function Preview({ view }: { view: string }) {
  const [open, setOpen] = useState(true);
  if (view === "app") return <Suspense fallback={null}><DemoApp /></Suspense>;
  installFakeTauri();
  const noop = () => {};
  if (view === "overlay/sequence") return <Suspense fallback={null}><OverlaySequence /></Suspense>;
  if (view === "overlay") return <Suspense fallback={null}><OverlayGallery /></Suspense>;
  if (view === "wizard") {
    return (
      <SetupWizard
        onComplete={noop}
        handleDownload={noop}
        handleCancelDownload={noop}
        downloadProgress={{}}
        settingsModels={MODELS}
        enableDenoise={true}
        setEnableDenoise={noop}
        enableOverlay={true}
        setEnableOverlay={noop}
        muteBackgroundAudio={false}
        setMuteBackgroundAudio={noop}
      />
    );
  }
  const tab = (view.split("/")[1] || "app") as never;
  return (
    <div style={{ height: "100vh", background: "#000" }}>
      <button type="button" id="preview-open-settings-btn" data-testid="preview-open-settings-btn" style={{ margin: 20 }} onClick={() => setOpen(true)}>Open settings</button>
      <SettingsModal
        isOpen={open}
        onClose={() => setOpen(false)}
        initialTab={tab}
        enableGrammarLM={true} setEnableGrammarLM={noop} llmStatus="Loaded"
        enableDenoise={true} setEnableDenoise={noop}
        muteBackgroundAudio={false} setMuteBackgroundAudio={noop}
        enableOverlay={true} setEnableOverlay={noop}
        llmBackend="gpu" setLlmBackend={noop}
        transcriptionStyle="Casual" setTranscriptionStyle={noop}
        soundVolume={0.6} soundMuted={false} setSoundVolume={noop} setSoundMuted={noop}
        dictionary={[]} addDictEntry={noop} updateDictEntry={noop} removeDictEntry={noop}
        snippets={[]} addSnippet={noop} updateSnippet={noop} removeSnippet={noop}
        settingsModels={MODELS.map((m, i) => ({ ...m, downloaded: i % 3 === 0 }))}
        downloadProgress={{}}
        onDownload={noop} onDelete={async () => {}} onCancelDownload={noop}
        closeBehavior="tray" setCloseBehavior={noop}
      />
    </div>
  );
}
