/**
 * Maps a linear RMS level (0..1) to a meter percentage on a decibel scale.
 *
 * Linear scaling makes normal call audio (RMS ~0.001-0.01) read as 0-1%, so the
 * meters looked dead while speech was being recorded. -60 dBFS reads 0% and
 * 0 dBFS reads 100%, which puts conversational speech around 30-60%.
 */
const FLOOR_DB = -60;

export function levelToPercent(rms: number): number {
  if (!(rms > 0)) return 0;
  const db = 20 * Math.log10(Math.min(rms, 1));
  return Math.max(0, Math.min(100, Math.round(((db - FLOOR_DB) / -FLOOR_DB) * 100)));
}
