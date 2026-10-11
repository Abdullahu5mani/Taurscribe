/** One explicitly selected audio player across meeting clips, vaults and recordings. */
type Player = { audio: HTMLAudioElement; interrupted: () => void };
let current: Player | null = null;
let mediaTarget: Player | null = null;
let playbackAllowed = true;
let generation = 0;
const watched = new WeakMap<HTMLAudioElement, { generation: number; interrupted: () => void }>();

/** Closing to tray revokes even a paused player's permission to resume via media keys. */
export function setAudioPlaybackVisible(visible: boolean) {
    playbackAllowed = visible;
    if (visible) return;
    generation++;
    const previous = current ?? mediaTarget;
    current = null;
    mediaTarget = null;
    if (previous) { previous.audio.pause(); previous.interrupted(); }
    if (typeof navigator !== "undefined" && navigator.mediaSession) {
        navigator.mediaSession.playbackState = "none";
        navigator.mediaSession.metadata = null;
    }
}

export function installAudioPlaybackLifecycle() {
    const visibility = () => setAudioPlaybackVisible(document.visibilityState !== "hidden");
    const focus = () => setAudioPlaybackVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("focus", focus);
    visibility();
    const session = navigator.mediaSession;
    if (session) {
        // Override the browser's default resume of a stale, hidden HTMLAudioElement.
        session.setActionHandler("play", () => {
            if (!playbackAllowed || !mediaTarget) return;
            const { audio, interrupted } = mediaTarget;
            claimAudioPlayback(audio, interrupted);
            void audio.play().catch(() => interrupted());
        });
        session.setActionHandler("pause", () => current?.audio.pause());
        session.setActionHandler("stop", () => {
            const visible = playbackAllowed;
            setAudioPlaybackVisible(false);
            if (visible) setAudioPlaybackVisible(true);
        });
    }
    return () => {
        document.removeEventListener("visibilitychange", visibility);
        window.removeEventListener("focus", focus);
        setAudioPlaybackVisible(false);
        if (session) for (const action of ["play", "pause", "stop"] as const) session.setActionHandler(action, null);
    };
}

/** Dispose a player when its screen or recording is replaced. */
export function releaseAudioPlayback(audio: HTMLAudioElement) {
    const watch = watched.get(audio);
    if (watch) watch.generation = -1;
    if (mediaTarget?.audio === audio) mediaTarget = null;
    if (current?.audio === audio) { current = null; watch?.interrupted(); }
    audio.pause();
}

export function claimAudioPlayback(audio: HTMLAudioElement, interrupted: () => void = () => {}) {
    if (!playbackAllowed) { audio.pause(); interrupted(); return; }
    let watch = watched.get(audio);
    if (!watch) {
        watch = { generation, interrupted };
        watched.set(audio, watch);
        const previousPause = audio.onpause;
        const previousPlay = audio.onplay;
        audio.onpause = event => {
            if (current?.audio === audio) { current = null; watch!.interrupted(); }
            previousPause?.call(audio, event);
        };
        audio.onplay = event => {
            if (!playbackAllowed || watch!.generation !== generation) { audio.pause(); return; }
            previousPlay?.call(audio, event);
        };
    }
    watch.generation = generation;
    watch.interrupted = interrupted;
    if (current?.audio === audio) return;
    const previous = current;
    current = mediaTarget = { audio, interrupted };
    if (previous) { previous.audio.pause(); previous.interrupted(); }
}
