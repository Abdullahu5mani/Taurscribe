// Bun scopes a hook registered by an imported module to the first test file
// that imports it, so hookHarness cannot register this itself: later files
// would keep components (and their timers) mounted into the next tests.
// Hooks registered in a preload run after every test in every file.
import { afterEach } from "bun:test";

afterEach(() => {
    const mounted: Set<() => void> | undefined = (globalThis as any).__hookHarnessMounted;
    for (const unmount of [...(mounted ?? [])]) unmount();
});
