test("a Stop waiting on tray state cannot stop a newer externally started capture", async () => {
    const tray = deferred<void>();
    const h = fixture({ tray: async (s) => { if (s === "processing_speech") await tray.promise; } });
    await h.handle.current.handleStartRecording();
    const stopping = h.handle.current.handleStopRecording(); await h.settle();
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    fire("recording-state", { is_recording: true, is_dual_channel: true }); await h.settle();
    tray.resolve(); await stopping; await h.settle();
    trace("final.superseded-stop", { commands: calls, recording: h.handle.current.isRecording });
    expect(calls.filter(c => c === "stop_recording")).toHaveLength(0);
    expect(h.handle.current.isRecording).toBe(true);
});

test("a failed Start waiting on cleanup cannot clear a newer external recording", async () => {
    const tray = deferred<void>();
    const h = fixture({ tray: async (s) => { if (s === "ready") await tray.promise; } });
    responses.set("start_recording", () => { throw new Error("synthetic device failure"); });
    const starting = h.handle.current.handleStartRecording(); await h.settle();
    fire("recording-state", { is_recording: true, is_dual_channel: true }); await h.settle();
    tray.resolve(); await starting; await h.settle();
    trace("final.superseded-start-cleanup", { recording: h.handle.current.isRecording, phases: h.phases });
    expect(h.handle.current.isRecording).toBe(true);
    expect(h.handle.current.isDualChannelRecording).toBe(true);
    expect(h.phases.at(-1)).toBe("recording");
});

for (const ending of ["meeting", "too-short", "silence", "dictation", "paste-failed"] as const) {
    test(`a delayed ${ending} tray response cannot overwrite a newer recording`, async () => {
        const tray = deferred<void>();
        const blockedState = ending === "too-short" || ending === "silence" ? "nothing_heard" : ending === "paste-failed" ? "paste_failed" : "done";
        const h = fixture({ tray: async (s) => { if (s === blockedState) await tray.promise; } });
        await h.handle.current.handleStartRecording();
        const outcome = stopped();
        if (ending === "meeting") outcome.data.kind = "meeting";
        if (ending === "too-short") outcome.data.too_short = true;
        if (ending === "silence") outcome.data.transcript = "";
        responses.set("stop_recording", () => outcome);
        if (ending === "paste-failed") responses.set("type_text", () => ({ ok: false, error: { code: "secure_input", message: "blocked" } }));
        const stop = h.handle.current.handleStopRecording(); await h.settle();
        fire("recording-state", { is_recording: false, is_dual_channel: false });
        fire("recording-state", { is_recording: true, is_dual_channel: true });
        h.handle.current.handleTranscriptionChunk("Synthetic new meeting speech"); await h.settle();
        tray.resolve(); await stop; await h.settle();
        trace("final.terminal-tray", { ending, phases: h.phases, transcripts: h.transcripts, busy: h.handle.current.isProcessingTranscript });
        expect(h.handle.current.isRecording).toBe(true);
        expect(h.phases.at(-1)).toBe("recording");
        expect(h.transcripts.at(-1)).toBe("Synthetic new meeting speech");
    });
}

for (const ending of ["meeting", "too-short", "silence"] as const) {
    test(`a delayed ${ending} unmute cannot reset a newer recording's transcript`, async () => {
        const h = fixture({ mute: true }); await h.handle.current.handleStartRecording();
        const unmute = deferred<null>(); responses.set("unmute_system_audio", () => unmute.promise);
        const outcome = stopped();
        if (ending === "meeting") outcome.data.kind = "meeting";
        if (ending === "too-short") outcome.data.too_short = true;
        if (ending === "silence") outcome.data.transcript = "";
        responses.set("stop_recording", () => outcome);
        const stop = h.handle.current.handleStopRecording(); await h.settle();
        fire("recording-state", { is_recording: false, is_dual_channel: false });
        fire("recording-state", { is_recording: true, is_dual_channel: true });
        h.handle.current.handleTranscriptionChunk("Synthetic new capture"); await h.settle();
        unmute.resolve(null); await stop; await h.settle();
        trace("final.unmute", { ending, phases: h.phases, transcripts: h.transcripts });
        expect(h.handle.current.isRecording).toBe(true);
        expect(h.transcripts.at(-1)).toBe("Synthetic new capture");
        expect(h.phases.at(-1)).toBe("recording");
    });
}

for (const action of ["pause", "resume"] as const) {
    test(`a failed ${action} request releases its guard and allows a retry`, async () => {
        const h = fixture(); await h.handle.current.handleStartRecording();
        if (action === "resume") await h.handle.current.handlePauseRecording();
        responses.set(`${action}_recording`, () => { throw new Error("temporary capture failure"); });
        await h.handle.current[action === "pause" ? "handlePauseRecording" : "handleResumeRecording"]();
        responses.delete(`${action}_recording`);
        await h.handle.current[action === "pause" ? "handlePauseRecording" : "handleResumeRecording"](); await h.settle();
        expect(h.handle.current.isPaused).toBe(action === "pause");
    });
}
