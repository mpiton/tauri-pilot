// Dependency-free tests for network capture skipping Pilot IPC (#153).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// real file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.network.test.mjs

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

function loadBridge() {
  Object.assign(console, REAL_CONSOLE);

  globalThis.location = { href: "https://app.example/" };
  globalThis.window = {
    fetch(input) {
      return Promise.resolve({
        status: 200,
        headers: { get() { return "0"; } },
      });
    },
    location: globalThis.location,
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  globalThis.document = { querySelector() { return null; } };

  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("plugin IPC fetch is omitted from networkRequests", async () => {
  const pilot = loadBridge();
  await window.fetch("http://ipc.localhost/plugin%3Apilot%7C__callback");
  await window.fetch("https://app.example/api");
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, ["https://app.example/api"]);
});

test("an app URL whose query contains the IPC substring is still recorded", async () => {
  const pilot = loadBridge();
  const app = "https://app.example/search?q=plugin%3Apilot%7C__callback";
  await window.fetch(app);
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, [app]);
});

test("an app path that only contains the IPC substring is still recorded", async () => {
  const pilot = loadBridge();
  const app = "https://app.example/api/plugin%3Apilot%7C__callback";
  await window.fetch(app);
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, [app]);
});
