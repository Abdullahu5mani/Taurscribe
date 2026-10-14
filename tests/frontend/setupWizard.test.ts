import { beforeEach, expect, mock, setDefaultTimeout, test } from "bun:test";

// Each step waits out the wizard's 400 ms slide transition.
setDefaultTimeout(20_000);
import { renderHook } from "./hookHarness";

// First-run setup: every step forwards and back on Windows/Linux (8 steps)
// and macOS (9, with permissions), the model download on the last step,
// Launch / Skip, and the permission states (granted, denied, restricted,
// granted later → restart).
let platform = "windows";
let perms: { mic: string; acc: boolean; input: boolean };
let systemAudio = "granted";
const calls: string[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string) => {
        calls.push(cmd);
        switch (cmd) {
            case "get_system_info": return { cpu_name: "Ryzen 7 8845HS", cpu_cores: 16, ram_total_gb: 15.3, gpu_name: "NVIDIA GeForce RTX 4070 Laptop GPU", cuda_available: true, vram_gb: 8, backend_hint: "CUDA" };
            case "get_platform": return platform;
            case "is_apple_silicon": return platform === "macos";
            case "get_gpu_report": return null;
            case "check_microphone_permission": return perms.mic;
            case "check_meeting_permissions": return { microphone: perms.mic, system_audio: systemAudio };
            case "request_system_audio_permission": systemAudio = "granted"; return systemAudio;
            case "check_accessibility_permission": return perms.acc;
            case "check_input_monitoring_permission": return perms.input;
            case "request_microphone_permission": perms.mic = "granted"; return null;
            case "request_accessibility_permission": return false;
            default: return null;
        }
    },
}));
const { SetupWizard } = await import("../../src/components/SetupWizard");

const nodes = (root: any): any[] => {
    if (root == null || typeof root !== "object") return [];
    if (Array.isArray(root)) return root.flatMap(nodes);
    return [root, ...nodes(root.props?.children)];
};
const byTest = (root: any, id: string) => nodes(root).find((n) => n.props?.["data-testid"] === id);
const textOf = (root: any): string => {
    if (root == null || typeof root === "boolean") return "";
    if (typeof root !== "object") return String(root);
    if (Array.isArray(root)) return root.map(textOf).join(" ");
    return textOf(root.props?.children);
};
/** The step element on screen (not the one animating out). */
const currentStep = (wizard: any) =>
    nodes(wizard).find((n) => typeof n.props?.className === "string" && n.props.className.includes("setup-step--enter")).props.children;
/** Renders a step element's own component, as React would. */
const renderStep = async (el: any) => {
    const view = renderHook(() => el.type(el.props));
    await view.settle();
    return view;
};
const pastTransition = () => new Promise((r) => setTimeout(r, 420));

let results: any[];
let downloads: string[];
let cancels: string[];
let props: any;
beforeEach(() => {
    platform = "windows";
    perms = { mic: "granted", acc: true, input: true };
    systemAudio = "granted";
    calls.length = 0;
    results = [];
    downloads = [];
    cancels = [];
    props = {
        onComplete: (r: any) => results.push(r),
        handleDownload: (id: string) => downloads.push(id),
        handleCancelDownload: (id: string) => cancels.push(id),
        downloadProgress: {},
        settingsModels: [],
        enableDenoise: true, setEnableDenoise: () => {},
        enableOverlay: true, setEnableOverlay: () => {},
        muteBackgroundAudio: false, setMuteBackgroundAudio: () => {},
    };
});

const openWizard = async () => {
    const view = renderHook(() => SetupWizard(props));
    await view.settle();
    return view;
};
const stepNames = async (view: any, forward: boolean, count: number) => {
    const names: string[] = [];
    for (let i = 0; i < count; i++) {
        const step = currentStep(view.handle.current);
        names.push(step.type.name);
        (forward ? step.props.onNext : step.props.onBack)();
        await view.settle();
        await pastTransition();
    }
    names.push(currentStep(view.handle.current).type.name);
    return names;
};

