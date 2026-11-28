export type AnalysisWatch = {
  /** `generated_at` of the report on screen when the run was asked for. */
  baseline: string | null;
  /** When the server first answered "not running" without a new report. */
  idleSince: number | null;
};

export type AnalysisSnapshot = {
  generatedAt: string | null;
  running: boolean;
};

/**
 * How long "not running" has to hold before a requested run counts as over
 * without a new report. Another pod can answer for a moment before it sees the
 * run, and a failed run never produces a report.
 */
export const ANALYSIS_IDLE_GRACE_MS = 10_000;

export function startAnalysisWatch(generatedAt: string | null): AnalysisWatch {
  return { baseline: generatedAt, idleSince: null };
}

/** Returns the watch to keep following the run with, or null once it is over. */
export function advanceAnalysisWatch(watch: AnalysisWatch, seen: AnalysisSnapshot, now: number): AnalysisWatch | null {
  if (seen.generatedAt !== watch.baseline) return null;
  if (seen.running) return watch.idleSince === null ? watch : { ...watch, idleSince: null };
  if (watch.idleSince === null) return { ...watch, idleSince: now };
  return now - watch.idleSince >= ANALYSIS_IDLE_GRACE_MS ? null : watch;
}
