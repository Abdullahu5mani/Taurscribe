/** One foreground audio player across meeting clips, vault previews and recordings. */
let current: { audio: HTMLAudioElement; interrupted: () => void } | null = null;
const watched = new WeakSet<HTMLAudioElement>();
export function claimAudioPlayback(audio: HTMLAudioElement, interrupted: () => void = () => {}) {
    if (current?.audio === audio) return;
    const previous = current;
    current = { audio, interrupted };
    if (previous) { previous.audio.pause(); previous.interrupted(); }
    if (watched.has(audio)) return;
    watched.add(audio);
    const previousPause = audio.onpause;
    audio.onpause = event => {
        if (current?.audio === audio) current = null;
        previousPause?.call(audio, event);
    };
}