test("Windows / Linux: eight steps, no permissions step, and Back returns through all of them", async () => {
    const view = await openWizard();
    expect(nodes(view.handle.current).filter((n) => String(n.props?.className).startsWith("setup-dot "))).toHaveLength(8);
    expect(await stepNames(view, true, 7)).toEqual([
        "StepWelcome", "StepHardware", "StepEngines", "StepFeatures", "StepFlowScribe",
        "StepHotkey", "StepRecordingSettings", "StepReady",
    ]);
    // StepReady has no Back; go back from the step before it.
    const view2 = await openWizard();
    await stepNames(view2, true, 6);
    expect(await stepNames(view2, false, 5)).toEqual([
        "StepRecordingSettings", "StepHotkey", "StepFlowScribe", "StepFeatures", "StepEngines", "StepHardware",
    ]);
});

test("macOS: nine steps with permissions before the last", async () => {
    platform = "macos";
    perms = { mic: "denied", acc: false, input: false };
    const view = await openWizard();
    expect(nodes(view.handle.current).filter((n) => String(n.props?.className).startsWith("setup-dot "))).toHaveLength(9);
    const names = await stepNames(view, true, 7);
    expect(names.at(-1)).toBe("StepPermissions");
});

test("a fast double click moves one step, not two", async () => {
    const view = await openWizard();
    const step = currentStep(view.handle.current);
    step.props.onNext();
    await view.settle();
    currentStep(view.handle.current).props.onNext(); // during the 400 ms transition
    await view.settle();
    expect(currentStep(view.handle.current).type.name).toBe("StepHardware");
});

test("the last step downloads the recommended model, can cancel, and launches only once it is ready", async () => {
    const view = await openWizard();
    await stepNames(view, true, 7);
    let ready = await renderStep(currentStep(view.handle.current));
    const download = byTest(ready.handle.current, "wizard-ready-download-model-btn");
    expect(byTest(ready.handle.current, "wizard-ready-launch-btn").props.disabled).toBe(true);
    download.props.onClick();
    const modelId = downloads[0];
    expect(modelId).toBeTruthy();

    props = { ...props, downloadProgress: { [modelId]: { status: "downloading", bytes: 50, total: 200, current_file: 2, total_files: 2 } } };
    view.rerender();
    ready = await renderStep(currentStep(view.handle.current));
    expect(textOf(ready.handle.current)).toContain("63%"); // file 2 of 2 at 25% → monotonic overall
    expect(textOf(byTest(ready.handle.current, "wizard-ready-bundle-indicator")).replace(/\s+/g, " ")).toContain("File 2 / 2");
    byTest(ready.handle.current, "wizard-ready-cancel-download-btn").props.onClick();
    expect(cancels).toEqual([modelId]);
    byTest(ready.handle.current, "wizard-ready-launch-btn").props.onClick();
    expect(results).toEqual([]);

    for (const status of ["copying", "cancelling"]) {
        props = { ...props, downloadProgress: { [modelId]: { status, bytes: 50, total: 200 } } };
        view.rerender(); ready = await renderStep(currentStep(view.handle.current));
        expect(byTest(ready.handle.current, "wizard-ready-download-model-btn")).toBeUndefined();
        expect(textOf(ready.handle.current)).toContain(status === "copying" ? "Copying…" : "Cancelling…");
        expect(byTest(ready.handle.current, "wizard-ready-launch-btn").props.disabled).toBe(true);
    }

    props = { ...props, downloadProgress: {}, settingsModels: [{ id: modelId, downloaded: true }] };
    view.rerender();
    ready = await renderStep(currentStep(view.handle.current));
    expect(byTest(ready.handle.current, "wizard-ready-launch-btn").props.disabled).toBe(false);
    byTest(ready.handle.current, "wizard-ready-launch-btn").props.onClick();
    expect(results).toEqual([{ openSettings: false, useCase: "quick_notes", modelId }]);
});

test("Skip finishes setup and opens the Models tab without a model", async () => {
    const view = await openWizard();
    await stepNames(view, true, 7);
    const ready = await renderStep(currentStep(view.handle.current));
    byTest(ready.handle.current, "wizard-ready-skip-btn").props.onClick();
    expect(results).toEqual([{ openSettings: true, useCase: "quick_notes", modelId: undefined }]);
});

const permissionsStep = async () => {
    platform = "macos";
    const view = await openWizard();
    await stepNames(view, true, 7);
    return { view, step: currentStep(view.handle.current) };
};

