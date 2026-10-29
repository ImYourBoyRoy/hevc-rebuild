import type { ReportRow } from './types.js';

export function metadataTextChanges(row: ReportRow): Array<{ scope?: string; field?: string; before?: string; after?: string; rules?: string[] }> {
    try {
      const parsed: unknown = JSON.parse(row.metadata_text_scrub_details ?? row.release_group_scrub_details ?? "[]");
      if (!Array.isArray(parsed)) return [];
      return parsed.filter((item): item is { scope?: string; field?: string; before?: string; after?: string; rules?: string[] } => {
        if (typeof item !== "object" || item === null) return false;
        const detail = item as Record<string, unknown>;
        return ["scope", "field", "before", "after"].every((key) => detail[key] === undefined || typeof detail[key] === "string") &&
          (detail.rules === undefined || (Array.isArray(detail.rules) && detail.rules.every((rule) => typeof rule === "string")));
      });
    } catch {
      return [];
    }
  }

export interface OptimizeSummary {
  label: string;
  detail?: string;
  verified?: string;
}

/**
 * Describe what YouTube Optimize decided for one report row, or null when the
 * row is not an Optimize row (Upscale rows and reports from older versions).
 */
export function optimizeSummary(row: ReportRow): OptimizeSummary | null {
  const action = row.optimize_action?.trim();
  if (!action) return null;
  const ssim = Number(row.quality_ssim_min);
  const hasSsim = row.quality_ssim_min !== undefined && row.quality_ssim_min !== "" && Number.isFinite(ssim);
  const label = action === "copy" ? "Video copied untouched" : action === "encode" ? "Re-encoded smaller" : `Optimize: ${action}`;
  const verified = action === "copy"
    ? (row.video_payload_identical === "True" ? "Packets verified identical" : undefined)
    : hasSsim ? `Sampled SSIM ${ssim.toFixed(4)}` : undefined;
  return { label, detail: row.optimize_reason || undefined, verified };
}
