import type { DiarizedTurn } from "../types/meetings";

/** Display grouping only: database turns stay separate for exact merge undo. */
export function conversationBlocks(turns: DiarizedTurn[]) {
  const blocks: (DiarizedTurn & { sourceIndices: number[]; intervals: { start_ms: number; end_ms: number }[] })[] = [];
  for (const [index, turn] of turns.entries()) {
    const last = blocks.at(-1);
    if (last && last.speaker_id === turn.speaker_id && last.channel === turn.channel && turn.start_ms >= last.start_ms && turn.start_ms <= last.end_ms + 1000) {
      last.end_ms = Math.max(last.end_ms, turn.end_ms);
      last.text = [last.text, turn.text].filter(Boolean).join(" ");
      last.sourceIndices.push(index);
      last.intervals.push({ start_ms: turn.start_ms, end_ms: turn.end_ms });
    } else blocks.push({ ...turn, sourceIndices: [index], intervals: [{ start_ms: turn.start_ms, end_ms: turn.end_ms }] });
  }
  return blocks;
}
export function spokenAt(intervals: { start_ms: number; end_ms: number }[], seconds: number) {
  const ms = seconds * 1000;
  return intervals.some(t => ms >= t.start_ms && ms < t.end_ms);
}

export function speakerCountHint(value: string): number | null {
  if (!value.trim()) return null;
  const count = Number(value);
  if (!Number.isInteger(count) || count < 1 || count > 32) throw new Error("Choose about 1–32 people, or leave the hint blank.");
  return count;
}
