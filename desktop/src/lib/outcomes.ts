// ./desktop/src/lib/outcomes.ts
// Report outcome classification and labels, extracted from the route.
// Pure: no IPC, no DOM, no state. Tested by pnpm test:outcomes.

export function isFailure(status: string | undefined) {
  const normalized = (status ?? "").trim().toUpperCase();
  return ["FAILED", "CORRUPT", "UNREADABLE", "ERROR"].includes(normalized) || normalized.includes("FAIL") || normalized.includes("ERROR") || normalized.includes("CORRUPT");
}

export function isSuccess(status: string | undefined) {
  return ["OK", "METADATA_OK", "TEST_OK", "INFO", "INTEGRITY_OK"].includes((status ?? "").trim().toUpperCase());
}

export function outcomeLabel(status: string | undefined) {
  const normalized = (status ?? "").trim().toUpperCase();
  return ({
    OK: "Succeeded",
    METADATA_OK: "Metadata copy succeeded",
    TEST_OK: "Short test succeeded",
    INFO: "Check finished",
    INTEGRITY_OK: "File is intact",
    CORRUPT: "File is damaged",
    UNREADABLE: "File could not be read",
    FAILED: "Failed",
    SKIPPED: "Skipped",
    INTERRUPTED: "Interrupted",
  } as Record<string, string>)[normalized] ?? (status || "Needs review");
}
