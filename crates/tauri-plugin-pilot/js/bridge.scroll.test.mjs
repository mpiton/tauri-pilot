// Dependency-free behavioural tests for bridge `scroll` (#157).
//
// `scroll --ref` used to look the value up only in the snapshot id map, so a
// CSS selector like `#log` raised `Unknown ref`. Other target-taking commands
// go through `resolveTarget` (`ref` / `selector` / `x,y`). Scroll must do the
// same, and still default to `window` when no target is given.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.scroll.test.mjs

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

function makeScroller() {
  return {
    tagName: "DIV",
    scrollTop: 40,
    scrollLeft: 0,
    scrollHeight: 1000,
    clientHeight: 200,
    clientWidth: 200,
    scrollWidth: 400,
    style: { overflow: "auto", overflowY: "auto", overflowX: "auto" },
    parentElement: null,
    calls: [],
    scrollBy(dx, dy) {
      this.calls.push(["scrollBy", dx, dy]);
      this.scrollLeft += dx;
      this.scrollTop += dy;
    },
    scrollTo(x, y) {
      this.calls.push(["scrollTo", x, y]);
      this.scrollLeft = x;
      this.scrollTop = y;
    },
  };
}

function loadBridge({ queryResult, fromPoint } = {}) {
  Object.assign(console, REAL_CONSOLE);
  const windowCalls = [];
  globalThis.window = {
    fetch() {},
    scrollX: 12,
    scrollY: 80,
    innerHeight: 600,
    calls: windowCalls,
    scrollBy(dx, dy) {
      windowCalls.push(["scrollBy", dx, dy]);
    },
    scrollTo(x, y) {
      windowCalls.push(["scrollTo", x, y]);
    },
    getComputedStyle(el) {
      return (el && el.style) || { overflow: "", overflowX: "", overflowY: "" };
    },
  };
  globalThis.document = {
    documentElement: { scrollHeight: 2000, clientHeight: 600 },
    body: { scrollHeight: 1800 },
    querySelector(selector) {
      if (queryResult === undefined) {
        throw new Error("unexpected querySelector(" + selector + ")");
      }
      return queryResult;
    },
    elementFromPoint(x, y) {
      if (typeof fromPoint === "function") return fromPoint(x, y);
      return fromPoint === undefined ? null : fromPoint;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("scroll with no target scrolls the window", () => {
  const pilot = loadBridge();
  assert.deepEqual(pilot.scroll({ direction: "down", amount: 50 }), { ok: true });
  assert.deepEqual(globalThis.window.calls, [["scrollBy", 0, 50]]);
});

test("scroll with a CSS selector scrolls that element (#157)", () => {
  const el = makeScroller();
  const pilot = loadBridge({ queryResult: el });
  assert.deepEqual(pilot.scroll({ direction: "down", amount: 50, selector: "#log" }), {
    ok: true,
  });
  assert.deepEqual(el.calls, [["scrollBy", 0, 50]]);
  assert.deepEqual(globalThis.window.calls, []);
});

test("scroll throws when the selector matches nothing", () => {
  const pilot = loadBridge({ queryResult: null });
  assert.throws(
    () => pilot.scroll({ direction: "down", selector: "#missing" }),
    /No element matches selector: #missing/,
  );
});

test("scroll with coordinates scrolls the element at that point", () => {
  const el = makeScroller();
  const pilot = loadBridge({ fromPoint: el });
  assert.deepEqual(pilot.scroll({ direction: "up", amount: 20, x: 10, y: 40 }), {
    ok: true,
  });
  assert.deepEqual(el.calls, [["scrollBy", 0, -20]]);
});

test("scroll with coordinates walks up to the nearest scroller", () => {
  const scroller = makeScroller();
  const child = {
    tagName: "SPAN",
    scrollTop: 0,
    scrollLeft: 0,
    scrollHeight: 16,
    clientHeight: 16,
    scrollWidth: 40,
    clientWidth: 40,
    style: { overflow: "visible", overflowY: "visible", overflowX: "visible" },
    parentElement: scroller,
    calls: [],
    scrollBy(dx, dy) {
      this.calls.push(["scrollBy", dx, dy]);
    },
  };
  const pilot = loadBridge({ fromPoint: child });
  assert.deepEqual(pilot.scroll({ direction: "down", amount: 50, x: 10, y: 40 }), {
    ok: true,
  });
  assert.deepEqual(scroller.calls, [["scrollBy", 0, 50]]);
  assert.deepEqual(child.calls, []);
});

test("scroll throws when no element is at the coordinates", () => {
  const pilot = loadBridge({ fromPoint: null });
  assert.throws(
    () => pilot.scroll({ direction: "down", x: 1, y: 2 }),
    /No element at \(1,2\)/,
  );
});

test("scroll with an unknown snapshot ref still throws", () => {
  const pilot = loadBridge();
  assert.throws(
    () => pilot.scroll({ direction: "down", ref: "e12" }),
    /Unknown ref: e12/,
  );
});

test("scroll top/bottom on a selector set scrollTop", () => {
  const el = makeScroller();
  const pilot = loadBridge({ queryResult: el });
  assert.deepEqual(pilot.scroll({ direction: "top", selector: "#log" }), { ok: true });
  assert.equal(el.scrollTop, 0);
  assert.deepEqual(pilot.scroll({ direction: "bottom", selector: "#log" }), { ok: true });
  assert.equal(el.scrollTop, 800);
});
