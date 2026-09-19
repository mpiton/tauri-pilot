// Dependency-free behavioural tests for the bridge `check` action (#154, #177,
// #212).
//
// `check` used to assign `el.checked = !el.checked` on any target. On a <div>
// that creates an expando and reports ok. It must accept only checkbox and
// radio inputs, using a realm-safe tag+type guard (not `instanceof`).
// Checkboxes toggle. Radios select and stay selected (no click-to-uncheck).
// The change must go through a native click so React's onChange runs (#212).
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.check.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const BRIDGE_SRC = readFileSync(join(here, "bridge.js"), "utf8");
const REAL_CONSOLE = {
  log: console.log.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  info: console.info.bind(console),
};

// Models the browser and React's value tracker. React wraps the instance
// `checked` setter, so a script write updates the tracker as well and React
// sees no change. A native click flips the state behind the tracker and fires
// click, input, change; React compares state with the tracker on `click` and
// runs onChange when they differ. A disabled input ignores the click.
function makeInput(type, checked = false) {
  let state = checked;
  return {
    tagName: "INPUT",
    type,
    tracker: checked,
    reactChanges: 0,
    disabled: false,
    events: [],
    get checked() {
      return state;
    },
    set checked(value) {
      state = Boolean(value);
      this.tracker = state;
    },
    focus() {},
    click() {
      if (this.disabled) return;
      const before = state;
      if (!(type === "radio" && state)) state = type === "radio" ? true : !state;
      this.dispatchEvent({ type: "click" });
      if (state !== before) {
        this.dispatchEvent({ type: "input" });
        this.dispatchEvent({ type: "change" });
      }
    },
    dispatchEvent(event) {
      this.events.push(event.type);
      if (event.type === "click" && state !== this.tracker) {
        this.tracker = state;
        this.reactChanges += 1;
      }
      return true;
    },
  };
}

function loadBridge(queryResult) {
  // Object.assign would go through the previous bridge's console setter and
  // stack this load on top of it; redefining restores a native console.
  for (const level of Object.keys(REAL_CONSOLE)) {
    Object.defineProperty(console, level, {
      value: REAL_CONSOLE[level],
      writable: true,
      configurable: true,
      enumerable: true,
    });
  }
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
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("check toggles a checkbox with a click React sees", () => {
  const el = makeInput("checkbox", false);
  const pilot = loadBridge(el);
  assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
  assert.equal(el.checked, true);
  assert.equal(el.reactChanges, 1);
  assert.deepEqual(el.events, ["click", "input", "change"]);
  assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
  assert.equal(el.checked, false);
  assert.equal(el.reactChanges, 2);
});

test("check selects a radio with a click React sees and leaves it selected", () => {
  const el = makeInput("radio", false);
  const pilot = loadBridge(el);
  assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
  assert.equal(el.checked, true);
  assert.equal(el.reactChanges, 1);
  assert.deepEqual(el.events, ["click", "input", "change"]);
  assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
  assert.equal(el.checked, true);
  assert.equal(el.reactChanges, 1);
  assert.deepEqual(el.events, ["click", "input", "change"]);
});

test("check on an already-selected radio fires no event", () => {
  const el = makeInput("radio", true);
  const pilot = loadBridge(el);
  assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
  assert.equal(el.checked, true);
  assert.deepEqual(el.events, []);
});

test("check throws when the click leaves the input unchanged", () => {
  const el = makeInput("checkbox", false);
  el.disabled = true;
  const pilot = loadBridge(el);
  assert.throws(() => pilot.check({ selector: "input" }), /did not change/);
  assert.equal(el.checked, false);
  assert.deepEqual(el.events, []);
});

test("check throws on a non-input target", () => {
  const el = { tagName: "DIV", checked: false, dispatchEvent() { return true; } };
  const pilot = loadBridge(el);
  assert.throws(
    () => pilot.check({ selector: "#qa-div" }),
    /checkbox|radio/i,
  );
  assert.equal(el.checked, false);
});

test("check throws on a non-checkable input", () => {
  const el = makeInput("text", false);
  const pilot = loadBridge(el);
  assert.throws(
    () => pilot.check({ selector: "input[name=q]" }),
    /checkbox|radio/i,
  );
  assert.equal(el.checked, false);
});
