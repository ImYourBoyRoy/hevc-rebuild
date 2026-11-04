// ./tools/interaction-waterfall-report.mjs
// Markdown renderer for state inventories and multiflow click waterfalls.

function escapeMarkdown(value) {
  return String(value ?? "").replace(/\|/gu, "\\|").replace(/\n/gu, " ");
}

function code(value) {
  return `\`${String(value ?? "").replaceAll("`", "'")}\``;
}

function sourceLink(source) {
  if (!source || !source.includes(":")) return code(source ?? "—");
  if (/:[0-9]+-[0-9]+(?:,|$)/u.test(source)) return code(source);
  const match = source.match(/^(.+?):(\d+)(?::\d+)?$/u);
  if (!match) return code(source);
  return `[${escapeMarkdown(source)}](../${match[1]}#L${match[2]})`;
}

function targetText(control) {
  if (!control?.targets?.length) return "—";
  return control.targets.map((target) => `${target.kind}:${target.value}${target.confidence === "static" ? "" : ` (${target.confidence})`}`).join("<br>");
}

function flattenEntries(entries) {
  return (entries ?? []).flatMap((entry) => entry.controls?.length ? entry.controls.map((control) => ({ entry, control })) : [{ entry, control: null }]);
}

function controlTable(entries, title = "Active controls") {
  const out = [`### ${title}`, "", "| Control | Kind | Source | Destination | Guards / notes |", "|---|---|---|---|---|"];
  for (const { entry, control } of flattenEntries(entries)) {
    const label = entry.label ?? control?.label ?? "(unlabeled)";
    const source = control ? sourceLink(control.source) : code(entry.source ?? "system/native");
    const kind = entry.kind ?? control?.kind ?? entry.type ?? "control";
    const guards = [
      ...(control?.guards ?? []),
      entry.conditional ? "conditional branch" : "",
      entry.note ?? "",
    ].filter(Boolean).join("; ") || "always";
    out.push(`| ${escapeMarkdown(label)} | ${escapeMarkdown(kind)} | ${source} | ${targetText(control)} | ${escapeMarkdown(guards)} |`);
  }
  if (out.length === 4) out.push("| — | — | — | — | No source controls in this state. |");
  return out;
}

function stateControlSection(state) {
  const out = [];
  out.push(`### ${state.title}`, "", `**State ID:** ${code(state.id)}`, `**Source condition:** ${escapeMarkdown(state.condition ?? "not specified")}`, "");
  out.push(`Active source templates: **${state.summary.activeTemplateControls}** · estimated runtime controls: **${state.summary.activeRuntimeControls}** · disabled source controls listed: **${state.summary.disabledTemplateControls}**.`, "");
  out.push(...controlTable(state.activeControls));
  if (state.conditionalControls.length) {
    out.push("", ...controlTable(state.conditionalControls, "Conditional / data-dependent controls"));
  }
  if (state.disabledControls.length) {
    out.push("", ...controlTable(state.disabledControls, "Visible but disabled during this state"));
  }
  if (state.systemControls.length) {
    out.push("", ...controlTable(state.systemControls, "Native/system actions"));
  }
  if (state.automatic?.length) {
    out.push("", "### Automatic transitions", "", "| Transition | Type | Next state | Source |", "|---|---|---|---|");
    for (const transition of state.automatic) out.push(`| ${escapeMarkdown(transition.label)} | ${escapeMarkdown(transition.type ?? "automatic")} | ${code(transition.next ?? state.id)} | ${sourceLink(transition.source)} |`);
  }
  if (state.notes?.length) out.push("", "Notes:", ...state.notes.map((note) => `- ${escapeMarkdown(note)}`));
  return out;
}

function costText(cost) {
  const parts = [];
  if (cost.app) parts.push(`${cost.app} app`);
  if (cost.native) parts.push(`${cost.native} native`);
  for (const [name, coefficient] of Object.entries(cost.variables ?? {})) parts.push(`${coefficient} ${name}`);
  return parts.length ? parts.join(" + ") : "0";
}

function renderFlow(flow) {
  const out = [];
  out.push(`### ${flow.title}`, "", `**Flow ID:** ${code(flow.id)} · **Start:** ${code(flow.start)} · **Finish:** ${code(flow.finish)}`, "");
  if (flow.assumptions?.length) out.push("Assumptions:", "", ...flow.assumptions.map((item) => `- ${escapeMarkdown(item)}`), "");
  out.push("| # | State transition | User/system action | Type | Click cost | Evidence |", "|---:|---|---|---|---:|---|");
  for (const step of flow.steps) {
    const transition = `${code(step.from)} → ${code(step.to)}`;
    const action = escapeMarkdown(step.label);
    const evidence = step.control ? sourceLink(step.control.source) : sourceLink(step.source);
    const detail = step.automatic ? `<br>${code(step.automatic)}` : "";
    out.push(`| ${step.number} | ${transition} | ${action}${detail} | ${escapeMarkdown(step.type)} | ${code(step.clickFormula)} | ${evidence} |`);
  }
  out.push("", `**Minimum formula:** ${code(flow.totals.clickFormula)} total clicks = ${flow.totals.appClicks} app + ${flow.totals.nativeClicks} native-picker/system + variable picker clicks.`, "", `Automatic/text-only steps: ${flow.totals.automaticSteps}. App clicks: ${flow.totals.appClicks}. Native/system clicks: ${flow.totals.nativeClicks}.`, "");
  return out;
}

