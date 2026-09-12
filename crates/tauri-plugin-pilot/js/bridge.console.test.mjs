// Pilot's console hook must survive later wrappers (#190).
//
// The bridge installs capture with a plain `console[level] = wrapper` at
// document-start. Any page code that later assigns console.log without
// chaining to what was there silently drops pilot out of the chain, and
// `tauri-pilot logs` reports "No logs captured" forever after. Extension-heavy
// apps do this routinely.
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// real file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.console.test.mjs

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
// though these tests only care about console.
function loadBridge() {
  Object.assign(console, REAL_CONSOLE);
  globalThis.location = { href: "https://app.example/" };
  globalThis.XMLHttpRequest = function () {};
  globalThis.XMLHttpRequest.prototype.open = function () {};
  globalThis.XMLHttpRequest.prototype.send = function () {};
  globalThis.XMLHttpRequest.prototype.addEventListener = function () {};
  globalThis.XMLHttpRequest.prototype.removeEventListener = function () {};
  globalThis.document = { querySelector() { return null; } };
  globalThis.window = {
    fetch() { return Promise.resolve({ status: 200, headers: { get() { return "0"; } } }); },
    location: globalThis.location,
    addEventListener() {},
  };

  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function messages(pilot) {
  return pilot.consoleLogs({ level: "log" }).map((e) => e.args[0]);
}

test("capture survives a wrapper that does not chain", () => {
  const pilot = loadBridge();
  const seen = [];
  // The shape that breaks it today: no reference to the previous console.log.
  console.log = (...args) => { seen.push(args[0]); };

  console.log("after");

  assert.deepEqual(messages(pilot), ["after"]);
  assert.deepEqual(seen, ["after"], "the replacement must still receive the call");
});

test("capture survives a wrapper that does chain", () => {
  const pilot = loadBridge();
  const seen = [];
  const previous = console.log;
  console.log = (...args) => { seen.push(args[0]); previous.apply(console, args); };

  console.log("chained");

  assert.deepEqual(messages(pilot), ["chained"], "must be recorded exactly once");
  assert.deepEqual(seen, ["chained"]);
});

test("a wrapper that calls back through console does not recurse forever", () => {
  const pilot = loadBridge();
  let calls = 0;
  // `previous` is pilot's wrapper, so a naive re-entry would loop.
  const previous = console.log;
  console.log = function (...args) {
    calls += 1;
    previous.apply(console, args);
  };

  console.log("reentrant");

  assert.equal(calls, 1);
  assert.deepEqual(messages(pilot), ["reentrant"], "must not be recorded twice");
});

test("two stacked wrappers both run and capture still works", () => {
  const pilot = loadBridge();
  const order = [];
  const first = console.log;
  console.log = (...args) => { order.push("first"); first.apply(console, args); };
  const second = console.log;
  console.log = (...args) => { order.push("second"); second.apply(console, args); };

  console.log("stacked");

  assert.deepEqual(order, ["second", "first"]);
  assert.deepEqual(messages(pilot), ["stacked"]);
});

test("restoring a saved console.log keeps capture alive", () => {
  const pilot = loadBridge();
  const saved = console.log;
  console.log = () => {};
  console.log = saved;

  console.log("restored");

  assert.deepEqual(messages(pilot), ["restored"]);
});

test("assigning a non-function falls back to the original console", () => {
  const pilot = loadBridge();
  console.log = "not a function";

  assert.doesNotThrow(() => console.log("still fine"));
  assert.deepEqual(messages(pilot), ["still fine"]);
});

test("each level keeps its own downstream", () => {
  const pilot = loadBridge();
  const warns = [];
  console.warn = (...args) => { warns.push(args[0]); };

  console.log("to log");
  console.warn("to warn");

  assert.deepEqual(warns, ["to warn"], "replacing warn must not divert log");
  assert.deepEqual(messages(pilot), ["to log"]);
  assert.deepEqual(pilot.consoleLogs({ level: "warn" }).map((e) => e.args[0]), ["to warn"]);
});
