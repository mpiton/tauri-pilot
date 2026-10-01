// Dependency-free behavioural tests for bridge `storageDelete` (#284).
//
// `storage` could get, set, list and clear, but not remove one key. The
// bridge now exposes `storageDelete({key, session})`, which calls
// `removeItem` and reports whether the key existed. Removing a missing key
// succeeds, like `removeItem` itself.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.storage.test.mjs

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

// Web Storage subset the bridge touches: `null` for a missing key, like the
// real `getItem`.
function makeStorage(entries) {
  const map = new Map(Object.entries(entries));
  return {
    map,
    get length() {
      return map.size;
    },
    key(i) {
      return [...map.keys()][i] ?? null;
    },
    getItem(key) {
      return map.has(key) ? map.get(key) : null;
    },
    setItem(key, value) {
      map.set(key, String(value));
    },
    removeItem(key) {
      map.delete(key);
    },
    clear() {
      map.clear();
    },
  };
}

function loadBridge({ local = {}, session = {} } = {}) {
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
  const localStorage = makeStorage(local);
  const sessionStorage = makeStorage(session);
  globalThis.localStorage = localStorage;
  globalThis.sessionStorage = sessionStorage;
  globalThis.window = { fetch() {} };
  globalThis.document = { querySelector() { return null; }, body: {} };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, localStorage, sessionStorage };
}

test("storageDelete removes an existing localStorage key (#284)", () => {
  const { pilot, localStorage } = loadBridge({
    local: { auth_token: "abc", theme: "dark" },
  });
  assert.deepEqual(pilot.storageDelete({ key: "auth_token", session: false }), {
    deleted: true,
  });
  assert.equal(localStorage.getItem("auth_token"), null);
  assert.equal(localStorage.getItem("theme"), "dark", "other keys stay");
});

test("storageDelete on a missing key succeeds with deleted: false (#284)", () => {
  const { pilot, localStorage } = loadBridge({ local: { theme: "dark" } });
  assert.deepEqual(pilot.storageDelete({ key: "missing", session: false }), {
    deleted: false,
  });
  assert.equal(localStorage.length, 1);
});

test("storageDelete counts a key holding an empty string as existing (#284)", () => {
  const { pilot, localStorage } = loadBridge({ local: { empty: "" } });
  assert.deepEqual(pilot.storageDelete({ key: "empty", session: false }), {
    deleted: true,
  });
  assert.equal(localStorage.length, 0);
});

test("storageDelete with session targets sessionStorage only (#284)", () => {
  const { pilot, localStorage, sessionStorage } = loadBridge({
    local: { tab_id: "local" },
    session: { tab_id: "session" },
  });
  assert.deepEqual(pilot.storageDelete({ key: "tab_id", session: true }), {
    deleted: true,
  });
  assert.equal(sessionStorage.getItem("tab_id"), null);
  assert.equal(localStorage.getItem("tab_id"), "local");
});

test("storageDelete rejects a non-string key like storageGet (#284)", () => {
  const { pilot } = loadBridge();
  assert.throws(
    () => pilot.storageDelete({ session: false }),
    /storageDelete requires a string key/,
  );
  assert.throws(
    () => pilot.storageDelete({ key: 42, session: false }),
    /storageDelete requires a string key/,
  );
});
