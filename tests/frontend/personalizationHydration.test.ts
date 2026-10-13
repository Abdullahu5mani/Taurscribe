import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
const deferred = () => {
    let resolve!: () => void;
    const promise = new Promise<void>(r => { resolve = r; });
    return { resolve, promise };
};
let loadGate: Promise<void> | null = null;
let getGate: Promise<void> | null = null;
let setGate: Promise<void> | null = null;
let saved: Record<string, any> = {};
const store = {
    get: async (key: string) => { const value = saved[key] ?? null; if (getGate) await getGate; return value; },
    set: async (key: string, value: unknown) => { const gate = setGate; setGate = null; if (gate) await gate; saved[key] = value; },
    save: async () => {},
};
const realStore = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({ ...realStore, Store: { load: async () => { if (loadGate) await loadGate; return store; } } }));
const { usePersonalization } = await import("../../src/hooks/usePersonalization");
beforeEach(() => { loadGate = null; getGate = null; setGate = null; saved = {}; });

for (const phase of ["load", "get"] as const) {
    test(`edits during ${phase} hydrate over saved entries without losing either`, async () => {
        saved = { custom_dictionary: [{ id: "old", soundsLike: "old", correct: "Old" }], snippets: [], custom_vocabulary: ["Existing"], context_bias_enabled: true };
        const gate = deferred();
        if (phase === "load") loadGate = gate.promise; else getGate = gate.promise;
        const { handle, settle } = renderHook(() => usePersonalization());
        await settle();
        handle.current.addDictEntry({ soundsLike: "taur", correct: "Taur" });
        handle.current.addSnippet({ trigger: "sig", expansion: "Hello" });
        handle.current.addVocabWord("existing");
        handle.current.addVocabWord("New");
        handle.current.setContextBiasEnabled(false);
        await settle();
        const id = handle.current.dictionary[0].id;
        handle.current.updateDictEntry(id, { correct: "Taurscribe" });
        gate.resolve();
        await settle();
        expect(handle.current.dictionary.map(d => d.correct)).toEqual(["Old", "Taurscribe"]);
        expect(handle.current.snippets[0].expansion).toBe("Hello");
        expect(handle.current.customVocabulary).toEqual(["Existing", "New"]);
        expect(handle.current.contextBiasEnabled).toBe(false);
        expect(saved.custom_dictionary).toEqual(handle.current.dictionary);
        expect(saved.snippets).toEqual(handle.current.snippets);
        expect(saved.custom_vocabulary).toEqual(["Existing", "New"]);
        expect(saved.context_bias_enabled).toBe(false);
    });
}

test("clear before hydration stays cleared and later saves remain ordered", async () => {
    saved.custom_vocabulary = ["Old"];
    const gate = deferred(); getGate = gate.promise;
    const { handle, settle } = renderHook(() => usePersonalization());
    handle.current.clearVocab();
    gate.resolve(); await settle();
    expect(saved.custom_vocabulary).toEqual([]);
    const write = deferred(); setGate = write.promise;
    handle.current.addVocabWord("One"); await settle();
    handle.current.addVocabWord("Two"); await settle();
    write.resolve(); await settle();
    expect(saved.custom_vocabulary).toEqual(["One", "Two"]);
});
