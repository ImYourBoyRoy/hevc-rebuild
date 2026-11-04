// ./tools/interaction-waterfall-analysis.mjs
// Resolves reviewed UI states and task waterfalls against the shared Svelte
// interaction inventory. The JSON configuration describes user-visible state;
// this module supplies source locations, destinations, costs, and warnings.

import { existsSync, readFileSync } from "node:fs";
import { relative, sep } from "node:path";
import { analyseInteractionMap } from "./interaction-map-analysis.mjs";

function normalize(value) {
  return String(value ?? "").replace(/\s+/gu, " ").trim().toLowerCase();
}

function selectorMatches(control, selector = {}) {
  if (selector.id && control.id !== selector.id) return false;
  if (selector.file && control.file !== selector.file) return false;
  if (selector.line != null && control.line !== selector.line) return false;
  if (selector.column != null && control.column !== selector.column) return false;
  if (selector.kind && control.kind !== selector.kind) return false;
  if (selector.label && normalize(control.label) !== normalize(selector.label)) return false;
  if (selector.labelIncludes && !normalize(control.label).includes(normalize(selector.labelIncludes))) return false;
  if (selector.sourceIncludes && !control.source.includes(selector.sourceIncludes)) return false;
  if (selector.handler && !control.events.some((event) => event.expression.includes(selector.handler) || event.calls.some((call) => call.name === selector.handler))) return false;
  if (selector.event && !control.events.some((event) => event.name === selector.event)) return false;
  if (selector.guardIncludes && !control.guards.some((guard) => guard.includes(selector.guardIncludes))) return false;
  if (selector.guardExcludes && control.guards.some((guard) => guard.includes(selector.guardExcludes))) return false;
  return true;
}

function resolveSelector(selector, controls) {
  const matches = controls.filter((control) => selectorMatches(control, selector));
  if (selector.all) return matches;
  const occurrence = selector.occurrence ?? 1;
  return matches[occurrence - 1] ? [matches[occurrence - 1]] : [];
}

function sourceRef(control) {
  return control ? `${control.file}:${control.line}:${control.column}` : null;
}

function trimControl(control, entry = {}) {
  if (!control) return null;
  return {
    id: control.id,
    file: control.file,
    line: control.line,
    column: control.column,
    source: sourceRef(control),
    kind: entry.kind ?? control.kind,
    label: entry.label ?? control.label,
    guards: control.guards,
    targets: control.targets,
    trace: control.trace,
    instances: entry.instances ?? (control.guards.some((guard) => guard.startsWith("Each(")) ? "runtime-variable" : 1),
    conditional: Boolean(entry.conditional),
    note: entry.note ?? null,
  };
}

function expandEntries(item, groups) {
  if (!item.group) return [item];
  const entries = groups[item.group] ?? [];
  const filtered = item.include
    ? entries.filter((entry) => item.include.includes(entry.id))
    : entries.filter((entry) => !(item.exclude ?? []).includes(entry.id));
  return filtered.map((entry) => ({ ...entry, ...item, group: undefined, include: undefined, exclude: undefined }));
}

function resolveEntries(items, controls, groups, warnings, context) {
  const resolved = [];
  for (const item of items ?? []) {
    for (const entry of expandEntries(item, groups)) {
      if (!entry.selector) {
        resolved.push({ ...entry, control: null, controls: [] });
        continue;
      }
      const matches = resolveSelector({ ...entry.selector, all: Boolean(entry.all || entry.selector.all) }, controls);
      if (!matches.length) {
        warnings.push(`${context}: selector matched nothing: ${JSON.stringify(entry.selector)}`);
        resolved.push({ ...entry, control: null, controls: [] });
        continue;
      }
      const trimmed = matches.map((control) => trimControl(control, entry));
      resolved.push({ ...entry, control: trimmed[0], controls: trimmed });
    }
  }
  return resolved;
}

function emptyCost() {
  return { app: 0, native: 0, variables: {} };
}

function normalizeCost(cost = {}) {
  const result = emptyCost();
  result.app = Number.isFinite(cost.app) ? cost.app : 0;
  result.native = Number.isFinite(cost.native) ? cost.native : 0;
  for (const [name, coefficient] of Object.entries(cost.variables ?? {})) {
    if (Number.isFinite(coefficient) && coefficient !== 0) result.variables[name] = coefficient;
  }
  return result;
}

function addCost(total, cost) {
  total.app += cost.app;
  total.native += cost.native;
  for (const [name, coefficient] of Object.entries(cost.variables)) total.variables[name] = (total.variables[name] ?? 0) + coefficient;
  return total;
}

export function formatFormula(cost, variables) {
  const variableTerms = Object.entries(cost.variables).filter(([, coefficient]) => coefficient !== 0).map(([name, coefficient]) => {
    const symbol = variables[name]?.symbol ?? name;
    return coefficient === 1 ? symbol : `${coefficient}${symbol}`;
  });
  const constant = cost.app + cost.native;
  if (constant) variableTerms.push(String(constant));
  return variableTerms.length ? variableTerms.join(" + ") : "0";
}

