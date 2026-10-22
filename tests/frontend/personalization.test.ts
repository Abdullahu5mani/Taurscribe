import { describe, expect, test } from "bun:test";
import { applyDictionary, applySnippets, genId } from "../../src/hooks/usePersonalization";

const dict = (soundsLike: string, correct: string) => ({ id: soundsLike, soundsLike, correct });
const snip = (trigger: string, expansion: string) => ({ id: trigger, trigger, expansion });

describe("applyDictionary", () => {
  test("replaces whole words case-insensitively", () => {
    expect(applyDictionary("ask tori about Tori's build", [dict("tori", "Tauri")])).toBe(
      "ask Tauri about Tauri's build",
    );
  });

  test("leaves words that only contain the term alone", () => {
    expect(applyDictionary("history and story", [dict("story", "Story")])).toBe("history and Story");
  });

  test("matches terms with accented letters", () => {
    expect(applyDictionary("meet at the cafe or café", [dict("café", "Café Luna")])).toBe(
      "meet at the cafe or Café Luna",
    );
  });

  test("does not match accented words that merely end with the term", () => {
    expect(applyDictionary("résumé sumé", [dict("sumé", "X")])).toBe("résumé X");
  });

  test("inserts dollar signs literally", () => {
    expect(applyDictionary("the price", [dict("price", "$& $$5")])).toBe("the $& $$5");
  });

  test("escapes regex metacharacters in the term", () => {
    expect(applyDictionary("use c++ and node.js", [dict("c++", "C++"), dict("node.js", "Node.js")])).toBe(
      "use C++ and Node.js",
    );
    expect(applyDictionary("nodeXjs", [dict("node.js", "Node.js")])).toBe("nodeXjs");
  });

  test("skips blank entries and replaces each spoken word once", () => {
    // "a" becomes "b"; that "b" is not rewritten again by the b→c entry.
    expect(applyDictionary("a b", [dict("  ", "x"), dict("a", ""), dict("a", "b"), dict("b", "c")])).toBe("b c");
    expect(applyDictionary("same", [])).toBe("same");
  });
});

describe("applySnippets", () => {
  test("expands triggers that start with punctuation", () => {
    expect(applySnippets("thanks /sig", [snip("/sig", "Best, Sam")])).toBe("thanks Best, Sam");
    expect(applySnippets("email @@me now", [snip("@@me", "sam@example.com")])).toBe("email sam@example.com now");
  });

  test("does not expand a trigger inside another word", () => {
    expect(applySnippets("brb and brbx", [snip("brb", "be right back")])).toBe("be right back and brbx");
  });

  test("keeps multi-line expansions and dollar signs intact", () => {
    expect(applySnippets("addr", [snip("addr", "1 Main St\nCost: $1")])).toBe("1 Main St\nCost: $1");
  });

  test("uses the trimmed trigger", () => {
    expect(applySnippets("say hi", [snip(" hi ", "hello")])).toBe("say hello");
  });
});

test("genId returns unique ids", () => {
  const ids = new Set(Array.from({ length: 100 }, () => genId()));
  expect(ids.size).toBe(100);
});

test("a replacement is never rewritten by a later entry", () => {
    // Each spoken word is replaced once, from the original text.
    expect(applyDictionary("call jon and john", [
        { id: "1", soundsLike: "jon", correct: "John" },
        { id: "2", soundsLike: "john", correct: "Jonathan" },
    ] as any)).toBe("call John and Jonathan");
    // An expansion that contains another trigger is not expanded again.
    expect(applySnippets("sig", [
        { id: "1", trigger: "sig", expansion: "Best, addr" },
        { id: "2", trigger: "addr", expansion: "1 Main St" },
    ] as any)).toBe("Best, addr");
    // Where terms overlap, the longer one wins regardless of entry order.
    expect(applyDictionary("visit new york soon", [
        { id: "1", soundsLike: "new", correct: "New" },
        { id: "2", soundsLike: "new york", correct: "New York City" },
    ] as any)).toBe("visit New York City soon");
});
