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

test("a fast double click moves one step, not two", async () => {
    const view = await openWizard();
    const step = currentStep(view.handle.current);
    step.props.onNext();
    await view.settle();
    currentStep(view.handle.current).props.onNext(); // during the 400 ms transition
    await view.settle();
    expect(currentStep(view.handle.current).type.name).toBe("StepHardware");
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
