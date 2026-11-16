// ./desktop/src/lib/state/run-ui-state.svelte.ts
// Run flow state: mode, pending batches, output preview, recovery review, per-file overrides, run-settings editor and the progress clock.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { RunMode, OutputPreview, MediaEntry, FileRunSettingsOverride } from "../types.js";
import type { RecoveryReviewItem } from "../ui-contracts.js";
import type { SourceRunBatch } from "../run-controller.ts";

export class RunUiState {
  mode = $state<RunMode>("check");
  integrityLevel = $state<"quick" | "full">("quick");
  stopOnError = $state(false);
  pendingRunBatches = $state<SourceRunBatch[]>([]);
  activeRunBatchIndex = $state(-1);
  pendingRunFullConfirmed = $state(false);
  outputPreview = $state<OutputPreview | null>(null);
  outputPreviewOpen = $state(false);
  outputPreviewBusy = $state(false);
  recoveryReviewOpen = $state(false);
  abortConfirmOpen = $state(false);
  recoveryReviewItems = $state<RecoveryReviewItem[]>([]);
  recoveryReviewIndex = $state(0);
  recoveryProbeEntry = $state<MediaEntry | null>(null);
  recoveryProbeError = $state("");
  recoveryProbeBusy = $state(false);
  recoveryProbeAttempted = $state(false);
  recoveryAcknowledged = $state(false);
  pendingRecoveryFullConfirmed = $state(false);
  recoveryReviewedPaths = $state<Set<string>>(new Set());
  fileTuneOverrides = $state<Record<string, "animation" | "none">>({});
  fileStreamOverrides = $state<Record<string, number[]>>({});
  fileSettingsOverrides = $state<Record<string, FileRunSettingsOverride>>({});
  runSettingsEditorOpen = $state(false);
  runSettingsEditorPaths = $state<string[]>([]);
  progressClock = $state(Date.now());
  runLaunchButton = $state<HTMLButtonElement | null>(null);
  fullAck = $state(false);
}
