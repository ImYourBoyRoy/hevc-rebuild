#!/usr/bin/env node
// ./tools/architecture-map.mjs
// Generates ARCHITECTURE.md: the directory waterfall of every tracked or
// non-ignored project file with line count, size and a one-line description.
//
// Run:   node tools/architecture-map.mjs            (rewrite ARCHITECTURE.md)
//        node tools/architecture-map.mjs --check    (exit 1 if stale or undescribed)
//        node tools/architecture-map.mjs --json     (machine-readable report on stdout)
//
// Inputs:  `git ls-files -co --exclude-standard`, file contents, and
//          docs/architecture-descriptions.json (path/glob -> description,
//          plus the `retiring` glob list and per-extension line budgets).
// Outputs: ARCHITECTURE.md (default mode only). --check and --json never write.
// Notes:   A description comes from the JSON map first (exact path, then the
//          longest matching glob), then from the file's own header comment.
//          A file with neither is reported as MISSING and fails --check.
//          Line budgets are advisory: over-budget files are listed, never fatal.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, statSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUTPUT = join(ROOT, "ARCHITECTURE.md");
const DESCRIPTIONS = join(ROOT, "docs", "architecture-descriptions.json");
const BINARY_EXTENSIONS = new Set([".png", ".icns", ".ico", ".jpg", ".jpeg", ".webp", ".gif", ".woff", ".woff2"]);
// Derived files whose contents depend on this report or on every other file.
const DERIVED = new Set(["ARCHITECTURE.md", "SHA256SUMS"]);
const args = new Set(process.argv.slice(2));
const unknown = [...args].filter((arg) => !["--check", "--json", "--help"].includes(arg));


function globToRegExp(glob) {
  let source = "";
  for (let index = 0; index < glob.length; index += 1) {
    const char = glob[index];
    if (char === "*" && glob[index + 1] === "*") {
      source += ".*";
      index += 1;
      if (glob[index + 1] === "/") index += 1;
    } else if (char === "*") {
      source += "[^/]*";
    } else {
      source += char.replace(/[.+^${}()|[\]\\?]/g, "\\$&");
    }
  }
  return new RegExp(`^${source}$`);
}

function loadMap() {
  if (!existsSync(DESCRIPTIONS)) return { files: {}, retiring: [], budgets: {} };
  const parsed = JSON.parse(readFileSync(DESCRIPTIONS, "utf8"));
  return { files: parsed.files ?? {}, retiring: parsed.retiring ?? [], budgets: parsed.budgets ?? {} };
}

