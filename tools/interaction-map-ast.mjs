// ./tools/interaction-map-ast.mjs
// Shared Svelte AST and template-control extraction helpers for the
// source-derived interaction map.

import { existsSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";

export function isNode(value) {
  return Boolean(value && typeof value === "object" && typeof value.type === "string");
}

export function walkNodes(value, visit) {
  if (!value || typeof value !== "object") return;
  if (isNode(value)) visit(value);
  for (const [key, child] of Object.entries(value)) {
    if (["loc", "name_loc", "start", "end", "type", "raw", "data"].includes(key)) continue;
    if (Array.isArray(child)) child.forEach((item) => walkNodes(item, visit));
    else walkNodes(child, visit);
  }
}

export function lineMap(source) {
  const starts = [0];
  for (let index = 0; index < source.length; index += 1) {
    if (source[index] === "\n") starts.push(index + 1);
  }
  return {
    at(offset) {
      let low = 0;
      let high = starts.length;
      while (low + 1 < high) {
        const middle = Math.floor((low + high) / 2);
        if (starts[middle] <= offset) low = middle;
        else high = middle;
      }
      return { line: low + 1, column: offset - starts[low] + 1 };
    },
  };
}

export function sourceSlice(source, node) {
  return node && Number.isInteger(node.start) && Number.isInteger(node.end) ? source.slice(node.start, node.end) : "";
}

export function expressionForAttribute(attribute) {
  if (!attribute) return null;
  if (attribute.type === "EventHandler") return attribute.expression ?? null;
  const value = attribute.value ?? [];
  const expression = value.find((item) => item.type === "MustacheTag" || item.type === "RawMustacheTag" || item.type === "AttributeShorthand");
  return expression?.expression ?? null;
}

export function attributeValue(source, attribute) {
  if (!attribute) return "";
  if (attribute.value === true || attribute.value == null) return "true";
  return (attribute.value ?? []).map((part) => {
    if (part.type === "Text") return part.data;
    if (part.expression) return `{${sourceSlice(source, part.expression)}}`;
    return sourceSlice(source, part);
  }).join("").trim();
}

export function findAttribute(node, names) {
  const wanted = new Set(names);
  return (node.attributes ?? []).find((attribute) => wanted.has(attribute.name));
}

export function eventName(attribute) {
  if (attribute.type === "EventHandler") return attribute.name;
  if (typeof attribute.name !== "string") return null;
  if (attribute.name.startsWith("on:") && attribute.name.length > 3) return attribute.name.slice(3);
  if (attribute.name.startsWith("on") && attribute.name.length > 2 && /^[a-z]/u.test(attribute.name[2])) return attribute.name.slice(2);
  return null;
}

export function textContent(node, source) {
  if (!node) return "";
  if (node.type === "Text") return node.data;
  if (node.type === "MustacheTag" || node.type === "RawMustacheTag") return `{${sourceSlice(source, node.expression)}}`;
  if (node.type === "Element" || node.type === "InlineComponent" || node.type === "SlotTemplate") return (node.children ?? []).map((child) => textContent(child, source)).join(" ");
  if (node.type === "IfBlock") return [...(node.children ?? []), ...(node.else?.children ?? [])].map((child) => textContent(child, source)).join(" ");
  if (node.type === "EachBlock" || node.type === "AwaitBlock" || node.type === "KeyBlock") return (node.children ?? []).map((child) => textContent(child, source)).join(" ");
  return "";
}

export function compact(value) {
  return String(value ?? "").replace(/\s+/gu, " ").trim();
}

export function shortText(value, limit = 220) {
  const text = compact(value);
  return text.length <= limit ? text : `${text.slice(0, limit - 1)}…`;
}

export function labelFor(node, source, parentLabel = "") {
  const aria = findAttribute(node, ["aria-label"]);
  const title = findAttribute(node, ["title"]);
  const label = attributeValue(source, aria) || attributeValue(source, title) || compact(textContent(node, source)) || parentLabel;
  return compact(label).slice(0, 220) || "(unlabeled control)";
}

export function guardText(source, expression) {
  return expression ? compact(sourceSlice(source, expression)) : "";
}

export function functionName(callee) {
  if (!callee) return "";
  if (callee.type === "Identifier") return callee.name;
  if (callee.type === "TSInstantiationExpression") return functionName(callee.expression);
  if (callee.type === "ChainExpression") return functionName(callee.expression);
  if (callee.type === "MemberExpression" || callee.type === "OptionalMemberExpression") {
    const left = functionName(callee.object);
    const right = callee.computed ? "" : callee.property?.name;
    return left && right ? `${left}.${right}` : right || left;
  }
  return "";
}

export function staticLiteral(node) {
  if (!node) return null;
  if (node.type === "Literal" && (typeof node.value === "string" || typeof node.value === "number" || typeof node.value === "boolean")) return node.value;
  if (node.type === "StringLiteral") return node.value;
  return null;
}

export function collectFunctions(ast) {
  const functions = new Map();
  const statements = ast.instance?.content?.body ?? [];
  const collectStatement = (statement) => {
    if (!statement) return;
    if (statement.type === "FunctionDeclaration" && statement.id?.name) functions.set(statement.id.name, statement);
    if (statement.type === "VariableDeclaration") {
      for (const declaration of statement.declarations ?? []) {
        if (declaration.id?.name && ["ArrowFunctionExpression", "FunctionExpression"].includes(declaration.init?.type)) functions.set(declaration.id.name, declaration.init);
      }
    }
    if (statement.type === "ExportNamedDeclaration") collectStatement(statement.declaration);
  };
  statements.forEach(collectStatement);
  return functions;
}

export function resolveComponentPath(name, sourceDirectory, allPaths, root) {
  if (!name || name.includes(":")) return null;
  const direct = allPaths.find((path) => path.endsWith(`/lib/${name}.svelte`) || path.endsWith(`/routes/${name}.svelte`));
  if (direct) return direct;
  if (name.startsWith("#lib/")) {
    const libRoot = existsSync(join(sourceDirectory, "lib")) ? join(sourceDirectory, "lib") : sourceDirectory;
    const candidate = relative(root, resolve(libRoot, name.slice(5))).split(sep).join("/");
    return allPaths.find((path) => path === candidate || path === `${candidate}.svelte`) ?? null;
  }
  return null;
}

export function staticContextLabel(node, source) {
  const aria = findAttribute(node, ["aria-label"]);
  return attributeValue(source, aria);
}

export function collectMarkup(analysis, allPaths, sourceRoot, root) {
  const controls = [];
  const bindings = [];
  const line = analysis.lines;
  let ordinal = 0;

  function visitChildren(children, context) {
    let current = { ...context };
    for (const child of children ?? []) {
      visit(child, current);
      if (child.type === "Element" && ["h1", "h2", "h3"].includes(child.name)) {
        current = { ...current, heading: compact(textContent(child, analysis.source)) };
      }
    }
  }

  function visit(node, context) {
    if (!node || typeof node !== "object") return;
    if (node.type === "IfBlock") {
      const condition = guardText(analysis.source, node.expression);
      visitChildren(node.children, { ...context, guards: [...context.guards, condition] });
      if (node.else) visitChildren(node.else.children, { ...context, guards: [...context.guards, `else(${condition})`] });
      return;
    }
    if (node.type === "EachBlock" || node.type === "AwaitBlock" || node.type === "KeyBlock") {
      const expression = guardText(analysis.source, node.expression);
      visitChildren(node.children, { ...context, guards: [...context.guards, `${node.type.replace("Block", "")}(${expression})`] });
      if (node.pending) visitChildren(node.pending.children, { ...context, guards: [...context.guards, "pending"] });
      if (node.then) visitChildren(node.then.children, { ...context, guards: [...context.guards, "then"] });
      if (node.catch) visitChildren(node.catch.children, { ...context, guards: [...context.guards, "catch"] });
      return;
    }
    if (node.type === "InlineComponent") {
      const target = resolveComponentPath(node.name, sourceRoot, allPaths, root);
      if (target) {
        for (const attribute of node.attributes ?? []) {
          const expression = expressionForAttribute(attribute);
          if (!expression) continue;
          bindings.push({ target, prop: attribute.name, owner: analysis.path, expression, line: line.at(attribute.start).line });
        }
      }
      visitChildren(node.children, context);
      return;
    }
    if (node.type !== "Element") {
      if (Array.isArray(node.children)) visitChildren(node.children, context);
      return;
    }

    const region = staticContextLabel(node, analysis.source) || context.region;
    const label = labelFor(node, analysis.source, context.label);
    const events = (node.attributes ?? []).map((attribute) => ({ attribute, name: eventName(attribute) })).filter((item) => item.name);
    const href = attributeValue(analysis.source, findAttribute(node, ["href"]));
    const role = attributeValue(analysis.source, findAttribute(node, ["role"]));
    const kind = node.name === "button"
      ? "button"
      : node.name === "a" && href
        ? "link"
        : node.name === "summary"
          ? "disclosure"
          : ["input", "select", "textarea"].includes(node.name)
            ? "form-control"
            : role === "button" || role === "radio" || role === "option"
              ? "clickable"
              : null;
    if (kind) {
      ordinal += 1;
      const position = line.at(node.start);
      controls.push({
        id: `${analysis.path}:${kind}:${position.line}:${position.column}`,
        ordinal,
        kind,
        file: analysis.path,
        line: position.line,
        column: position.column,
        label,
        role: role || null,
        region: region || null,
        heading: context.heading || null,
        guards: context.guards.filter(Boolean),
        disabled: attributeValue(analysis.source, findAttribute(node, ["disabled"])) || null,
        href: href || null,
        events: events.map(({ attribute, name }) => ({
          name,
          expression: shortText(sourceSlice(analysis.source, expressionForAttribute(attribute))),
          expressionNode: expressionForAttribute(attribute),
        })),
        source: sourceSlice(analysis.source, node).slice(0, 400),
      });
    }

    const childLabel = ["label", "button", "a"].includes(node.name) ? label : context.label;
    const nextContext = {
      ...context,
      region: node.name === "section" || node.name === "nav" ? region : context.region,
      label: childLabel,
    };
    visitChildren(node.children, nextContext);
  }

  visitChildren(analysis.ast.html?.children ?? [], { guards: [], region: "", heading: "", label: "" });
  analysis.controls = controls;
  analysis.bindings = bindings;
}
