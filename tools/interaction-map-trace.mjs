// ./tools/interaction-map-trace.mjs
// Static handler, callback-boundary, and high-signal destination tracing for
// the source-derived interaction map.

import { functionName, isNode, shortText, sourceSlice, staticLiteral } from "./interaction-map-ast.mjs";

function callRecord(owner, name, args, node, via = "handler") {
  const position = node?.start != null ? owner.lines.at(node.start) : { line: 0, column: 0 };
  return {
    name,
    args: args.map((arg) => ({ text: shortText(arg.text), literal: arg.literal ?? null })),
    file: owner.path,
    line: position.line,
    via,
  };
}

const terminalCalls = new Set(["openPage", "invoke", "openDialog", "showModal", "close", "window.open", "openExternalLink", "navigator.clipboard.writeText"]);

function traceExpression(owner, expression, state, stack = [], via = "handler") {
  if (!expression) return [];
  const calls = [];
  const seen = new Set();

  function traceDefinition(current, name, nextStack) {
    const key = `${current.path}::${name}`;
    if (nextStack.includes(key)) return [];
    const definition = current.functions.get(name);
    if (!definition) return [];
    return traceNode(definition.type === "FunctionDeclaration" ? definition.body : definition.body, current, [...nextStack, key], "function");
  }

  function boundCalls(current, name, args, nextStack) {
    const bindings = state.bindings.get(`${current.path}::${name}`) ?? [];
    const result = [];
    for (const binding of bindings) {
      const key = `${binding.owner}::${binding.target}::${binding.prop}`;
      if (nextStack.includes(key)) continue;
      if (binding.expression.type === "Identifier") {
        const boundName = binding.expression.name;
        result.push(callRecord(state.analyses.get(binding.owner), boundName, args, binding.expression, "bound-callback"));
        const parent = state.analyses.get(binding.owner);
        if (parent) {
          if (parent.functions.has(boundName) && !terminalCalls.has(boundName)) result.push(...traceDefinition(parent, boundName, [...nextStack, key]));
          else if (!parent.functions.has(boundName)) result.push(...boundCalls(parent, boundName, args, [...nextStack, key]));
        }
      } else {
        result.push(...traceNode(binding.expression, state.analyses.get(binding.owner), [...nextStack, key], "bound-callback"));
      }
    }
    return result;
  }

  function traceCall(node, current, nextStack, callVia = "handler") {
    const name = functionName(node.callee);
    if (!name) return [];
    const args = (node.arguments ?? []).map((argument) => ({ text: sourceSlice(current.source, argument), literal: staticLiteral(argument) }));
    const signature = `${current.path}:${node.start}:${name}`;
    if (seen.has(signature)) return [];
    seen.add(signature);
    const result = [callRecord(current, name, args, node, callVia)];
    if (current.functions.has(name) && !terminalCalls.has(name)) result.push(...traceDefinition(current, name, nextStack));
    else if (!current.functions.has(name)) result.push(...boundCalls(current, name, args, nextStack));
    for (const argument of node.arguments ?? []) result.push(...traceNode(argument, current, nextStack, "argument"));
    return result;
  }

  function traceNode(node, current, nextStack, nodeVia = via) {
    if (!node || !current || typeof node !== "object") return [];
    if (node.type === "CallExpression" || node.type === "OptionalCallExpression") return traceCall(node, current, nextStack, nodeVia);
    if (node.type === "Identifier") return [];
    const result = [];
    for (const [key, child] of Object.entries(node)) {
      if (["loc", "name_loc", "start", "end", "type", "raw", "data", "callee"].includes(key)) continue;
      if (Array.isArray(child)) {
        for (const item of child) if (isNode(item)) result.push(...traceNode(item, current, nextStack, nodeVia));
      } else if (isNode(child)) {
        result.push(...traceNode(child, current, nextStack, nodeVia));
      }
    }
    return result;
  }

  if (expression.type === "Identifier") {
    if (owner.functions.has(expression.name) && !terminalCalls.has(expression.name)) return traceDefinition(owner, expression.name, stack);
    if (owner.functions.has(expression.name) && terminalCalls.has(expression.name)) return [callRecord(owner, expression.name, [], expression, via)];
    return [callRecord(owner, expression.name, [], expression, "callback"), ...boundCalls(owner, expression.name, [], stack)];
  }
  return traceNode(expression, owner, stack, via);
}