function listFiles() {
  const output = execFileSync("git", ["ls-files", "-co", "--exclude-standard", "-z"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  return [...new Set(output.split("\0").filter(Boolean))].filter((path) => !DERIVED.has(path) && existsSync(join(ROOT, path)) && statSync(join(ROOT, path)).isFile()).sort();
}

function firstSentence(text) {
  const flat = text.replace(/\s+/g, " ").trim();
  const match = flat.match(/^(.{1,160}?[.!?])(\s|$)/);
  return (match ? match[1] : flat.slice(0, 160)).trim();
}

/** Pull a one-line description out of a file's own header comment. */
function headerDescription(path, text) {
  const extension = extname(path);
  const lines = text.split("\n").slice(0, 40);
  if (extension === ".rs") {
    const doc = lines.filter((line) => line.startsWith("//!")).map((line) => line.slice(3).trim());
    const end = doc.findIndex((line) => line === "");
    const block = (end === -1 ? doc : doc.slice(0, end)).join(" ").replace(/^\.\/\S+\s*/, "");
    return block ? firstSentence(block) : "";
  }
  if ([".ts", ".mjs", ".js"].includes(extension)) {
    const body = lines.filter((line) => line.startsWith("//") && !/^\/\/\s*\.\//.test(line) && !line.startsWith("#!")).map((line) => line.replace(/^\/\/\s?/, "").trim());
    const end = body.findIndex((line) => line === "" || /^(Run|Inputs|Outputs|Notes|Usage):/i.test(line));
    const block = (end === -1 ? body : body.slice(0, end)).join(" ");
    return block ? firstSentence(block) : "";
  }
  if (extension === ".css") {
    const match = text.match(/^\/\*([\s\S]*?)\*\//);
    if (!match) return "";
    const body = match[1].split("\n").map((line) => line.trim()).filter((line) => line && !/^\.\//.test(line));
    return body.length ? firstSentence(body.join(" ")) : "";
  }
  if (extension === ".py") {
    const match = text.match(/^(?:#.*\n)*\s*(?:from __future__[^\n]*\n\s*)?"""([\s\S]*?)"""/);
    return match ? firstSentence(match[1]) : "";
  }
  if (extension === ".md") {
    const heading = lines.find((line) => line.startsWith("# "));
    return heading ? heading.slice(2).replace(/<[^>]+>/g, "").trim() : "";
  }
  return "";
}

function describe(path, text, map, globs) {
  if (map.files[path]) return { text: map.files[path], source: "map" };
  const glob = globs.filter((entry) => entry.regexp.test(path)).sort((a, b) => b.pattern.length - a.pattern.length)[0];
  if (glob) return { text: map.files[glob.pattern], source: "map" };
  const header = text === null ? "" : headerDescription(path, text);
  return header ? { text: header, source: "header" } : { text: "", source: "missing" };
}

function countLines(text) {
  if (text.length === 0) return 0;
  const breaks = text.split("\n").length - 1;
  return text.endsWith("\n") ? breaks : breaks + 1;
}

function kib(bytes) {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KiB`;
}

function analyse() {
  const map = loadMap();
  const globs = Object.keys(map.files).filter((key) => key.includes("*")).map((pattern) => ({ pattern, regexp: globToRegExp(pattern) }));
  const retiring = map.retiring.map(globToRegExp);
  const budgetExtensions = Object.entries(map.budgets);
  const records = listFiles().map((path) => {
    const bytes = statSync(join(ROOT, path)).size;
    const binary = BINARY_EXTENSIONS.has(extname(path).toLowerCase());
    const text = binary ? null : readFileSync(join(ROOT, path), "utf8");
    const lines = text === null ? null : countLines(text);
    const description = describe(path, text, map, globs);
    const budgetEntry = budgetExtensions.find(([extension]) => path.endsWith(extension));
    const budget = budgetEntry ? budgetEntry[1] : null;
    return {
      path,
      directory: dirname(path) === "." ? "." : dirname(path),
      lines,
      bytes,
      description: description.text,
      descriptionSource: description.source,
      retiring: retiring.some((regexp) => regexp.test(path)),
      budget,
      overBudget: lines !== null && budget !== null && lines > budget,
    };
  });
  return { records, map };
}

function render({ records }) {
  const totalLines = records.reduce((sum, record) => sum + (record.lines ?? 0), 0);
  const totalBytes = records.reduce((sum, record) => sum + record.bytes, 0);
  const python = records.filter((record) => record.path.endsWith(".py"));
  const retiringRecords = records.filter((record) => record.retiring);
  const over = records.filter((record) => record.overBudget).sort((a, b) => b.lines - a.lines);
  const directories = [...new Set(records.map((record) => record.directory))].sort((a, b) => {
    if (a === ".") return -1;
    if (b === ".") return 1;
    return a.localeCompare(b);
  });
  const out = [];
  out.push("# Architecture waterfall");
  out.push("");
  out.push("<!-- GENERATED by tools/architecture-map.mjs — do not edit by hand. -->");
  out.push("<!-- Edit descriptions in docs/architecture-descriptions.json, then run: node tools/architecture-map.mjs -->");
  out.push("");
  out.push("Every tracked or non-ignored project file (except the derived `ARCHITECTURE.md` and `SHA256SUMS`), grouped by directory, with its line count, size and role. `pnpm architecture:check` (or `node tools/architecture-map.mjs --check`) fails when this file is stale or a file has no description. Rules for keeping it current are in `AGENTS.md`.");
  out.push("");
  out.push("## Totals");
  out.push("");
  out.push("| Measure | Value |");
  out.push("|---|---|");
  out.push(`| Files | ${records.length} |`);
  out.push(`| Text lines | ${totalLines.toLocaleString("en-US")} |`);
  out.push(`| Size on disk | ${kib(totalBytes)} |`);
  out.push(`| Python files still present (retirement target) | ${python.length} files / ${python.reduce((sum, record) => sum + (record.lines ?? 0), 0).toLocaleString("en-US")} lines |`);
  out.push(`| Files flagged retiring | ${retiringRecords.length} |`);
  out.push(`| Files over their advisory line budget | ${over.length} |`);
  out.push("");
  out.push("## Over-budget watchlist");
  out.push("");
  if (over.length === 0) {
    out.push("No file exceeds its advisory line budget.");
  } else {
    out.push("Budgets are advisory (see `budgets` in `docs/architecture-descriptions.json`). Split a file along a responsibility boundary before adding to it.");
    out.push("");
    out.push("| File | Lines | Budget | Over by |");
    out.push("|---|---:|---:|---:|");
    for (const record of over) out.push(`| \`${record.path}\` | ${record.lines.toLocaleString("en-US")} | ${record.budget.toLocaleString("en-US")} | ${(record.lines - record.budget).toLocaleString("en-US")} |`);
  }
  out.push("");
  out.push("## File waterfall");
  for (const directory of directories) {
    const group = records.filter((record) => record.directory === directory).sort((a, b) => a.path.localeCompare(b.path));
    const groupLines = group.reduce((sum, record) => sum + (record.lines ?? 0), 0);
    const groupBytes = group.reduce((sum, record) => sum + record.bytes, 0);
    out.push("");
    out.push(`### \`${directory === "." ? "/" : `${directory}/`}\``);
    out.push("");
    out.push(`${group.length} file${group.length === 1 ? "" : "s"} · ${groupLines.toLocaleString("en-US")} lines · ${kib(groupBytes)}`);
    out.push("");
    out.push("| File | Lines | Size | Description |");
    out.push("|---|---:|---:|---|");
    for (const record of group) {
      const name = record.path.slice(record.path.lastIndexOf("/") + 1);
      const tags = [record.retiring ? "**retiring**" : "", record.overBudget ? "**over budget**" : ""].filter(Boolean).join(" ");
      const description = (record.description || "**MISSING DESCRIPTION**").replace(/\|/g, "\\|");
      out.push(`| \`${name}\` | ${record.lines === null ? "—" : record.lines.toLocaleString("en-US")} | ${kib(record.bytes)} | ${tags ? `${tags} — ` : ""}${description} |`);
    }
  }
  out.push("");
  return out.join("\n");
}

function main() {
  const analysis = analyse();
  const missing = analysis.records.filter((record) => record.descriptionSource === "missing");
  const rendered = render(analysis);

  if (args.has("--json")) {
    console.log(JSON.stringify({ files: analysis.records.length, missingDescriptions: missing.map((record) => record.path), records: analysis.records }, null, 2));
    return;
  }

  if (args.has("--check")) {
    const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8") : "";
    const problems = [];
    if (current !== rendered) problems.push("ARCHITECTURE.md is stale. Run: node tools/architecture-map.mjs");
    for (const record of missing) problems.push(`No description for ${record.path}. Add it to docs/architecture-descriptions.json.`);
    if (problems.length) {
      for (const problem of problems) console.error(problem);
      process.exitCode = 1;
      return;
    }
    console.log(`ARCHITECTURE.md is current (${analysis.records.length} files described).`);
    return;
  }

  writeFileSync(OUTPUT, rendered);
  console.log(`Wrote ARCHITECTURE.md: ${analysis.records.length} files, ${missing.length} missing description${missing.length === 1 ? "" : "s"}.`);
  for (const record of missing) console.warn(`  missing: ${record.path}`);
}

if (args.has("--help") || unknown.length) {
  console.log("Usage: node tools/architecture-map.mjs [--check | --json]");
  process.exitCode = unknown.length ? 2 : 0;
} else {
  main();
}
