// Dependency-free behavioural tests for the bridge `snapshot` value capture (#120).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. The mock reproduces the DOM quirk behind #120:
// `HTMLLIElement.value` is an IDL `long` (a number, the item's ordinal, default
// `0`), so every `<li>` makes the bridge capture a JSON integer while the plugin
// types `SnapshotElement.value` as `Option<String>` — `diff` then aborts with
// `invalid type: integer 0, expected a string`.
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
  return el;
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
