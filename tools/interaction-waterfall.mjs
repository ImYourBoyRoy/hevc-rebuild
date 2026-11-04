#!/usr/bin/env node
// ./tools/interaction-waterfall.mjs
// Generates and checks the state-oriented multiflow UI waterfall map.
//
// Run from the repository root:
//   node tools/interaction-waterfall.mjs
//   node tools/interaction-waterfall.mjs --check
//   node tools/interaction-waterfall.mjs --json

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadCompiler } from "./interaction-map-analysis.mjs";
import { analyseInteractionWaterfalls } from "./interaction-waterfall-analysis.mjs";
import { renderWaterfallMarkdown } from "./interaction-waterfall-report.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

const sourceRoot = resolve(ROOT, option("--src", "desktop/src"));
const configPath = resolve(ROOT, option("--config", "docs/interaction-waterfalls.json"));
const tasksPath = resolve(ROOT, option("--tasks", "docs/interaction-tasks.json"));
const markdownPath = resolve(ROOT, option("--output", "docs/interaction-waterfall.md"));
const jsonPath = resolve(ROOT, option("--json-output", "docs/interaction-waterfall.json"));

function main() {
  const { parse } = loadCompiler(ROOT);
  const report = analyseInteractionWaterfalls({ root: ROOT, sourceRoot, configPath, tasksPath, parse });
  const json = `${JSON.stringify(report, null, 2)}\n`;
  const markdown = renderWaterfallMarkdown(report);
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
      console.error(problems.map((problem) => `Interaction waterfall: ${problem}`).join("\n"));
      process.exitCode = 1;
      return;
    }
    console.log(`Interaction waterfall is current: ${report.summary.states} states, ${report.summary.flows} flows, ${report.summary.warnings} warnings.`);
    return;
  }
  writeFileSync(markdownPath, markdown);
  writeFileSync(jsonPath, json);
  console.log(`Wrote ${relative(ROOT, markdownPath)} and ${relative(ROOT, jsonPath)}: ${report.summary.states} states, ${report.summary.flows} flows.`);
  if (report.warnings.length) for (const warning of report.warnings) console.warn(`  warning: ${warning}`);
}

const validOptions = new Set(["--check", "--json", "--help", "--src", "--config", "--tasks", "--output", "--json-output"]);
if (args.has("--help") || [...args].some((arg) => arg.startsWith("--") && !validOptions.has(arg))) {
  console.log("Usage: node tools/interaction-waterfall.mjs [--check | --json] [--src dir] [--config file] [--tasks file] [--output file] [--json-output file]");
} else {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
