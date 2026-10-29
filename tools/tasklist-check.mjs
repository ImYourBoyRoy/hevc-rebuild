// ./tools/tasklist-check.mjs
// Validates and summarizes the dated tasklists (MM-DD-YYYY_tasklist.md).
// Run: node tools/tasklist-check.mjs [--dir <path>] [--open] [--json]
//   --dir   tasklist directory (default: ../tasklist, then ./tasklist)
//   --open  list every open "[ ]" item, newest tasklist first (session-start view)
//   --json  machine-readable report
// Rules checked: filename pattern; newest list is "latest"; every item is a
// "### T<n>.<m> title" heading with checkboxes; latest list needs
// "**Files:**" per item, an "Open questions" section and a "Supersedes" note.
// Exit 1 on a violation in the latest list; older lists only warn (legacy).

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};

const candidates = [option("--dir"), resolve("..", "tasklist"), resolve("tasklist")].filter(Boolean);
const dir = candidates.find((candidate) => existsSync(candidate));
if (!dir) {
  console.error(`tasklist directory not found (tried: ${candidates.join(", ")})`);
  process.exit(2);
}

const NAME = /^(\d{2})-(\d{2})-(\d{4})_tasklist\.md$/;
const sortKey = (file) => {
  const [, month, day, year] = NAME.exec(file);
  return `${year}${month}${day}`;
};

const files = readdirSync(dir).filter((file) => file.endsWith(".md"));
const problems = [];
const lists = [];

for (const file of files) {
  if (!NAME.test(file)) {
    problems.push({ file, level: "error", message: "filename must match MM-DD-YYYY_tasklist.md" });
    continue;
  }
  const text = readFileSync(join(dir, file), "utf8");
  const items = [];
  let current = null;
  for (const line of text.split("\n")) {
    const heading = /^### (T\d+(?:\.\d+)?[a-z]?)\s+(.*)$/.exec(line);
    if (heading) {
      current = { id: heading[1], title: heading[2], open: 0, done: 0, hasFiles: false, openLines: [] };
      items.push(current);
      continue;
    }
    if (!current) continue;
    if (/^\*\*Files:\*\*/.test(line)) current.hasFiles = true;
    if (/^- \[ \] /.test(line)) {
      current.open += 1;
      current.openLines.push(line.slice(6).trim());
    } else if (/^- \[x\] /i.test(line)) current.done += 1;
  }
  lists.push({
    file,
    key: sortKey(file),
    items,
    hasOpenQuestions: /^## Open questions/m.test(text),
    hasSupersedes: /^> \*\*Supersedes:\*\*/m.test(text),
    open: items.reduce((sum, item) => sum + item.open, 0),
    done: items.reduce((sum, item) => sum + item.done, 0),
  });
}

lists.sort((a, b) => b.key.localeCompare(a.key));
const latest = lists[0];

for (const list of lists) {
  const strict = list === latest;
  const level = strict ? "error" : "warning";
  if (list.items.length === 0) problems.push({ file: list.file, level, message: "no '### T<n>.<m>' items found" });
  if (!strict) continue;
  for (const item of list.items) {
    if (!item.hasFiles) problems.push({ file: list.file, level, message: `${item.id}: missing **Files:** line` });
    if (item.open + item.done === 0) problems.push({ file: list.file, level, message: `${item.id}: no checkbox items` });
  }
  if (!list.hasOpenQuestions) problems.push({ file: list.file, level, message: "missing '## Open questions' section" });
  if (!list.hasSupersedes) problems.push({ file: list.file, level, message: "missing '> **Supersedes:**' note" });
}

const report = {
  dir,
  latest: latest?.file ?? null,
  lists: lists.map(({ file, items, open, done }) => ({ file, items: items.length, open, done })),
  problems,
};

if (flag("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`tasklists in ${dir} (newest wins on conflict)`);
  for (const list of report.lists) {
    console.log(`  ${list.file === report.latest ? "*" : " "} ${list.file}: ${list.items} items, ${list.done} done, ${list.open} open`);
  }
  if (flag("--open")) {
    for (const list of lists) {
      for (const item of list.items.filter((entry) => entry.open > 0)) {
        console.log(`\n[${list.file}] ${item.id} ${item.title}`);
        for (const line of item.openLines) console.log(`  [ ] ${line}`);
      }
    }
  }
  for (const problem of problems) console.log(`${problem.level.toUpperCase()}: ${problem.file}: ${problem.message}`);
  if (problems.length === 0) console.log("tasklists valid");
}
process.exitCode = problems.some((problem) => problem.level === "error") ? 1 : 0;
