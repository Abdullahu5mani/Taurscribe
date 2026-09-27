/**
 * Auto-record countdown for a detected meeting.
 *
 * The settings it needs arrive asynchronously, so cancelling has to cover
 * both the countdown and the still-pending settings read: a banner dismissed
 * (or a call that ended) while settings were loading must not start a timer
 * or a recording afterwards.
 */
export interface AutoRecordOptions {
    /** Resolves to [auto-record enabled, saved delay in seconds]. */
    loadSettings: () => Promise<[boolean, number | undefined]>;
    defaultDelay: number;
    isRecording: () => boolean;
    start: () => void;
    /** Seconds left, or null when no countdown is showing. */
    onCountdown: (secondsLeft: number | null) => void;
}

/** Starts the auto-record flow; returns a function that cancels it. */
export function scheduleAutoRecord(opts: AutoRecordOptions): () => void {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    const stopTimer = () => {
        if (timer !== null) {
            clearInterval(timer);
            timer = null;
        }
    };

    opts.loadSettings()
        .then(([autoRecord, savedDelay]) => {
            if (cancelled || !autoRecord || opts.isRecording()) return;
            let timeLeft = savedDelay ?? opts.defaultDelay;
            if (timeLeft <= 0) {
                opts.start();
                return;
            }
            opts.onCountdown(timeLeft);
            timer = setInterval(() => {
                if (cancelled) return;
                timeLeft -= 1;
                if (timeLeft <= 0) {
                    stopTimer();
                    opts.onCountdown(null);
                    if (!opts.isRecording()) opts.start();
                } else {
                    opts.onCountdown(timeLeft);
                }
            }, 1000);
        })
        .catch(() => {});

    return () => {
        cancelled = true;
        stopTimer();
        opts.onCountdown(null);
    };
}