function targetForCall(call) {
  const first = call.args[0];
  if (call.name === "openPage") return { kind: "page", value: first?.literal != null ? String(first.literal) : `dynamic:${first?.text ?? "unknown"}`, confidence: first?.literal != null ? "static" : "dynamic" };
  if (call.name === "invoke") return { kind: "native-command", value: first?.literal != null ? String(first.literal) : `dynamic:${first?.text ?? "unknown"}`, confidence: first?.literal != null ? "static" : "dynamic" };
  if (call.name === "openDialog") return { kind: "native-dialog", value: "file/folder picker", confidence: "static" };
  if (call.name === "showModal" || call.name === "close") return { kind: "modal", value: call.name, confidence: "static" };
  if (call.name === "window.open" || call.name === "openExternalLink") return { kind: "external", value: first?.literal != null ? String(first.literal) : `dynamic:${first?.text ?? "unknown"}`, confidence: first?.literal != null ? "static" : "dynamic" };
  if (call.via === "callback" || call.via === "bound-callback" || /^on[A-Z]/u.test(call.name)) return { kind: "callback", value: call.name, confidence: "boundary" };
  return null;
}

function dedupeTargets(targets) {
  const seen = new Set();
  return targets.filter((target) => {
    const key = `${target.kind}:${target.value}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function compactCalls(calls) {
  const unique = [];
  const seen = new Set();
  for (const call of calls) {
    const key = `${call.name}|${call.args.map((arg) => arg.text).join(",")}|${call.file}:${call.line}`;
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(call);
  }
  const useful = unique.filter((call) => targetForCall(call) || /^(add|apply|assign|begin|browse|choose|clear|confirm|copy|create|finish|install|load|open|persist|preview|refresh|remove|repair|request|save|scan|search|select|set|stage|start|stop|toggle|update)/u.test(call.name));
  if (useful.length <= 16) return useful;
  const terminals = useful.filter((call) => targetForCall(call));
  const kept = [...useful.slice(0, 8), ...terminals.slice(-8)];
  const result = [];
  const resultKeys = new Set();
  for (const call of kept) {
    const key = `${call.name}|${call.args.map((arg) => arg.text).join(",")}|${call.file}:${call.line}`;
    if (resultKeys.has(key)) continue;
    resultKeys.add(key);
    result.push(call);
  }
  return [...result.slice(0, 15), { name: "…", args: [], file: result[0]?.file ?? "", line: result.at(-1)?.line ?? 0, via: "truncated" }];
}

export function enrichControls(analyses, bindings) {
  const state = { analyses, bindings };
  for (const analysis of analyses.values()) {
    for (const control of analysis.controls) {
      for (const event of control.events) {
        const calls = traceExpression(analysis, event.expressionNode, state).filter((call) => call.name && !["event", "currentTarget"].includes(call.name));
        event.targets = dedupeTargets(calls.map(targetForCall).filter(Boolean));
        event.callCount = calls.length;
        event.calls = compactCalls(calls);
      }
      const hrefTarget = control.href
        ? [{ kind: control.href.startsWith("#") ? "anchor" : "external", value: control.href, confidence: "static" }]
        : [];
      control.targets = dedupeTargets([...control.events.flatMap((event) => event.targets ?? []), ...hrefTarget]);
      control.trace = control.events.flatMap((event) => event.calls ?? []).map((call) => call.name + (call.args.length ? `(${call.args.map((arg) => arg.text).join(", ")})` : ""));
      control.clickable = control.kind === "button" || control.kind === "link" || control.kind === "disclosure" || control.events.some((event) => event.name === "click");
    }
  }
  return state;
}
