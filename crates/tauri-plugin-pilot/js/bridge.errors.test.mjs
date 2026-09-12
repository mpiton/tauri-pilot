// Uncaught errors and unhandled rejections must reach the log buffer (#188).
//
// README step 6 of the AI-agent workflow is `tauri-pilot logs --level error`
// to "check for JS errors", but the bridge only wrapped console.*. A page that
// throws never calls console.error itself — the browser prints that — so the
// one command documented for finding JS errors could not see them.
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// real file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.errors.test.mjs

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

// The bridge wraps fetch/XHR on load, so the mock has to carry both even
// though these tests only care about the error listeners.
function baseGlobals() {
  Object.assign(console, REAL_CONSOLE);
  globalThis.location = { href: "https://app.example/" };
  globalThis.XMLHttpRequest = function () {};
  globalThis.XMLHttpRequest.prototype.open = function () {};
  globalThis.XMLHttpRequest.prototype.send = function () {};
  globalThis.XMLHttpRequest.prototype.addEventListener = function () {};
  globalThis.XMLHttpRequest.prototype.removeEventListener = function () {};
  globalThis.document = { querySelector() { return null; } };
  return {
    fetch() { return Promise.resolve({ status: 200, headers: { get() { return "0"; } } }); },
    location: globalThis.location,
  };
}

function loadBridge() {
  const listeners = Object.create(null);
  globalThis.window = Object.assign(baseGlobals(), {
    addEventListener(type, handler) {
      (listeners[type] || (listeners[type] = [])).push(handler);
    },
    dispatch(type, event) {
      for (const handler of listeners[type] || []) handler(event);
    },
  });

  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, win: globalThis.window };
}

test("an uncaught error lands in the log buffer at level error", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("error", {
    message: "Uncaught TypeError: x is not a function",
    filename: "https://app.example/main.js",
    lineno: 42,
    colno: 7,
  });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.equal(errors[0].args[0], "Uncaught TypeError: x is not a function");
  assert.match(errors[0].source, /main\.js:42:7/);
});

test("an unhandled rejection lands in the log buffer at level error", () => {
  const { pilot, win } = loadBridge();
  const reason = new Error("boom");
  reason.stack = "Error: boom\n    at https://app.example/main.js:9:1";
  win.dispatch("unhandledrejection", { reason });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.match(errors[0].args[0], /Unhandled rejection: .*boom/);
});

test("a rejection with a non-Error reason is still recorded", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("unhandledrejection", { reason: "plain string" });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.match(errors[0].args[0], /Unhandled rejection: plain string/);
});

test("a failed resource load is not reported as a JS error", () => {
  const { pilot, win } = loadBridge();
  // Resource errors (img/script 404) fire the same event type on window but
  // carry no `message`; they are not JS errors and would be noise in `logs`.
  win.dispatch("error", { target: { tagName: "IMG" } });

  assert.deepEqual(pilot.consoleLogs({ level: "error" }), []);
});

test("uncaught errors share the id sequence with console entries", () => {
  const { pilot, win } = loadBridge();
  console.log("first");
  win.dispatch("error", { message: "Uncaught Error: second", filename: "a.js", lineno: 1, colno: 1 });

  const all = pilot.consoleLogs({});
  assert.equal(all.length, 2);
  assert.ok(all[1].id > all[0].id, "ids must stay monotonic across sources");
  // sinceId polling must not skip the uncaught error
  assert.equal(pilot.consoleLogs({ sinceId: all[0].id }).length, 1);
});

test("the bridge still loads when window has no addEventListener", () => {
  globalThis.window = baseGlobals();

  (0, eval)(BRIDGE_SRC);
  assert.ok(globalThis.window.__PILOT__, "bridge must not throw without addEventListener");
});

test("console entries still record the calling site", () => {
  // pushLog() is shared with the error listeners, but extractSource() skips a
  // fixed frame count, so the console wrapper must keep calling it directly.
  const { pilot } = loadBridge();
  console.log("hello");

  const [entry] = pilot.consoleLogs({});
  assert.match(entry.source, /bridge\.errors\.test\.mjs/);
});
