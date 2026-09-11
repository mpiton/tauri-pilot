// Dependency-free behavioural tests for bridge `wait` (#159).
//
// `wait --gone` used to resolve `{ found: true }` and reject with
// `Timeout waiting for <sel>`, both of which describe the opposite of
// waiting for an element to disappear.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.wait.test.mjs

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

function loadBridge(query) {
  Object.assign(console, REAL_CONSOLE);
  let observerCb;
  globalThis.window = { fetch() {} };
  globalThis.document = {
    querySelector(selector) {
      return typeof query === "function" ? query(selector) : query;
    },
    body: {},
  };
  globalThis.MutationObserver = class {
    constructor(cb) {
      observerCb = cb;
    }
    observe() {}
    disconnect() {}
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return {
    pilot: globalThis.window.__PILOT__,
    fireMutation() {
      if (observerCb) observerCb();
    },
  };
}

test("wait --gone on a missing element reports gone, not found (#159)", async () => {
  const { pilot } = loadBridge(null);
  assert.deepEqual(
    await pilot.wait({ selector: "#already-removed", gone: true }),
    { gone: true },
  );
});

test("wait --gone times out with a disappear message (#159)", async () => {
  const { pilot } = loadBridge({ tagName: "DIV" });
  await assert.rejects(
    () => pilot.wait({ selector: "#permanent-element", gone: true, timeout: 0 }),
    /Timeout waiting for #permanent-element to disappear/,
  );
});

test("wait --gone reports gone after the element is removed (#159)", async () => {
  let el = { tagName: "DIV" };
  const { pilot, fireMutation } = loadBridge(() => el);
  const pending = pilot.wait({ selector: "#x", gone: true, timeout: 1000 });
  el = null;
  fireMutation();
  assert.deepEqual(await pending, { gone: true });
});

test("wait for an existing element still reports found", async () => {
  const { pilot } = loadBridge({ tagName: "DIV" });
  assert.deepEqual(await pilot.wait({ selector: "#already-there" }), {
    found: true,
  });
});

test("wait times out without a disappear suffix when the element never appears", async () => {
  const { pilot } = loadBridge(null);
  await assert.rejects(
    () => pilot.wait({ selector: "#missing", timeout: 0 }),
    (err) => {
      assert.match(err.message, /Timeout waiting for #missing/);
      assert.doesNotMatch(err.message, /disappear/);
      return true;
    },
  );
});
