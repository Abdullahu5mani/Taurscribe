import { expect, test } from "bun:test";
import { chronologicalRecordings, moveRecording, recordingWarnings, nearbyRecordingGroups, recordingGroupKey, planRecordingImports, type RecordingSource } from "../../src/utils/meetingImport";

const source = (name: string, start: string | null, duration = 60_000): RecordingSource => ({
  name, path: `/audio/${name}`, duration_ms: duration, recorded_start: start,
  recorded_end: start ? new Date(Date.parse(start) + duration).toISOString() : null,
  file_created_at: "2026-10-09T12:00:00Z", file_modified_at: "2026-10-09T12:00:00Z",
  timestamp_source: start ? "Embedded recording timestamp" : null, offset_ms: 0,
});

test("dated files sort by actual instant; undated files retain selected positions", () => {
  const late = source("late", "2026-10-09T10:00:00-04:00"), early = source("early", "2026-10-09T13:00:00Z"), unknown = source("undated", null);
  const selected = [late, unknown, early];
  expect(chronologicalRecordings(selected).map(s => s.name)).toEqual(["early", "undated", "late"]);
  expect(selected.map(s => s.name)).toEqual(["late", "undated", "early"]);
});

test("large gaps warn but retain all sources; copy dates do not generate warnings", () => {
  const selected = [source("first", "2026-10-09T12:00:00Z"), source("second", "2026-10-10T12:00:00Z")];
  expect(recordingWarnings(selected)[0]).toContain("may be different meetings");
  expect(recordingWarnings(selected)[0]).toContain("still import");
  expect(selected).toHaveLength(2);
  expect(recordingWarnings([source("one", null), source("two", null)])).toEqual([]);
});

test("overlaps and reversed chronology are explicit; manual moves win", () => {
  const a = source("a", "2026-10-09T12:00:00Z"), b = source("b", "2026-10-09T12:00:30Z");
  expect(recordingWarnings([a, b])[0]).toContain("overlap by 30 seconds");
  const reversed = moveRecording([a, b], 1, -1);
  expect(reversed.map(s => s.name)).toEqual(["b", "a"]);
  expect(recordingWarnings(reversed)[0]).toContain("chosen order");
  expect(moveRecording(reversed, 0, -1)).toBe(reversed);
});


test("only real nearby recording times suggest merging; consent affects only that group", () => {
 const a=source("a","2026-10-10T12:00:00Z"), b=source("b","2026-10-10T12:02:00Z"), c=source("c","2026-10-10T14:00:00Z"), d=source("d",null);
 const files=[a,b,c,d];
 expect(nearbyRecordingGroups(files)).toEqual([[a,b]]);
 expect(planRecordingImports(files,false,new Set())).toEqual([[a],[b],[c],[d]]);
 expect(planRecordingImports(files,false,new Set([recordingGroupKey([a,b])]))).toEqual([[a,b],[c],[d]]);
 expect(nearbyRecordingGroups([d,source("unknown",null)])).toEqual([]);
 expect(nearbyRecordingGroups([b,a])).toEqual([]);
});
