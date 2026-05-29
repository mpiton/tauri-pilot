// Dependency-free behavioural tests for the bridge `select` action (#113).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. The mock `<select>` reproduces the DOM spec
// quirk at the heart of #113: assigning `HTMLSelectElement.value` a string that
// matches no `<option>` silently yields `value === ""` / `selectedIndex === -1`.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.select.test.mjs

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

// Build a fake <select> whose prototype carries a `value` accessor matching the
// HTMLSelectElement spec: setting `.value` selects the option whose value
// matches, otherwise clears the selection. `nativeValueSetter` in the bridge
// reads the setter off this prototype, so the accessor must live on the proto.
function makeSelect(options) {
  const proto = {
    get value() {
      const sel = this._options.find((o) => o.selected);
      return sel ? sel.value : "";
    },
    set value(v) {
      const target = String(v);
      let hit = false;
      for (const o of this._options) {
        if (!hit && o.value === target) {
          o.selected = true;
          hit = true;
        } else {
          o.selected = false;
        }
      }
    },
    get selectedIndex() {
      return this._options.findIndex((o) => o.selected);
    },
    dispatchEvent(event) {
      this.events.push(event.type);
      return true;
    },
    focus() {},
  };
  const el = Object.create(proto);
  el.tagName = "SELECT";
  el._options = options.map((o) => ({ value: o.value, text: o.text, selected: false }));
  el.events = [];
  Object.defineProperty(el, "options", { get() { return this._options; } });
  return el;
}

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
function loadBridge({ queryResult } = {}) {
  Object.assign(console, REAL_CONSOLE);

  globalThis.window = { fetch() {} };
  globalThis.document = {
    querySelector(selector) {
      if (queryResult === undefined) {
        throw new Error("unexpected querySelector(" + selector + ")");
      }
      return queryResult;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;

  // Indirect eval runs in global scope; the IIFE then resolves bare `window`,
  // `document`, etc. against the globals set above.
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("select by option value selects the matching option", () => {
  const el = makeSelect([
    { value: "user", text: "User" },
    { value: "admin", text: "Admin" },
    { value: "editor", text: "Editor" },
  ]);
  const pilot = loadBridge({ queryResult: el });

  const result = pilot.select({ selector: "select[name=role]", value: "admin" });

  assert.deepEqual(result, { ok: true });
  assert.equal(el.value, "admin");
  assert.equal(el.selectedIndex, 1);
  assert.ok(el.events.includes("change"), "should dispatch a change event");
});

test("select by visible label selects the matching option", () => {
  const el = makeSelect([
    { value: "user", text: "User" },
    { value: "admin", text: "Admin" },
    { value: "editor", text: "Editor" },
  ]);
  const pilot = loadBridge({ queryResult: el });

  const result = pilot.select({ selector: "select[name=role]", value: "Editor" });

  assert.deepEqual(result, { ok: true });
  assert.equal(el.value, "editor");
  assert.equal(el.selectedIndex, 2);
  assert.ok(el.events.includes("change"), "should dispatch a change event");
});

test("select throws when no option matches value or label", () => {
  const el = makeSelect([
    { value: "user", text: "User" },
    { value: "admin", text: "Admin" },
    { value: "editor", text: "Editor" },
  ]);
  const pilot = loadBridge({ queryResult: el });

  assert.throws(
    () => pilot.select({ selector: "select[name=role]", value: "zzz" }),
    /no option/i,
  );
  // The selection must be left untouched, never silently cleared to -1.
  assert.equal(el.selectedIndex, -1);
});

test("select still rejects a non-<select> target", () => {
  const notSelect = { tagName: "INPUT", dispatchEvent() { return true; }, focus() {} };
  const pilot = loadBridge({ queryResult: notSelect });

  assert.throws(
    () => pilot.select({ selector: "input[name=role]", value: "admin" }),
    /<select>/i,
  );
});
