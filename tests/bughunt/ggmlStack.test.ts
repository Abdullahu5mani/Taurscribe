import { expect, test } from "bun:test";
import { nativeSearchFlags } from "../../scripts/ggml-stack";

test("native archive search paths with spaces remain one rustc argument", () => {
  const flags = nativeSearchFlags("/Volumes/ExternalSSD/Code Projects/lib",
    ["-C", "link-arg=-mmacosx-version-min=14.0"], "-C debuginfo=1");
  expect(flags.split("\x1f")).toEqual([
    "-C", "link-arg=-mmacosx-version-min=14.0", "-C", "debuginfo=1",
    "-L", "native=/Volumes/ExternalSSD/Code Projects/lib",
  ]);
});

test("nested build invocations preserve encoded flags without duplicate search paths", () => {
  const first = nativeSearchFlags("/project with spaces/lib", ["-lc++"]);
  expect(nativeSearchFlags("/project with spaces/lib", ["-lc++"],
    "-C debuginfo=1", "-C opt-level=2", first)).toBe(first);
});

test("explicit RUSTFLAGS retain Cargo precedence over target configuration", () => {
  expect(nativeSearchFlags("/lib", ["-lc++"], "-C debuginfo=1", "-C opt-level=2")
    .split("\x1f")).toEqual(["-C", "opt-level=2", "-L", "native=/lib"]);
});

test("an explicitly empty encoded flag list does not create an empty rustc argument", () => {
  expect(nativeSearchFlags("/lib", ["-lc++"], undefined, undefined, "")
    .split("\x1f")).toEqual(["-L", "native=/lib"]);
});