export function renderWaterfallMarkdown(report) {
  const out = [];
  out.push(
    "# Multiflow UI waterfall map",
    "",
    "<!-- GENERATED by tools/interaction-waterfall.mjs — do not edit by hand. -->",
    "<!-- Edit docs/interaction-waterfalls.json for reviewed states and flows, then run: node tools/interaction-waterfall.mjs -->",
    "",
    "This is the task-oriented companion to the source interaction inventory. It models the application as a state graph: only controls declared for the current state are treated as active, each action points to its source and traced destination, and native/system steps are kept separate from app clicks.",
    "",
    "The cardinal rule is evaluated as a minimum click formula. `x` means native file-picker navigation and selection clicks; the native picker confirmation is counted separately. Waiting for boot, scanning, encoding, validation, report writing, and following progress are automatic steps with zero clicks.",
    "",
    "## Snapshot",
    "",
    "| Measure | Value |",
    "|---|---:|",
    `| Reviewed states | ${report.summary.states} |`,
    `| Reviewed flows | ${report.summary.flows} |`,
    `| Waterfall steps | ${report.summary.flowSteps} |`,
    `| Active source-template entries across states | ${report.summary.activeTemplateControls} |`,
    `| Map warnings | ${report.summary.warnings} |`,
    "",
    "## The three click buckets",
    "",
    "| Bucket | Counts | Example |",
    "|---|---|---|",
    "| App click | A button, checkbox, disclosure, or other in-app user action | Add folder, Check integrity, output confirmation |",
    "| Native/system click | OS picker confirmation or another native confirmation | Confirm selected folder in the file picker |",
    "| Variable native navigation | `x`, dependent on the user's path and OS picker layout | Navigate to `/media/Movies/` and select it |",
    "",
    "A click formula is therefore more honest than a single hard-coded number for the complete launch-to-output task. For example, metadata-only from a fresh empty library is `x + 4`: Add folder (1) + picker navigation (`x`) + picker confirmation (1) + Metadata-only MKV (1) + output confirmation (1).",
    "",
    "## State inventories",
    "",
  );
  for (const state of report.states) out.push(...stateControlSection(state), "");
  out.push("## Waterfall flows", "", "Each table is a shortest reviewed happy path under its assumptions. Collision, missing-tool, canceled-picker, recovery, and failed-run branches remain visible as state conditions but are not silently folded into the happy-path minimum.", "");
  for (const flow of report.flows) out.push(...renderFlow(flow), "");
  out.push("## Direct answers", "", "- Yes: the app-side portion is knowable from the Svelte source because the template declares the visible control, handler, state guard, and callback/native destination.", "- No: the exact number of clicks inside a native file picker is not knowable from Svelte. It is represented as `x`, with the picker confirmation counted as one native click.", "- Tauri is a boundary, not the reason the Svelte counts are inflated. Tauri adds native dialogs, IPC, window state, and OS behavior that a DOM-only census cannot turn into one runtime control.", "- The prior 271/327 figures are still useful as a source inventory, but they are not a task map. They include mutually exclusive pages, hidden branches, loop templates, persistence callbacks, and controls that are not simultaneously active.", "", "## Evidence and limits", "", "- Source evidence is resolved through the same Svelte AST/handler trace used by `interaction-map.mjs`.", "- State membership is reviewed configuration in `docs/interaction-waterfalls.json`; the tool does not pretend to evaluate arbitrary Svelte reactivity or reconstruct a live DOM snapshot.", "- `Each(...)` controls are template controls; runtime card/queue instances can be greater than one and are marked `runtime-variable` unless the reviewed state supplies a known instance count.", "- Native picker navigation, OS confirmation wording, drag/drop, provider responses, collision branches, and actual media output still require a real Tauri walkthrough.", "- A stale selector or a new unresolved branch fails the waterfall check instead of disappearing from the map.");
  if (report.warnings.length) out.push("", "## Map warnings", "", ...report.warnings.map((warning) => `- ${escapeMarkdown(warning)}`));
  return `${out.join("\n").replace(/\n{3,}/gu, "\n\n").trimEnd()}\n`;
}
