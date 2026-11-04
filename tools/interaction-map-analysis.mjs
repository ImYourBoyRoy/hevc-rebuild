// ./tools/interaction-map-analysis.mjs
// Builds the source-derived control inventory, task budgets, and redundancy
// groups consumed by the interaction-map CLI and Markdown renderer.

import { createRequire } from "node:module";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { extname, join, relative, resolve, sep } from "node:path";
import {
  collectFunctions,
  collectMarkup,
  lineMap,
  compact,
} from "./interaction-map-ast.mjs";
import { enrichControls } from "./interaction-map-trace.mjs";

export function loadCompiler(root) {
  try {
    const require = createRequire(import.meta.url);
    const compilerPath = require.resolve("svelte/compiler", { paths: [join(root, "desktop")] });
    return require(compilerPath);
  } catch (error) {
    throw new Error(`Svelte compiler unavailable. Run 'pnpm --dir desktop install' first. ${String(error)}`);
  }
}

function listFiles(directory) {
  if (!existsSync(directory)) return [];
  const result = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) result.push(...listFiles(path));
    else if (entry.isFile() && extname(path) === ".svelte") result.push(path);
  }
  return result.sort();
}

function normalizeLabel(value) {
  return compact(value).toLowerCase().replace(/[{}'"`]/gu, "");
}

function selectorMatches(control, selector) {
  if (selector.file && control.file !== selector.file) return false;
  if (selector.line && control.line !== selector.line) return false;
  if (selector.kind && control.kind !== selector.kind) return false;
  if (selector.label && normalizeLabel(control.label) !== normalizeLabel(selector.label)) return false;
  if (selector.labelIncludes && !normalizeLabel(control.label).includes(normalizeLabel(selector.labelIncludes))) return false;
  if (selector.sourceIncludes && !control.source.includes(selector.sourceIncludes)) return false;
  if (selector.handler && !control.events.some((event) => event.expression.includes(selector.handler) || event.calls.some((call) => call.name === selector.handler))) return false;
  if (selector.event && !control.events.some((event) => event.name === selector.event)) return false;
  return true;
}

function resolveTaskSteps(tasks, controls) {
  const warnings = [];
  const byPath = new Map(controls.map((control) => [control.id, control]));
  const resolveSelector = (selector) => {
    if (selector.id) return byPath.get(selector.id) ? [byPath.get(selector.id)] : [];
    const matches = controls.filter((control) => selectorMatches(control, selector));
    const occurrence = selector.occurrence ?? 1;
    return matches[occurrence - 1] ? [matches[occurrence - 1]] : [];
  };
  const normalizedTasks = (tasks.tasks ?? []).map((task) => {
    const paths = (task.paths ?? []).map((path) => {
      const steps = [];
      for (const selector of path.steps ?? []) {
        const matches = resolveSelector(selector);
        if (!matches.length) {
          warnings.push(`${task.id}/${path.id}: selector matched nothing: ${JSON.stringify(selector)}`);
          steps.push({ selector, unresolved: true, clicks: 0 });
          continue;
        }
        const control = matches[0];
        const clicks = Number.isFinite(selector.clicks) ? Math.max(0, selector.clicks) : control.clickable ? 1 : 0;
        steps.push({ selector, controlId: control.id, label: control.label, file: control.file, line: control.line, kind: control.kind, clicks, targets: control.targets });
      }
      const clicks = steps.reduce((sum, step) => sum + step.clicks, 0);
      return { ...path, steps, clicks, unresolved: steps.some((step) => step.unresolved) };
    });
    const complete = paths.filter((path) => !path.unresolved);
    const minimumClicks = complete.length ? Math.min(...complete.map((path) => path.clicks)) : null;
    return {
      ...task,
      paths: paths.map((path) => ({ ...path, excessClicks: minimumClicks == null ? null : path.clicks - minimumClicks })),
      minimumClicks,
      warnings: paths.filter((path) => path.unresolved).map((path) => path.id),
    };
  });
  return { tasks: normalizedTasks, warnings };
}

const userFacingNativeCommands = new Set([
  "apply_library_rename",
  "check_ubuntu_package_updates",
  "install_hdr10plus",
  "open_file_location",
  "preview_library_rename",
  "preview_queue_output_targets",
  "refresh_ubuntu_package_catalog",
  "request_stop",
  "scan_folder",
  "start_run",
]);

function isUserFacingDestination(destination) {
  const separator = destination.indexOf(":");
  const kind = separator >= 0 ? destination.slice(0, separator) : destination;
  const value = separator >= 0 ? destination.slice(separator + 1) : "";
  if (kind === "native-command") return userFacingNativeCommands.has(value);
  if (kind === "callback") return !/^(onLookup|onMatch|onOrganizer|patch|persist|queue|schedule)/u.test(value);
  return true;
}

export function analyseInteractionMap({ root, sourceRoot, tasksPath, parse }) {
  const paths = listFiles(sourceRoot).map((path) => relative(root, path).split(sep).join("/"));
  const analyses = new Map();
  for (const path of paths) {
    const absolute = resolve(root, path);
    const source = readFileSync(absolute, "utf8");
    const ast = parse(source);
    analyses.set(path, { path, absolute, source, ast, lines: lineMap(source), functions: collectFunctions(ast), controls: [], bindings: [] });
  }
  for (const analysis of analyses.values()) collectMarkup(analysis, paths, sourceRoot, root);

  const bindingMap = new Map();
  for (const analysis of analyses.values()) {
    for (const binding of analysis.bindings) {
      const key = `${binding.target}::${binding.prop}`;
      if (!bindingMap.has(key)) bindingMap.set(key, []);
      bindingMap.get(key).push(binding);
    }
  }
  enrichControls(analyses, bindingMap);
  const controls = [...analyses.values()].flatMap((analysis) => analysis.controls).sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column);
  const tasks = existsSync(tasksPath) ? JSON.parse(readFileSync(tasksPath, "utf8")) : { tasks: [] };
  const resolvedTasks = resolveTaskSteps(tasks, controls);

  const destinationGroups = new Map();
  for (const control of controls) for (const target of control.targets) {
    const key = `${target.kind}:${target.value}`;
    if (!destinationGroups.has(key)) destinationGroups.set(key, []);
    destinationGroups.get(key).push(control.id);
  }
  const duplicateDestinations = [...destinationGroups.entries()]
    .filter(([, ids]) => ids.length > 1)
    .map(([destination, ids]) => ({ destination, controls: ids }));
  const duplicateUserFacingDestinations = duplicateDestinations.filter((group) => isUserFacingDestination(group.destination));

  const labelGroups = new Map();
  for (const control of controls) {
    const key = normalizeLabel(control.label);
    if (!labelGroups.has(key)) labelGroups.set(key, []);
    labelGroups.get(key).push(control.id);
  }
  const duplicateLabels = [...labelGroups.entries()]
    .filter(([, ids]) => ids.length > 1)
    .map(([label, ids]) => ({ label, controls: ids }));

  const summary = {
    files: analyses.size,
    controls: controls.length,
    buttons: controls.filter((control) => control.kind === "button").length,
    clickable: controls.filter((control) => control.clickable).length,
    links: controls.filter((control) => control.kind === "link").length,
    formControls: controls.filter((control) => control.kind === "form-control").length,
    disclosures: controls.filter((control) => control.kind === "disclosure").length,
    clickEvents: controls.reduce((sum, control) => sum + control.events.filter((event) => event.name === "click").length, 0),
    duplicateDestinations: duplicateDestinations.length,
    duplicateUserFacingDestinations: duplicateUserFacingDestinations.length,
    duplicateLabels: duplicateLabels.length,
  };
  return { generatedBy: "tools/interaction-map.mjs", sourceRoot: relative(root, sourceRoot), tasksFile: relative(root, tasksPath), summary, controls, tasks: resolvedTasks.tasks, warnings: resolvedTasks.warnings, duplicateDestinations, duplicateUserFacingDestinations, duplicateLabels };
}
