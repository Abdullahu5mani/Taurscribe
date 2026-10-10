export interface RecordingSource {
  path: string;
  name: string;
  duration_ms: number;
  recorded_start: string | null;
  recorded_end: string | null;
  timestamp_source: string | null;
  file_created_at: string | null;
  file_modified_at: string | null;
  offset_ms: number;
}

function timestamp(value: string | null): number | null {
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isFinite(parsed) ? parsed : null;
}

/** Sort dated recordings within their slots; undated files keep their positions. */
export function chronologicalRecordings(sources: RecordingSource[]): RecordingSource[] {
  const dated = sources.filter(s => timestamp(s.recorded_start) !== null)
    .sort((a, b) => timestamp(a.recorded_start)! - timestamp(b.recorded_start)!);
  let next = 0;
  return sources.map(source => timestamp(source.recorded_start) === null ? source : dated[next++]);
}

export function recordingWarnings(sources: RecordingSource[]): string[] {
  const warnings: string[] = [];
  const dated = sources.filter(source => timestamp(source.recorded_start) !== null);
  for (let i = 1; i < dated.length; i++) {
    const previous = dated[i - 1], current = dated[i];
    const start = timestamp(current.recorded_start), end = timestamp(previous.recorded_end);
    const previousStart = timestamp(previous.recorded_start);
    if (start === null || end === null) continue;
    if (previousStart !== null && start < previousStart) {
      warnings.push(`${current.name} was recorded before ${previous.name}. Your chosen order will be used.`);
    } else if (start - end > 30 * 60_000) {
      warnings.push(`${previous.name} and ${current.name} are ${Math.round((start - end) / 60_000)} minutes apart and may be different meetings. You can still import them together.`);
    } else if (end - start > 1000) {
      warnings.push(`${previous.name} and ${current.name} overlap by ${Math.ceil((end - start) / 1000)} seconds. Both recordings will be kept in full.`);
    }
  }
  return warnings;
}

export function moveRecording(sources: RecordingSource[], index: number, direction: -1 | 1): RecordingSource[] {
  const target = index + direction;
  if (index < 0 || index >= sources.length || target < 0 || target >= sources.length) return sources;
  const next = [...sources];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

/** Only embedded recording times can suggest that adjacent files belong together. */
export function nearbyRecordingGroups(sources: RecordingSource[]): RecordingSource[][] {
  const groups: RecordingSource[][] = [];
  let group: RecordingSource[] = [];
  for (const source of sources) {
    const previous = group.at(-1);
    const start = timestamp(source.recorded_start);
    const end = previous ? timestamp(previous.recorded_end) : null;
    const previousStart = previous ? timestamp(previous.recorded_start) : null;
    if (previous && start !== null && end !== null && previousStart !== null && start >= previousStart && Math.abs(start - end) <= 5 * 60_000) {
      group.push(source);
    } else {
      if (group.length > 1) groups.push(group);
      group = [source];
    }
  }
  if (group.length > 1) groups.push(group);
  return groups;
}
export const recordingGroupKey = (group: RecordingSource[]) => JSON.stringify(group.map(source => source.path));

export function planRecordingImports(sources: RecordingSource[], combineAll: boolean, approvedGroups: Set<string>): RecordingSource[][] {
  if (combineAll) return [sources];
  const approved = nearbyRecordingGroups(sources).filter(group => approvedGroups.has(recordingGroupKey(group)));
  const starts = new Map(approved.map(group => [group[0].path, group]));
  const consumed = new Set(approved.flatMap(group => group.slice(1).map(source => source.path)));
  return sources.filter(source => !consumed.has(source.path)).map(source => starts.get(source.path) ?? [source]);
}
