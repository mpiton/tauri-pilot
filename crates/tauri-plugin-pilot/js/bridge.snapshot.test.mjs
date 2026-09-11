// Dependency-free behavioural tests for the bridge `snapshot` (#120, #155).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. The mock reproduces the DOM quirk behind #120:
// `HTMLLIElement.value` is an IDL `long` (a number, the item's ordinal, default
// `0`), so every `<li>` makes the bridge capture a JSON integer while the plugin
// types `SnapshotElement.value` as `Option<String>` — `diff` then aborts with
// `invalid type: integer 0, expected a string`.
//
// #155: walk() only emits a node when getRole() is non-null, and ROLE_MAP has
// no DIV entry. Interactive divs (draggable, contenteditable, click handlers)
// were therefore dropped from both the full snapshot and `snapshot -i`.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.snapshot.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const BRIDGE_SRC = readFileSync(join(here, "bridge.js"), "utf8");

// Real console methods, captured once so each bridge load re-wraps the
// originals instead of stacking wrappers across tests.
const REAL_CONSOLE = {
  log: console.log.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  info: console.info.bind(console),
};

// Minimal element mock. Only the surface `snapshot`/`getRole`/`getName` read:
// tagName, nodeType, children, attributes, textContent, and the `value` IDL
// property (which the test sets explicitly, mirroring real DOM types).
function makeEl(tag, props = {}) {
  const el = {
    tagName: tag.toUpperCase(),
    nodeType: 1, // Node.ELEMENT_NODE
    children: props.children || [],
    textContent: props.text || "",
    _attrs: props.attrs || {},
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(this._attrs, name)
        ? this._attrs[name]
        : null;
    },
    hasAttribute(name) {
      return Object.prototype.hasOwnProperty.call(this._attrs, name);
    },
  };
  if ("value" in props) el.value = props.value;
  if ("disabled" in props) el.disabled = props.disabled;
  if ("isContentEditable" in props) el.isContentEditable = props.isContentEditable;
  if ("contentEditable" in props) el.contentEditable = props.contentEditable;
  if ("onclick" in props) el.onclick = props.onclick;
  return el;
}

