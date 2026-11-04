#!/usr/bin/env node
// ./tools/interaction-map.mjs
// Builds a source-derived interaction inventory and click-budget report for
// the Svelte desktop UI. Native dialogs and conditional state remain marked as
// unverified until a real Tauri walkthrough runs.
//
// Run from the repository root:
//   node tools/interaction-map.mjs
//   node tools/interaction-map.mjs --check
//   node tools/interaction-map.mjs --json
//
// Optional paths make the scanner reusable for another Svelte tree:
//   --src <directory>       default: desktop/src
//   --tasks <file>          default: docs/interaction-tasks.json
//   --output <file>         default: docs/interaction-map.md
//   --json-output <file>    default: docs/interaction-map.json

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { analyseInteractionMap, loadCompiler } from "./interaction-map-analysis.mjs";
import { renderMarkdown } from "./interaction-map-report.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

const sourceRoot = resolve(ROOT, option("--src", "desktop/src"));
const tasksPath = resolve(ROOT, option("--tasks", "docs/interaction-tasks.json"));
const markdownPath = resolve(ROOT, option("--output", "docs/interaction-map.md"));
const jsonPath = resolve(ROOT, option("--json-output", "docs/interaction-map.json"));

function main() {
  const { parse } = loadCompiler(ROOT);
  const report = analyseInteractionMap({ root: ROOT, sourceRoot, tasksPath, parse });
  const json = `${JSON.stringify(report, null, 2)}\n`;
  const markdown = renderMarkdown(report);
  if (args.has("--json")) {
    process.stdout.write(json);
    return;
  }
  if (args.has("--check")) {
    const problems = [];
    if (!existsSync(markdownPath) || readFileSync(markdownPath, "utf8") !== markdown) problems.push(`${relative(ROOT, markdownPath)} is stale`);
    if (!existsSync(jsonPath) || readFileSync(jsonPath, "utf8") !== json) problems.push(`${relative(ROOT, jsonPath)} is stale`);
    if (report.warnings.length) problems.push(...report.warnings);
    if (problems.length) {
      console.error(problems.map((problem) => `Interaction map: ${problem}`).join("\n"));
      process.exitCode = 1;
      return;
    }
    console.log(`Interaction map is current: ${report.summary.buttons} buttons, ${report.summary.clickable} clickable controls, ${report.tasks.length} tasks.`);
    return;
  }
  writeFileSync(markdownPath, markdown);
  writeFileSync(jsonPath, json);
  console.log(`Wrote ${relative(ROOT, markdownPath)} and ${relative(ROOT, jsonPath)}: ${report.summary.buttons} buttons, ${report.summary.clickable} clickable controls, ${report.tasks.length} tasks.`);
  if (report.warnings.length) for (const warning of report.warnings) console.warn(`  warning: ${warning}`);
}

if (args.has("--help") || [...args].some((arg) => arg.startsWith("--") && !["--check", "--json", "--help", "--src", "--tasks", "--output", "--json-output"].includes(arg))) {
  console.log("Usage: node tools/interaction-map.mjs [--check | --json] [--src dir] [--tasks file] [--output file] [--json-output file]");
} else {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
