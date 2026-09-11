// Dependency-free behavioural tests for the bridge `check` action (#154).
//
// `check` used to assign `el.checked = !el.checked` on any target. On a <div>
// that creates an expando and reports ok. It must accept only checkbox and
// radio inputs, using a realm-safe tag+type guard (not `instanceof`).
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

function makeInput(type, checked = false) {
  return {
    tagName: "INPUT",
    type,
    checked,
    events: [],
    focus() {},
    dispatchEvent(event) {
      this.events.push(event.type);
      return true;
    },
  };
}

function loadBridge(queryResult) {
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
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("check toggles checkbox and radio inputs", () => {
  for (const type of ["checkbox", "radio"]) {
    const el = makeInput(type, false);
    const pilot = loadBridge(el);
    assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
    assert.equal(el.checked, true);
    assert.ok(el.events.includes("change"));
    assert.deepEqual(pilot.check({ selector: "input" }), { ok: true });
    assert.equal(el.checked, false);
  }
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
