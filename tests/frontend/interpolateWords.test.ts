import { expect, test } from "bun:test";
import { interpolateWords } from "../../src/utils/meetingTimeline";

test("words cover the turn exactly, in order, longer words taking longer", () => {
    const words = interpolateWords("I think we should reconsider this", 10_000, 14_000);
    expect(words.map(w => w.text)).toEqual(["I", "think", "we", "should", "reconsider", "this"]);
    expect(words[0].start_ms).toBe(10_000);
    expect(words.at(-1)!.end_ms).toBe(14_000);
    for (let i = 1; i < words.length; i++) expect(words[i].start_ms).toBe(words[i - 1].end_ms);
    const dur = (i: number) => words[i].end_ms - words[i].start_ms;
    expect(dur(4)).toBeGreaterThan(dur(0));
});

test("empty and one-word turns are safe", () => {
    expect(interpolateWords("   ", 0, 1000)).toEqual([]);
    expect(interpolateWords("Okay", 500, 900)).toEqual([{ text: "Okay", start_ms: 500, end_ms: 900 }]);
    // A zero-length turn still yields ordered, non-negative times.
    const w = interpolateWords("a b", 200, 200);
    expect(w.every(x => x.start_ms === 200 && x.end_ms === 200)).toBe(true);
});
