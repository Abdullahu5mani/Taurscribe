import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

// Speaker Vault: delete needs a confirming second click, rename / delete /
// sample-cycling failures are shown, and cycling updates the shown clip.
let vault: any[];
let failures: Record<string, string>;
const calls: { cmd: string; args: any }[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    convertFileSrc: (p: string) => p,
    invoke: async (cmd: string, args: any) => {
        calls.push({ cmd, args });
        if (failures[cmd]) throw failures[cmd];
        switch (cmd) {
            case "get_download_status": return [{ id: "speaker-ecapa", downloaded: true }, ...args.modelIds.map((id: string) => ({ id, downloaded: true }))];
            case "list_speaker_vault": return vault;
            case "delete_speaker_from_vault": vault = vault.filter((s) => s.id !== args.speakerId); return null;
            case "rename_vault_speaker": vault = vault.map((s) => s.id === args.speakerId ? { ...s, name: args.newName } : s); return null;
            case "cycle_vault_speaker_snippet": return ["/clips/alice-2.wav", 1];
            default: return null;
        }
    },
}));
const { SpeakerVaultModal } = await import("../../src/components/SpeakerVaultModal");

let updates: number;
beforeEach(() => {
    calls.length = 0;
    failures = {};
    updates = 0;
    vault = [
        { id: "p1", name: "Alice", meeting_count: 3, last_seen: "2026-10-01T00:00:00Z", snippet_path: "/clips/alice-1.wav", candidate_snippets: ["/clips/alice-1.wav", "/clips/alice-2.wav"] },
        { id: "p2", name: "Sam", meeting_count: 1, last_seen: "2026-10-02T00:00:00Z", snippet_path: null, candidate_snippets: [] },
    ];
});
const open = async () => {
    const view = renderHook(() => SpeakerVaultModal({ isOpen: true, onClose: () => {}, onSpeakerUpdated: () => updates++ }));
    await view.settle();
    return view;
};
const deletes = () => calls.filter((c) => c.cmd === "delete_speaker_from_vault");

test("Delete asks first; Cancel keeps the person", async () => {
    const { handle, settle } = await open();
    await byTest(handle.current, "speaker-vault-delete-p1").props.onClick();
    await settle();
    expect(deletes()).toHaveLength(0);
    expect(flat(byTest(handle.current, "speaker-vault-delete-confirm-p1"))).toBe("Delete Alice?");
    byTest(handle.current, "speaker-vault-delete-cancel-p1").props.onClick();
    await settle();
    expect(byTest(handle.current, "speaker-vault-delete-confirm-p1")).toBeUndefined();
    expect(deletes()).toHaveLength(0);
});

test("confirming deletes that person only and refreshes the list", async () => {
    const { handle, settle } = await open();
    await byTest(handle.current, "speaker-vault-delete-p1").props.onClick();
    await settle();
    await byTest(handle.current, "speaker-vault-delete-confirm-p1").props.onClick();
    await settle();
    expect(deletes().map((c) => c.args)).toEqual([{ speakerId: "p1" }]);
    expect(textOf(handle.current)).not.toContain("Alice");
    expect(textOf(handle.current)).toContain("Sam");
    expect(updates).toBe(1);
});

test("confirming one person's delete never deletes another", async () => {
    const { handle, settle } = await open();
    await byTest(handle.current, "speaker-vault-delete-p1").props.onClick();
    await settle();
    await byTest(handle.current, "speaker-vault-delete-p2").props.onClick(); // switches the question to Sam
    await settle();
    expect(byTest(handle.current, "speaker-vault-delete-confirm-p1")).toBeUndefined();
    expect(byTest(handle.current, "speaker-vault-delete-confirm-p2")).toBeDefined();
    expect(deletes()).toHaveLength(0);
});

test("a failed delete is shown and the person stays", async () => {
    failures.delete_speaker_from_vault = "database is locked";
    const { handle, settle } = await open();
    await byTest(handle.current, "speaker-vault-delete-p1").props.onClick();
    await settle();
    await byTest(handle.current, "speaker-vault-delete-confirm-p1").props.onClick();
    await settle();
    expect(textOf(byTest(handle.current, "speaker-vault-error"))).toBe("Couldn't delete Alice: database is locked");
    expect(textOf(handle.current)).toContain("Alice");
});

test("rename saves the trimmed name; a failure is shown", async () => {
    const { handle, settle } = await open();
    byTest(handle.current, "speaker-vault-rename-p1").props.onClick();
    await settle();
    byTest(handle.current, "speaker-vault-rename-input-p1").props.onChange({ target: { value: "  Alicia  " } });
    await settle();
    await byTest(handle.current, "speaker-vault-save-name-p1").props.onClick();
    await settle();
    expect(calls.find((c) => c.cmd === "rename_vault_speaker")!.args).toEqual({ speakerId: "p1", newName: "Alicia" });
    expect(textOf(handle.current)).toContain("Alicia");

    failures.rename_vault_speaker = "name taken";
    byTest(handle.current, "speaker-vault-rename-p1").props.onClick();
    await settle();
    byTest(handle.current, "speaker-vault-rename-input-p1").props.onChange({ target: { value: "Bob" } });
    await settle();
    await byTest(handle.current, "speaker-vault-save-name-p1").props.onClick();
    await settle();
    expect(textOf(byTest(handle.current, "speaker-vault-error"))).toBe("Couldn't rename Alicia: name taken");
});

test("cycling the sample shows the next clip; running out of clips is explained", async () => {
    const { handle, settle } = await open();
    expect(flat(byTest(handle.current, "speaker-vault-cycle-sample-p1"))).toBe("Sample 1/2");
    await byTest(handle.current, "speaker-vault-cycle-sample-p1").props.onClick();
    await settle();
    expect(flat(byTest(handle.current, "speaker-vault-cycle-sample-p1"))).toBe("Sample 2/2");

    failures.cycle_vault_speaker_snippet = "No other voice samples are available for this person";
    await byTest(handle.current, "speaker-vault-cycle-sample-p1").props.onClick();
    await settle();
    expect(textOf(byTest(handle.current, "speaker-vault-error"))).toContain("No other voice samples");
});
