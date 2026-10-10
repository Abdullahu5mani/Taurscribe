import { expect, test } from "bun:test";
import { conversationBlocks, spokenAt, speakerCountHint } from "../../src/utils/meetingTimeline";
const turn = (id: string, start: number, end: number, text = id) => ({ speaker_id: id, speaker_name: id, start_ms: start, end_ms: end, channel: 1, text });
test("display joins adjacent turns without mutating originals or inventing speech in gaps", () => {
  const source = [turn("a",0,1000), turn("a",1500,2000), turn("b",2000,3000), turn("a",3000,4000)];
  const blocks = conversationBlocks(source);
  expect(blocks.length).toBe(3); expect(source.length).toBe(4); expect(source[0].end_ms).toBe(1000);
  expect(blocks[0].text).toBe("a a"); expect(spokenAt(blocks[0].intervals,1.25)).toBe(false);
  expect(spokenAt(blocks[0].intervals,1.5)).toBe(true); expect(spokenAt(blocks[0].intervals,2)).toBe(false);
});
test("overlapping speakers both highlight, including fractional scrub positions", () => {
  const blocks = conversationBlocks([turn("a",0,2000),turn("b",1000,3000)]);
  expect(blocks.filter(b=>spokenAt(b.intervals,1.001)).length).toBe(2);
  expect(blocks.filter(b=>spokenAt(b.intervals,3)).length).toBe(0);
});

test("speaker count is optional but rejects invalid or fractional counts", () => {
  expect(speakerCountHint("")).toBeNull(); expect(speakerCountHint("4")).toBe(4);
  for (const bad of ["0","33","2.5","unknown"]) expect(() => speakerCountHint(bad)).toThrow("1–32");
});
