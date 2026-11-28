import { describe, expect, test } from "vitest";

import { ANALYSIS_IDLE_GRACE_MS, advanceAnalysisWatch, startAnalysisWatch } from "@/lib/analysis-watch";

const OLD = "2026-11-28T11:18:49Z";
const NEW = "2026-11-28T11:36:02Z";

describe("advanceAnalysisWatch", () => {
  test("ends when a new report lands", () => {
    const watch = startAnalysisWatch(OLD);
    expect(advanceAnalysisWatch(watch, { generatedAt: NEW, running: false }, 0)).toBeNull();
    expect(advanceAnalysisWatch(startAnalysisWatch(null), { generatedAt: NEW, running: true }, 0)).toBeNull();
  });

  test("keeps following while the run is reported", () => {
    const watch = startAnalysisWatch(OLD);
    expect(advanceAnalysisWatch(watch, { generatedAt: OLD, running: true }, 0)).toBe(watch);
  });

  test("survives a pod that does not see the run yet", () => {
    let watch = advanceAnalysisWatch(startAnalysisWatch(OLD), { generatedAt: OLD, running: false }, 1_000);
    expect(watch).toEqual({ baseline: OLD, idleSince: 1_000 });
    watch = advanceAnalysisWatch(watch!, { generatedAt: OLD, running: true }, 3_000);
    expect(watch).toEqual({ baseline: OLD, idleSince: null });
    expect(advanceAnalysisWatch(watch!, { generatedAt: NEW, running: false }, 5_000)).toBeNull();
  });

  test("gives up when no run shows for the grace period", () => {
    const idle = advanceAnalysisWatch(startAnalysisWatch(OLD), { generatedAt: OLD, running: false }, 1_000)!;
    expect(advanceAnalysisWatch(idle, { generatedAt: OLD, running: false }, 1_000 + ANALYSIS_IDLE_GRACE_MS - 1)).toBe(idle);
    expect(advanceAnalysisWatch(idle, { generatedAt: OLD, running: false }, 1_000 + ANALYSIS_IDLE_GRACE_MS)).toBeNull();
  });
});
