// Dependency-free behavioural tests for the bridge `drag` action (#130).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. #130 is about the offset path: it resolves the
// drop point with `elementFromPoint`, which is viewport-bound, so a source
// element outside the viewport made the lookup fail with a misleading
// "No element at offset" error. The bridge must scroll the source into view
// first, like a user would, and name the viewport in the residual error.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.drag.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const BRIDGE_SRC = readFileSync(join(here, "bridge.js"), "utf8");

// Real console methods, captured once so each bridge load re-wraps the
// originals instead of stacking wrappers across tests.
const REAL_CONSOLE = {
  log: console.log.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  info: console.info.bind(console),
};

// Viewport-relative rect helper, mirroring getBoundingClientRect().
function rect(left, top, width, height) {
  return { left, top, width, height, right: left + width, bottom: top + height };
}

// Element mock recording dispatched events and scrollIntoView calls. When
// `visibleRect` is given, scrollIntoView swaps the rect to it, simulating the
// element entering the viewport.
function makeElement(initialRect, { visibleRect } = {}) {
  return {
    _rect: initialRect,
    dispatched: [],
    scrollCalls: [],
    getBoundingClientRect() {
      return this._rect;
    },
    scrollIntoView(options) {
      this.scrollCalls.push(options);
      if (visibleRect) this._rect = visibleRect;
    },
    dispatchEvent(event) {
      this.dispatched.push(event);
      return true;
    },
  };
}

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
// `elements` maps selectors to mocks; `elementFromPoint` resolves drop points.
function loadBridge({ elements = {}, elementFromPoint } = {}) {
  Object.assign(console, REAL_CONSOLE);

  globalThis.htmlToImage = {
    async toPng() {
      return "data:image/png;base64,AAAA";
    },
  };

  const fromPointCalls = [];
  globalThis.window = { fetch() {} };
  globalThis.document = {
    documentElement: { clientWidth: 800, clientHeight: 700 },
    body: {},
    querySelector(selector) {
      return elements[selector] || null;
    },
    elementFromPoint(x, y) {
      fromPointCalls.push({ x, y });
      return elementFromPoint ? elementFromPoint(x, y) : null;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;

  // drag() constructs DataTransfer, MouseEvent, and DragEvent.
  globalThis.DataTransfer = class DataTransfer {
    constructor() {
      this.items = { add() {} };
    }
  };
  class FakeUIEvent {
    constructor(type, init) {
      this.type = type;
      Object.assign(this, init || {});
    }
  }
  globalThis.MouseEvent = class MouseEvent extends FakeUIEvent {};
  globalThis.DragEvent = class DragEvent extends FakeUIEvent {};

  // Indirect eval runs in global scope; the IIFE then resolves bare `window`,
  // `document`, etc. against the globals set above.
  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, fromPointCalls };
}

test("drag with offset scrolls a below-fold source into view before resolving the drop point", () => {
  // Source center starts at (110,1510), far below the 700px viewport. After
  // scrollIntoView the center lands at (110,350); offset (130,0) puts the drop
  // point at (240,350).
  const source = makeElement(rect(100, 1500, 20, 20), {
    visibleRect: rect(100, 340, 20, 20),
  });
  const dropTarget = makeElement(rect(200, 300, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#slider-thumb": source },
    elementFromPoint: (x, y) => (x === 240 && y === 350 ? dropTarget : null),
  });

  const result = pilot.drag({
    source: { selector: "#slider-thumb" },
    offset: { x: 130, y: 0 },
  });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 1);
  // "instant" so a page-level `scroll-behavior: smooth` cannot turn the scroll
  // into an animation that outlives the synchronous rect recompute below.
  assert.deepEqual(source.scrollCalls[0], {
    behavior: "instant",
    block: "center",
    inline: "center",
  });
  // The drop point must come from the post-scroll rect, not the stale one.
  assert.deepEqual(fromPointCalls, [{ x: 240, y: 350 }]);
  const drop = dropTarget.dispatched.find((e) => e.type === "drop");
  assert.ok(drop, "drop event dispatched on the resolved target");
  assert.equal(drop.clientX, 240);
  assert.equal(drop.clientY, 350);
});

test("drag with offset scrolls an above-viewport source into view", () => {
  // Source center starts at (110,-40), above the fold.
  const source = makeElement(rect(100, -50, 20, 20), {
    visibleRect: rect(100, 340, 20, 20),
  });
  const dropTarget = makeElement(rect(200, 300, 100, 100));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => dropTarget,
  });

  const result = pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 } });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 1);
});

test("drag with offset does not scroll a fully visible source", () => {
  const source = makeElement(rect(100, 100, 20, 20));
  const dropTarget = makeElement(rect(200, 80, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => dropTarget,
  });

  const result = pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 } });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.deepEqual(fromPointCalls, [{ x: 240, y: 110 }]);
});

test("drag with offset does not scroll a source wider than the viewport when its center is visible", () => {
  // rect spills past both horizontal edges (canvas/timeline case) but the
  // center (400,110) — the actual start point — is inside the 800x700
  // viewport, so scrolling would only move a usable start point around.
  const source = makeElement(rect(-100, 100, 1000, 20));
  const dropTarget = makeElement(rect(500, 80, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#timeline": source },
    elementFromPoint: () => dropTarget,
  });

  const result = pilot.drag({ source: { selector: "#timeline" }, offset: { x: 130, y: 0 } });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.deepEqual(fromPointCalls, [{ x: 530, y: 110 }]);
});

test("drag with offset names the viewport when the drop point lands outside it", () => {
  // Offset pushes the drop point to x=5110, past the 800px-wide viewport.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  assert.throws(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 5000, y: 0 } }),
    /outside the viewport \(800x700\)/,
  );
});

test("drag with offset reports the computed drop point when nothing is there", () => {
  // Drop point (240,110) is inside the viewport but hits no element.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  assert.throws(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 } }),
    /No element at drop point \(240,110\)/,
  );
});

test("drag with offset echoes a defaulted axis as 0 in the error, not undefined", () => {
  // MCP passes the offset object through unvalidated, so {x:130} without y is
  // reachable. The computation defaults y to 0; the message must match.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  assert.throws(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130 } }),
    /for offset \(130,0\)/,
  );
});

test("drag to a target element never scrolls and skips elementFromPoint", () => {
  // Both elements below the fold: the target path dispatches directly on the
  // resolved elements, so it works without scrolling and must stay that way.
  const source = makeElement(rect(100, 1500, 20, 20));
  const target = makeElement(rect(400, 1600, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#card": source, "#column": target },
  });

  const result = pilot.drag({
    source: { selector: "#card" },
    target: { selector: "#column" },
  });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.equal(target.scrollCalls.length, 0);
  assert.equal(fromPointCalls.length, 0);
  assert.ok(target.dispatched.some((e) => e.type === "drop"));
});