function named(elements, name) {
  return elements.find((e) => e.name === name);
}

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
function loadBridge(body) {
  Object.assign(console, REAL_CONSOLE);

  globalThis.Node = { ELEMENT_NODE: 1, TEXT_NODE: 3 };
  globalThis.window = { fetch() {} };
  globalThis.document = {
    body,
    getElementById() {
      return null;
    },
    querySelector() {
      return null;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;

  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("snapshot never emits a numeric value for <li> items (#120)", () => {
  // Every <li> outside an <ol> reports `.value === 0` (a number) per the DOM spec.
  const li = (text) => makeEl("li", { text, value: 0 });
  const list = makeEl("ul", { children: [li("a"), li("b"), li("c"), li("d")] });
  const body = makeEl("body", { children: [list] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const items = elements.filter((e) => e.role === "listitem");

  assert.equal(items.length, 4, "all four <li> should be captured");
  for (const item of items) {
    assert.notEqual(
      typeof item.value,
      "number",
      "value must serialise as a string, never a JSON number",
    );
    if (item.value !== undefined) {
      assert.equal(typeof item.value, "string");
    }
  }
});

test("snapshot coerces a numeric ordinal (<ol> <li value>) to a string", () => {
  const li = makeEl("li", { text: "second", value: 2 });
  const list = makeEl("ol", { children: [li] });
  const body = makeEl("body", { children: [list] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const item = elements.find((e) => e.role === "listitem");

  assert.equal(item.value, "2");
});

test("snapshot preserves a genuine string value unchanged", () => {
  const input = makeEl("input", { value: "hello", attrs: { type: "text" } });
  const body = makeEl("body", { children: [input] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const field = elements.find((e) => e.value === "hello");

  assert.ok(field, "the input value should still be captured");
  assert.equal(field.value, "hello");
});

test("snapshot includes a draggable card and assigns a usable ref (#155)", () => {
  const card = makeEl("div", {
    text: "Kanban card",
    attrs: { draggable: "true", id: "card-1" },
  });
  const layout = makeEl("div", { children: [card] });
  const body = makeEl("body", { children: [layout] });
  const fullPilot = loadBridge(body);
  const full = fullPilot.snapshot().elements;
  const interactive = loadBridge(body).snapshot({ interactive: true }).elements;

  const fromFull = named(full, "Kanban card");
  const fromInteractive = named(interactive, "Kanban card");
  assert.ok(fromFull, "full snapshot must emit the draggable card");
  assert.ok(fromInteractive, "snapshot -i must emit the draggable card");
  assert.equal(fromFull.role, "generic");
  assert.equal(fromInteractive.role, "generic");
  assert.equal(typeof fromFull.ref, "string");
  assert.equal(fullPilot.resolve(fromFull.ref), card);
  assert.equal(
    full.filter((e) => e !== fromFull).length,
    0,
    "plain layout divs must stay out of the full snapshot",
  );
});

test("snapshot -i includes contenteditable, onclick attribute, and onclick property (#155)", () => {
  const editor = makeEl("div", {
    text: "Rich text",
    isContentEditable: true,
    contentEditable: "true",
  });
  const plaintext = makeEl("div", {
    text: "Plain host",
    contentEditable: "plaintext-only",
  });
  const attrEditor = makeEl("div", {
    text: "Attr editor",
    attrs: { contenteditable: "true" },
  });
  const attrClick = makeEl("div", {
    text: "Attr click",
    attrs: { onclick: "doThing()" },
  });
  const propClick = makeEl("div", {
    text: "Prop click",
    onclick() {},
  });
  const tabbable = makeEl("div", {
    text: "Tab target",
    attrs: { tabindex: "0" },
  });
  const skipTarget = makeEl("div", {
    text: "Skip wrapper",
    attrs: { tabindex: "-1" },
  });
  const inert = makeEl("div", { text: "Just a box" });
  const notDrag = makeEl("div", {
    text: "Not draggable",
    attrs: { draggable: "false" },
  });
  const body = makeEl("body", {
    children: [
      editor,
      plaintext,
      attrEditor,
      attrClick,
      propClick,
      tabbable,
      skipTarget,
      inert,
      notDrag,
    ],
  });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  const editorEl = named(interactive, "Rich text");
  const plainEl = named(interactive, "Plain host");
  const attrEl = named(interactive, "Attr click");
  const propEl = named(interactive, "Prop click");
  const tabEl = named(interactive, "Tab target");
  assert.ok(editorEl, "contenteditable host must appear in snapshot -i");
  assert.equal(editorEl.role, "textbox");
  assert.ok(plainEl, "plaintext-only contenteditable must appear in snapshot -i");
  assert.equal(plainEl.role, "textbox");
  const attrEditorEl = named(interactive, "Attr editor");
  assert.ok(attrEditorEl, "contenteditable attribute alone must appear in snapshot -i");
  assert.equal(attrEditorEl.role, "textbox");
  assert.equal(pilot.resolve(attrEditorEl.ref), attrEditor);
  assert.ok(attrEl, "onclick attribute must make a div appear in snapshot -i");
  assert.equal(attrEl.role, "generic");
  assert.ok(propEl, "onclick property must make a div appear in snapshot -i");
  assert.equal(propEl.role, "generic");
  assert.ok(tabEl, "tabindex must still emit a previously unmapped div");
  assert.equal(tabEl.role, "generic");
  assert.equal(pilot.resolve(editorEl.ref), editor);
  assert.equal(pilot.resolve(attrEl.ref), attrClick);
  assert.equal(pilot.resolve(propEl.ref), propClick);
  assert.equal(pilot.resolve(tabEl.ref), tabbable);
  assert.equal(
    named(interactive, "Skip wrapper"),
    undefined,
    "unmapped tabindex=-1 wrappers must stay out of snapshot -i",
  );
  assert.equal(
    named(interactive, "Just a box"),
    undefined,
    "inert divs must stay out of snapshot -i",
  );
  assert.equal(
    named(interactive, "Not draggable"),
    undefined,
    "draggable=false must not count as interactive",
  );
});

test("snapshot -i lists the contenteditable host, not inherited descendants (#155)", () => {
  const paragraph = makeEl("p", {
    text: "Inner paragraph",
    isContentEditable: true,
    contentEditable: "inherit",
  });
  const island = makeEl("div", {
    text: "Widget island",
    contentEditable: "false",
    attrs: { contenteditable: "false" },
  });
  const editor = makeEl("div", {
    text: "Editor",
    isContentEditable: true,
    contentEditable: "true",
    children: [paragraph, island],
  });
  const body = makeEl("body", { children: [editor] });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  assert.ok(named(interactive, "Editor"), "the editable host must appear");
  assert.equal(
    named(interactive, "Inner paragraph"),
    undefined,
    "inherited contenteditable on inner nodes must not flood snapshot -i",
  );
  assert.equal(
    named(interactive, "Widget island"),
    undefined,
    "contenteditable=false islands must stay out of snapshot -i",
  );
  assert.equal(pilot.resolve(named(interactive, "Editor").ref), editor);
});

test("snapshot -i keeps an explicit-role host with tabindex=-1 and drops an unmapped -1 wrapper (#155)", () => {
  const dialog = makeEl("div", {
    text: "Modal",
    attrs: { role: "dialog", tabindex: "-1" },
  });
  const wrapper = makeEl("div", {
    text: "Dismissable layer",
    attrs: { tabindex: "-1" },
  });
  const tabbable = makeEl("div", {
    text: "Tab target",
    attrs: { tabindex: "0" },
  });
  const body = makeEl("body", { children: [dialog, wrapper, tabbable] });
  const interactive = loadBridge(body).snapshot({ interactive: true }).elements;

  const dialogEl = named(interactive, "Modal");
  assert.ok(dialogEl, "explicit role=dialog must still appear with tabindex=-1");
  assert.equal(dialogEl.role, "dialog");
  assert.equal(
    named(interactive, "Dismissable layer"),
    undefined,
    "unmapped tabindex=-1 wrappers must stay out of snapshot -i",
  );
  const tabEl = named(interactive, "Tab target");
  assert.ok(tabEl, "tabindex=0 host must still appear");
  assert.equal(tabEl.role, "generic");
});

test("snapshot -i still lists native controls and skips a wrapping layout div (#155)", () => {
  const button = makeEl("button", { text: "Save" });
  const wrapper = makeEl("div", { children: [button] });
  const body = makeEl("body", { children: [wrapper] });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  assert.equal(interactive.length, 1);
  assert.equal(interactive[0].role, "button");
  assert.equal(interactive[0].name, "Save");
  assert.equal(pilot.resolve(interactive[0].ref), button);
});
