import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import { scheduleAutoRecord } from "../../src/utils/autoRecord";

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

function setup(recording = false) {
  const settings = deferred<[boolean, number | undefined]>();
  const calls = { start: 0, countdown: [] as (number | null)[] };
  const cancel = scheduleAutoRecord({
    loadSettings: () => settings.promise,
    defaultDelay: 5,
    isRecording: () => recording,
    start: () => calls.start++,
    onCountdown: (n) => calls.countdown.push(n),
  });
  return { settings, calls, cancel };
}


describe("scheduleAutoRecord", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  test("counts down and starts recording", async () => {
    const { settings, calls } = setup();
    settings.resolve([true, 3]);
    await settings.promise;
    await Promise.resolve();
    expect(calls.countdown).toEqual([3]);
    jest.advanceTimersByTime(3000);
    expect(calls.countdown).toEqual([3, 2, 1, null]);
    expect(calls.start).toBe(1);
  });

  test("a zero delay starts immediately", async () => {
    const { settings, calls } = setup();
    settings.resolve([true, 0]);
    await settings.promise;
    await Promise.resolve();
    expect(calls.start).toBe(1);
  });

  test("cancelling while settings load prevents a later countdown", async () => {
    const { settings, calls, cancel } = setup();
    cancel();
    settings.resolve([true, 3]);
    await settings.promise;
    await Promise.resolve();
    jest.advanceTimersByTime(10_000);
    expect(calls.start).toBe(0);
    expect(calls.countdown).toEqual([null]);
  });

  test("cancelling while settings load prevents an immediate start", async () => {
    const { settings, calls, cancel } = setup();
    cancel();
    settings.resolve([true, 0]);
    await settings.promise;
    await Promise.resolve();
    expect(calls.start).toBe(0);
  });

  test("cancelling mid-countdown stops it", async () => {
    const { settings, calls, cancel } = setup();
    settings.resolve([true, 5]);
    await settings.promise;
    await Promise.resolve();
    jest.advanceTimersByTime(2000);
    cancel();
    jest.advanceTimersByTime(10_000);
    expect(calls.start).toBe(0);
    expect(calls.countdown.at(-1)).toBeNull();
  });

  test("does nothing when disabled or already recording", async () => {
    const off = setup();
    off.settings.resolve([false, 1]);
    const busy = setup(true);
    busy.settings.resolve([true, 1]);
    await Promise.all([off.settings.promise, busy.settings.promise]);
    await Promise.resolve();
    jest.advanceTimersByTime(5000);
    expect(off.calls.start + busy.calls.start).toBe(0);
    expect(off.calls.countdown).toEqual([]);
  });

  test("uses the default delay when none is saved", async () => {
    const { settings, calls } = setup();
    settings.resolve([true, undefined]);
    await settings.promise;
    await Promise.resolve();
    expect(calls.countdown).toEqual([5]);
  });

  test("a failed settings read starts nothing", async () => {
    const calls = { start: 0 };
    scheduleAutoRecord({
      loadSettings: () => Promise.reject(new Error("no store")),
      defaultDelay: 1,
      isRecording: () => false,
      start: () => calls.start++,
      onCountdown: () => {},
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(calls.start).toBe(0);
  });
});