test("permissions: everything already granted skips the step going forward", async () => {
    let advanced = 0;
    const { step } = await permissionsStep();
    await renderStep({ ...step, props: { ...step.props, onNext: () => advanced++ } });
    expect(advanced).toBe(1);
});

test("microphone access alone cannot skip system audio setup", async () => {
    systemAudio = "denied";
    let advanced = 0;
    const { step } = await permissionsStep();
    const view = await renderStep({ ...step, props: { ...step.props, onNext: () => advanced++ } });
    expect(advanced).toBe(0);
    expect(byTest(view.handle.current, "wizard-perm-audio-grant-btn").props["aria-label"]).toBe("Open System Audio Recording Settings");
    await byTest(view.handle.current, "wizard-perm-audio-grant-btn").props.onClick();
    expect(calls).toContain("open_system_audio_settings");
});

test("system audio Grant Access requests permission without marking an unknown grant ready", async () => {
    systemAudio = "undetermined";
    const { step } = await permissionsStep();
    const view = await renderStep(step);
    await byTest(view.handle.current, "wizard-perm-audio-grant-btn").props.onClick();
    await view.settle();
    expect(calls).toContain("request_system_audio_permission");
    expect(byTest(view.handle.current, "wizard-perm-audio-ok")).toBeDefined();
});

test("permissions: denied mic points to Settings, restricted says so, missing ones can be skipped", async () => {
    perms = { mic: "denied", acc: false, input: false };
    const { step } = await permissionsStep();
    let view = await renderStep(step);
    expect(byTest(view.handle.current, "wizard-perm-mic-settings-btn")).toBeDefined();
    expect(byTest(view.handle.current, "wizard-perm-next-btn").props["aria-label"]).toBe("Skip permissions for now");
    await byTest(view.handle.current, "wizard-perm-mic-settings-btn").props.onClick();
    expect(calls).toContain("open_microphone_settings");

    perms = { mic: "restricted", acc: false, input: false };
    view = await renderStep(step);
    expect(byTest(view.handle.current, "wizard-perm-mic-restricted")).toBeDefined();
});

test("permissions: asking for the mic grants it; an Accessibility refusal opens System Settings", async () => {
    perms = { mic: "undetermined", acc: false, input: false };
    const { step } = await permissionsStep();
    const view = await renderStep(step);
    await byTest(view.handle.current, "wizard-perm-mic-grant-btn").props.onClick();
    await view.settle();
    expect(byTest(view.handle.current, "wizard-perm-mic-ok")).toBeDefined();
    await byTest(view.handle.current, "wizard-perm-accessibility-grant-btn").props.onClick();
    expect(calls).toContain("open_accessibility_settings");
});

test("permissions: Accessibility granted while the step is open asks for a restart", async () => {
    perms = { mic: "granted", acc: false, input: true };
    const { step } = await permissionsStep();
    const view = await renderStep(step);
    expect(byTest(view.handle.current, "wizard-perm-restart-notice")).toBeUndefined();
    perms.acc = true;
    await new Promise((r) => setTimeout(r, 1600)); // the step re-checks every 1.5 s
    await view.settle();
    expect(byTest(view.handle.current, "wizard-perm-restart-notice")).toBeDefined();
    await byTest(view.handle.current, "wizard-perm-restart-btn").props.onClick();
    expect(calls).toContain("relaunch_app");
});

test("the engines step continues only after all three engine slides were viewed", async () => {
    const view = await openWizard();
    await stepNames(view, true, 2);
    const engines = currentStep(view.handle.current);
    expect(engines.type.name).toBe("StepEngines");
    const step = await renderStep(engines);
    const next = () => byTest(step.handle.current, "wizard-engines-next-btn");
    expect(next().props.disabled).toBe(true);
    const slideNext = nodes(step.handle.current).find((n) => n.props?.["aria-label"] === "Next engine slide");
    slideNext.props.onClick();
    await step.settle();
    expect(next().props.disabled).toBe(true);
    nodes(step.handle.current).find((n) => n.props?.["aria-label"] === "Next engine slide").props.onClick();
    await step.settle();
    expect(next().props.disabled).toBe(false);
});
