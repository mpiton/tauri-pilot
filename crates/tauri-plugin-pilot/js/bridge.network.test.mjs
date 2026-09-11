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
const OriginalURL = globalThis.URL;

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

// JSC/WebKit treats `ipc:` as a non-special scheme: `new URL(...).pathname`
// is `//localhost/plugin:...`, not `/plugin:...`. That is the #156 miss.
function WebKitIpcURL(text, base) {
  const raw = String(text);
  if (/^ipc:/i.test(raw)) {
    this.pathname = raw.replace(/^ipc:/i, "");
    return;
  }
  this.pathname = new OriginalURL(raw, base).pathname;
}

function loadBridge({ fetchImpl, URLCtor } = {}) {
  Object.assign(console, REAL_CONSOLE);
  if (URLCtor) globalThis.URL = URLCtor;

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

function restoreGlobals() {
  globalThis.URL = OriginalURL;
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
  try {
    const pilot = loadBridge();
    for (const url of PILOT_IPC_URLS) {
      await window.fetch(url);
    }
    await window.fetch("https://app.example/api");
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, ["https://app.example/api"]);
  } finally {
    restoreGlobals();
  }
});

test("plugin IPC fetch is omitted when URL.pathname looks like WebKit ipc:", async () => {
  try {
    const pilot = loadBridge({ URLCtor: WebKitIpcURL });
    const ipc = "ipc://localhost/plugin%3Apilot%7C__callback";
    await window.fetch(ipc);
    await window.fetch("https://app.example/api");
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, ["https://app.example/api"]);
  } finally {
    restoreGlobals();
  }
});

test("failed plugin IPC fetch is omitted from failed-only results", async () => {
  try {
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
  } finally {
    restoreGlobals();
  }
});

test("plugin IPC XHR is omitted from networkRequests", () => {
  try {
    const pilot = loadBridge();
    for (const url of PILOT_IPC_URLS) {
      sendXhr(url);
    }
    sendXhr("https://app.example/api");
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, ["https://app.example/api"]);
  } finally {
    restoreGlobals();
  }
});

test("failed plugin IPC XHR is omitted from failed-only results", () => {
  try {
    const pilot = loadBridge();
    sendXhr("ipc://localhost/plugin%3Apilot%7C__callback", { errorEvent: "error" });
    sendXhr("https://app.example/api", { status: 500 });
    const failed = pilot.networkRequests({ failedOnly: true }).map((e) => e.url);
    assert.deepEqual(failed, ["https://app.example/api"]);
  } finally {
    restoreGlobals();
  }
});

test("an app URL whose query contains the IPC substring is still recorded", async () => {
  try {
    const pilot = loadBridge();
    const app = "https://app.example/search?q=plugin%3Apilot%7C__callback";
    await window.fetch(app);
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, [app]);
  } finally {
    restoreGlobals();
  }
});

test("an app path that only contains the IPC substring is still recorded", async () => {
  try {
    const pilot = loadBridge();
    const app = "https://app.example/api/plugin%3Apilot%7C__callback";
    await window.fetch(app);
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, [app]);
  } finally {
    restoreGlobals();
  }
});

test("an app URL whose path is exactly the IPC command is still recorded", async () => {
  try {
    const pilot = loadBridge();
    const app = "https://app.example/plugin%3Apilot%7C__callback";
    await window.fetch(app);
    const urls = pilot.networkRequests().map((e) => e.url);
    assert.deepEqual(urls, [app]);
  } finally {
    restoreGlobals();
  }
});