function resolveState(state, controls, groups, warnings) {
  const active = resolveEntries(state.activeControls, controls, groups, warnings, `${state.id}/activeControls`);
  const activeIds = new Set(active.flatMap((entry) => entry.controls.map((control) => control.id)));
  const conditional = resolveEntries(state.conditionalControls, controls, groups, warnings, `${state.id}/conditionalControls`)
    .map((entry) => ({ ...entry, controls: entry.controls.filter((control) => !activeIds.has(control.id)) }))
    .filter((entry) => entry.controls.length || !entry.selector);
  const disabled = resolveEntries(state.disabledControls, controls, groups, warnings, `${state.id}/disabledControls`);
  const system = (state.systemControls ?? []).map((entry) => ({ ...entry, cost: normalizeCost(entry.cost) }));
  return {
    ...state,
    activeControls: active,
    conditionalControls: conditional,
    disabledControls: disabled,
    systemControls: system,
    summary: {
      activeTemplateControls: active.reduce((sum, entry) => sum + (entry.controls.length || 1), 0),
      activeRuntimeControls: active.reduce((sum, entry) => sum + (typeof entry.instances === "number" ? entry.instances : entry.controls.length || 1), 0),
      conditionalTemplateControls: conditional.reduce((sum, entry) => sum + entry.controls.length, 0),
      disabledTemplateControls: disabled.reduce((sum, entry) => sum + entry.controls.length, 0),
    },
  };
}

function resolveFlow(flow, actions, controls, states, warnings, variables) {
  let current = flow.start;
  const total = emptyCost();
  const steps = [];
  for (const [index, rawStep] of (flow.steps ?? []).entries()) {
    const definition = rawStep.action ? actions[rawStep.action] : rawStep;
    if (rawStep.action && !definition) {
      warnings.push(`${flow.id}/step-${index + 1}: unknown action ${rawStep.action}`);
      continue;
    }
    const from = definition.state ?? rawStep.from ?? current;
    if (from !== current) warnings.push(`${flow.id}/step-${index + 1}: expected state ${from}, but waterfall is in ${current}`);
    const cost = normalizeCost(definition.cost ?? rawStep.cost);
    addCost(total, cost);
    const matches = definition.selector ? resolveSelector(definition.selector, controls) : [];
    if (definition.selector && !matches.length) warnings.push(`${flow.id}/step-${index + 1}: action selector matched nothing: ${JSON.stringify(definition.selector)}`);
    steps.push({
      number: index + 1,
      action: rawStep.action ?? null,
      label: definition.label ?? rawStep.label ?? rawStep.action ?? `Step ${index + 1}`,
      type: definition.type ?? rawStep.type ?? "automatic",
      from,
      to: definition.next ?? rawStep.to ?? current,
      cost,
      clickFormula: formatFormula(cost, variables),
      source: matches[0] ? sourceRef(matches[0]) : definition.source ?? rawStep.source ?? null,
      control: matches[0] ? trimControl(matches[0], definition) : null,
      automatic: definition.automatic ?? rawStep.automatic ?? null,
      note: definition.note ?? rawStep.note ?? null,
    });
    current = definition.next ?? rawStep.to ?? current;
  }
  if (current !== flow.finish) warnings.push(`${flow.id}: ended in ${current}; expected ${flow.finish}`);
  return {
    ...flow,
    steps,
    totals: {
      ...total,
      clickFormula: formatFormula(total, variables),
      appClicks: total.app,
      nativeClicks: total.native,
      automaticSteps: steps.filter((step) => step.type === "automatic" || step.type === "text-entry").length,
      variableClicks: Object.fromEntries(Object.entries(total.variables)),
    },
  };
}

export function analyseInteractionWaterfalls({ root, sourceRoot, configPath, tasksPath, parse }) {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const base = analyseInteractionMap({ root, sourceRoot, tasksPath, parse });
  const warnings = [...base.warnings];
  const states = (config.states ?? []).map((state) => resolveState(state, base.controls, config.groups ?? {}, warnings));
  const stateMap = new Map(states.map((state) => [state.id, state]));
  for (const flow of config.flows ?? []) {
    if (!stateMap.has(flow.start)) warnings.push(`${flow.id}: unknown start state ${flow.start}`);
    if (!stateMap.has(flow.finish)) warnings.push(`${flow.id}: unknown finish state ${flow.finish}`);
  }
  const flows = (config.flows ?? []).map((flow) => resolveFlow(flow, config.actions ?? {}, base.controls, stateMap, warnings, config.variables ?? {}));
  return {
    generatedBy: "tools/interaction-waterfall.mjs",
    sourceRoot: relative(root, sourceRoot).split(sep).join("/"),
    configFile: relative(root, configPath).split(sep).join("/"),
    variables: config.variables ?? {},
    states,
    flows,
    warnings: [...new Set(warnings)],
    summary: {
      states: states.length,
      flows: flows.length,
      flowSteps: flows.reduce((sum, flow) => sum + flow.steps.length, 0),
      activeTemplateControls: states.reduce((sum, state) => sum + state.summary.activeTemplateControls, 0),
      warnings: [...new Set(warnings)].length,
    },
  };
}

export function loadWaterfallConfig(path) {
  if (!existsSync(path)) throw new Error(`Waterfall config not found: ${path}`);
  return JSON.parse(readFileSync(path, "utf8"));
}
