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
