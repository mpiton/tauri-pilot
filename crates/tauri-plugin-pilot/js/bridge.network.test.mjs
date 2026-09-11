// Dependency-free tests for network capture skipping Pilot IPC (#153, #156).
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

// Tauri convertFileSrc(cmd, "ipc") shapes. Unix/macOS uses the ipc: scheme;
// Windows/Android rewrite it to http(s)://ipc.localhost/.
const PILOT_IPC_URLS = [
  "ipc://localhost/plugin%3Apilot%7C__callback",
  "ipc://localhost/plugin%3Apilot%7Ccallback",
  "ipc://localhost/plugin:pilot|__callback",
  "http://ipc.localhost/plugin%3Apilot%7C__callback",
  "https://ipc.localhost/plugin%3Apilot%7C__callback",
];

function makeXhrClass() {
  function XMLHttpRequestStub() {
    this._listeners = Object.create(null);
    this.status = 0;
    this.response = "";
    this.responseType = "";
  }
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  XMLHttpRequestStub.prototype.addEventListener = function (type, fn) {
    (this._listeners[type] || (this._listeners[type] = [])).push(fn);
  };
  XMLHttpRequestStub.prototype.removeEventListener = function (type, fn) {
    const list = this._listeners[type];
    if (!list) return;
    this._listeners[type] = list.filter((listener) => listener !== fn);
  };
  XMLHttpRequestStub.prototype.getResponseHeader = function () {
    return "0";
  };
  XMLHttpRequestStub.prototype.dispatchEvent = function (event) {
    const list = this._listeners[event.type] || [];
    for (const listener of list) listener.call(this, event);
    return true;
  };
  return XMLHttpRequestStub;
}

function loadBridge({ fetchImpl } = {}) {
  Object.assign(console, REAL_CONSOLE);

  globalThis.location = { href: "https://app.example/" };
  globalThis.window = {
    fetch: fetchImpl || function () {
      return Promise.resolve({
        status: 200,
        headers: { get() { return "0"; } },
      });
    },
    location: globalThis.location,
  };
  globalThis.XMLHttpRequest = makeXhrClass();
  globalThis.document = { querySelector() { return null; } };

  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function sendXhr(url, { status = 200, errorEvent } = {}) {
  const xhr = new XMLHttpRequest();
  xhr.open("POST", url);
  xhr.status = status;
  xhr.responseType = "";
  xhr.response = "";
  xhr.send();
  xhr.dispatchEvent({ type: errorEvent || "load" });
  return xhr;
}

test("plugin IPC fetch is omitted from networkRequests", async () => {
  const pilot = loadBridge();
  for (const url of PILOT_IPC_URLS) {
    await window.fetch(url);
  }
  await window.fetch("https://app.example/api");
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, ["https://app.example/api"]);
});

test("failed plugin IPC fetch is omitted from failed-only results", async () => {
  const ipc = "ipc://localhost/plugin%3Apilot%7C__callback";
  const pilot = loadBridge({
    fetchImpl(input) {
      if (String(input).includes("plugin")) {
        return Promise.reject(new Error("denied"));
      }
      return Promise.resolve({
        status: 200,
        headers: { get() { return "0"; } },
      });
    },
  });
  await window.fetch(ipc).catch(() => {});
  assert.deepEqual(pilot.networkRequests(), []);
  assert.deepEqual(pilot.networkRequests({ failedOnly: true }), []);
});

test("plugin IPC XHR is omitted from networkRequests", () => {
  const pilot = loadBridge();
  for (const url of PILOT_IPC_URLS) {
    sendXhr(url);
  }
  sendXhr("https://app.example/api");
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, ["https://app.example/api"]);
});

test("failed plugin IPC XHR is omitted from failed-only results", () => {
  const pilot = loadBridge();
  sendXhr("ipc://localhost/plugin%3Apilot%7C__callback", { errorEvent: "error" });
  sendXhr("https://app.example/api", { status: 500 });
  const failed = pilot.networkRequests({ failedOnly: true }).map((e) => e.url);
  assert.deepEqual(failed, ["https://app.example/api"]);
});

test("non-callback IPC URLs stay in the network log", async () => {
  const pilot = loadBridge();
  const keep = [
    "ipc://localhost/plugin%3Aevent%7Cemit",
    "http://ipc.localhost/plugin%3Afoo%7Cbar",
    "https://app.example/api",
  ];
  for (const url of keep) {
    await window.fetch(url);
  }
  for (const url of keep) {
    sendXhr(url);
  }
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, keep.concat(keep));
});

test("userinfo, nested, and encoded-relative callback URLs stay in the log", async () => {
  const pilot = loadBridge();
  const keep = [
    "https://ipc.localhost:443@attacker.example/plugin:pilot|__callback",
    "https://ipc.localhost/nested/plugin:pilot|__callback",
    "ipc://evil.example/plugin:pilot|__callback",
    "ipc%3A%2F%2Flocalhost%2Fplugin%3Apilot%7C__callback",
  ];
  for (const url of keep) {
    await window.fetch(url);
  }
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, keep);
});

test("double-slash and trailing-slash callback URLs stay in the network log", async () => {
  const pilot = loadBridge();
  const keep = [
    "https://ipc.localhost//plugin:pilot|__callback",
    "https://ipc.localhost/plugin:pilot|__callback/",
    "http://ipc.localhost/plugin%3Apilot%7C__callback/",
  ];
  for (const url of keep) {
    await window.fetch(url);
  }
  for (const url of keep) {
    sendXhr(url);
  }
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, keep.concat(keep));
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

test("an app URL whose path is exactly the IPC command is still recorded", async () => {
  const pilot = loadBridge();
  const app = "https://app.example/plugin%3Apilot%7C__callback";
  await window.fetch(app);
  const urls = pilot.networkRequests().map((e) => e.url);
  assert.deepEqual(urls, [app]);
});
