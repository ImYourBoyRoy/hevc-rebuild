// ./desktop/src/lib/state/tool-setup-state.svelte.ts
// Environment inspection result and the package-repair job, logs and tool outcomes.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { Environment, AptUpdateStatus, CommandCheck, ProvisionPlan, ToolPaths } from "../types.js";
import type { ToolSetupLogSummary, RepairToolOutcome } from "../ui-contracts.js";

export class ToolSetupState {
  environment = $state<Environment | null>(null);
  environmentLoading = $state(false);
  environmentError = $state("");
  setupConfirmOpen = $state(false);
  setupPlan = $state<ProvisionPlan | null>(null);
  setupUpdates = $state<ProvisionPlan | null>(null);
  aptUpdates = $state<AptUpdateStatus | null>(null);
  aptChecking = $state(false);
  repairId = $state("");
  activeToolSetupLogId = $state("");
  packageJobKind = $state<"provision" | null>(null);
  repairLines = $state<string[]>([]);
  repairSummary = $state("");
  repairSummaryIsError = $state(false);
  repairLogWarning = $state("");
  toolSetupLogs = $state<ToolSetupLogSummary[]>([]);
  selectedToolSetupLog = $state("");
  viewedToolSetupLog = $state("");
  toolSetupLogLoading = $state(false);
  toolSetupLogError = $state("");
  toolSetupLogExporting = $state(false);
  repairToolChecksBefore = $state<CommandCheck[] | null>(null);
  repairAffectedToolKeys = $state<string[]>([]);
  repairToolOutcomes = $state<RepairToolOutcome[]>([]);
  setNotice: (message: string, kind?: "info" | "success" | "error") => void = () => {};
  currentToolPaths: () => ToolPaths = () => ({ ffmpeg: null, ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null });
  declare readonly aptHost: boolean;
  declare readonly autoUpdateTools: boolean;
  declare readonly workflowBusy: boolean;
  declare readonly scanning: boolean;
  declare readonly runId: string;
  declare readonly runStarting: boolean;
}
