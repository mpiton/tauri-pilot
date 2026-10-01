// Dependency-free behavioural tests for bridge `visible` with `missingOk`
// (#281).
//
// `assert hidden` calls `visible` and inverts the answer. A selector that
// matches nothing threw "No element matches selector", so asserting that a
// removed element is gone failed. With `missingOk: true` a selector that
// matches nothing reports `{visible: false}`. An unknown ref still throws: it
// usually means a stale snapshot, not a removed element.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.visible.test.mjs

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

function loadBridge({ queryResult, pointResult } = {}) {
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
  globalThis.Node = { ELEMENT_NODE: 1, TEXT_NODE: 3 };
  globalThis.window = { fetch() {} };
  globalThis.getComputedStyle = (el) => el._style;
  globalThis.document = {
    body: { tagName: "BODY", nodeType: 1, children: [] },
    getElementById() {
      return null;
    },
    querySelector() {
      return queryResult ?? null;
    },
    querySelectorAll() {
      return [];
    },
    elementFromPoint() {
      return pointResult ?? null;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function shownElement() {
  return {
    tagName: "DIV",
    nodeType: 1,
    offsetWidth: 10,
    offsetHeight: 10,
    _style: { display: "block", visibility: "visible", opacity: "1" },
  };
}

test("visible with missingOk reports a missing selector as not visible (#281)", () => {
  const pilot = loadBridge();
  assert.deepEqual(
    pilot.visible({ selector: "#does-not-exist", missingOk: true }),
    { visible: false },
  );
});

test("visible without missingOk still throws on a missing selector", () => {
  const pilot = loadBridge();
  assert.throws(
    () => pilot.visible({ selector: "#does-not-exist" }),
    /No element matches selector: #does-not-exist/,
  );
});

test("visible with missingOk still throws on an unknown ref (#281)", () => {
  const pilot = loadBridge();
  assert.throws(
    () => pilot.visible({ ref: "e5", missingOk: true }),
    /Unknown ref: e5/,
  );
});

test("visible with missingOk lets a ref win over a missing selector", () => {
  // `resolveTarget` reads the ref first, so the relaxation must too.
  const pilot = loadBridge();
  assert.throws(
    () => pilot.visible({ ref: "e5", selector: "#gone", missingOk: true }),
    /Unknown ref: e5/,
  );
});

test("visible with missingOk resolves coordinates as before", () => {
  // Only a selector is relaxed; coordinates go through `elementFromPoint`.
  const pilot = loadBridge({ pointResult: shownElement() });
  assert.deepEqual(
    pilot.visible({ x: 5, y: 5, missingOk: true }),
    { visible: true },
  );
});

test("visible with missingOk still reports a present element's state", () => {
  const pilot = loadBridge({ queryResult: shownElement() });
  assert.deepEqual(
    pilot.visible({ selector: "#shown", missingOk: true }),
    { visible: true },
  );
});
