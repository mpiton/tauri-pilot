// Dependency-free behavioural tests for the bridge `drag` action (#130).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// #130 is about the offset path: it resolves the drop point with
// `elementFromPoint`, which is viewport-bound, so a source element outside the
// viewport made the lookup fail with a misleading "No element at offset" error.
// The bridge must scroll the source into view first, like a user would, and name
// the viewport in the residual error.
//
// The gesture's contents matter just as much. HTML5 DragEvents alone drive
// only `draggable="true"` handlers; JS drag libraries (dnd-kit, sortable.js,
// interact.js, react-dnd's mouse backend) activate on mousedown and then track
// repeated mousemove/pointermove on `document` before committing on mouseup. The
// gesture must satisfy both, and must press the deepest node under the point
// because library listeners commonly sit on an inner handle.
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
  log: console.log,
  info: console.info,
  warn: console.warn,
  error: console.error,
  debug: console.debug,
};

// Keep the suite quick: the gesture's inter-step and settle delays are real
// timers, and they are covered explicitly by their own tests.
const FAST = { stepDelayMs: 0, settleMs: 0 };

// Viewport-relative rect helper, mirroring getBoundingClientRect().
function rect(left, top, width, height) {
  return { left, top, width, height, right: left + width, bottom: top + height };
}

// Element mock recording dispatched events and scrollIntoView calls. When
// `visibleRect` is given, scrollIntoView swaps the rect to it, simulating the
// element entering the viewport.
function makeElement(initialRect, { visibleRect, cancelDrop = false, descendants = [] } = {}) {
  return {
    _rect: initialRect,
    dispatched: [],
    scrollCalls: [],
    // The press target is gated on containment, so the mock needs the real
    // Node.contains() semantics: an element contains itself and its subtree.
    contains(node) {
      return node === this || descendants.includes(node);
    },
    getBoundingClientRect() {
      return this._rect;
    },
    scrollIntoView(options) {
      this.scrollCalls.push(options);
      if (visibleRect) this._rect = visibleRect;
    },
    dispatchEvent(event) {
      this.dispatched.push(event);
      // dispatchEvent returns false when a cancelable event was preventDefault-ed.
      return !(cancelDrop && event.type === "drop");
    },
  };
}

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
// `elements` maps selectors to mocks; `elementFromPoint` resolves drop points.
function loadBridge({ elements = {}, elementFromPoint, pointerEvents = true } = {}) {
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
    // The gesture's move/release events go to `document`, so it needs to record
    // them like any other node.
    dispatched: [],
    dispatchEvent(event) {
      this.dispatched.push(event);
      return true;
    },
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

  // drag() constructs DataTransfer, MouseEvent, PointerEvent, and DragEvent.
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
  if (pointerEvents) {
    globalThis.PointerEvent = class PointerEvent extends FakeUIEvent {};
  } else {
    delete globalThis.PointerEvent;
  }

  // Indirect eval runs in global scope; the IIFE then resolves bare `window`,
  // `document`, etc. against the globals set above.
  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, fromPointCalls, document: globalThis.document };
}

const types = (node) => node.dispatched.map((e) => e.type);

test("drag with offset scrolls a below-fold source into view before resolving the drop point", async () => {
  // Source center starts at (110,1510), far below the 700px viewport. After
  // scrollIntoView the center lands at (110,350); offset (130,0) puts the drop
  // point at (240,350).
  const source = makeElement(rect(100, 1500, 20, 20), {
    visibleRect: rect(100, 340, 20, 20),
  });
  const dropTarget = makeElement(rect(200, 300, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#slider-thumb": source },
    elementFromPoint: (x, y) => (x === 240 && y === 350 ? dropTarget : source),
  });

  const result = await pilot.drag({
    source: { selector: "#slider-thumb" },
    offset: { x: 130, y: 0 },
    ...FAST,
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
  // The drop point must come from the post-scroll rect, not the stale one. The
  // second lookup is the press target (deepest node under the start point).
  // Moves hit-test too, so only the leading pair is the drop point + press target.
  assert.deepEqual(fromPointCalls.slice(0, 2), [
    { x: 240, y: 350 },
    { x: 110, y: 350 },
  ]);
  const drop = dropTarget.dispatched.find((e) => e.type === "drop");
  assert.ok(drop, "drop event dispatched on the resolved target");
  assert.equal(drop.clientX, 240);
  assert.equal(drop.clientY, 350);
});

test("drag with offset scrolls an above-viewport source into view", async () => {
  // Source center starts at (110,-40), above the fold.
  const source = makeElement(rect(100, -50, 20, 20), {
    visibleRect: rect(100, 340, 20, 20),
  });
  const dropTarget = makeElement(rect(200, 300, 100, 100));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => dropTarget,
  });

  const result = await pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 }, ...FAST });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 1);
});

test("drag with offset does not scroll a fully visible source", async () => {
  const source = makeElement(rect(100, 100, 20, 20));
  const dropTarget = makeElement(rect(200, 80, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => dropTarget,
  });

  const result = await pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 }, ...FAST });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.deepEqual(fromPointCalls.slice(0, 2), [
    { x: 240, y: 110 },
    { x: 110, y: 110 },
  ]);
});

test("drag with offset does not scroll a source wider than the viewport when its center is visible", async () => {
  // rect spills past both horizontal edges (canvas/timeline case) but the
  // center (400,110) — the actual start point — is inside the 800x700
  // viewport, so scrolling would only move a usable start point around.
  const source = makeElement(rect(-100, 100, 1000, 20));
  const dropTarget = makeElement(rect(500, 80, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#timeline": source },
    elementFromPoint: () => dropTarget,
  });

  const result = await pilot.drag({ source: { selector: "#timeline" }, offset: { x: 130, y: 0 }, ...FAST });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.deepEqual(fromPointCalls.slice(0, 2), [
    { x: 530, y: 110 },
    { x: 400, y: 110 },
  ]);
});

test("drag with offset names the viewport when the drop point lands outside it", async () => {
  // Offset pushes the drop point to x=5110, past the 800px-wide viewport.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  await assert.rejects(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 5000, y: 0 }, ...FAST }),
    /outside the viewport \(800x700\)/,
  );
});

test("drag with offset reports the computed drop point when nothing is there", async () => {
  // Drop point (240,110) is inside the viewport but hits no element.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  await assert.rejects(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130, y: 0 }, ...FAST }),
    /No element at drop point \(240,110\)/,
  );
});

test("drag with offset echoes a defaulted axis as 0 in the error, not undefined", async () => {
  // MCP passes the offset object through unvalidated, so {x:130} without y is
  // reachable. The computation defaults y to 0; the message must match.
  const source = makeElement(rect(100, 100, 20, 20));
  const { pilot } = loadBridge({
    elements: { "#thumb": source },
    elementFromPoint: () => null,
  });

  await assert.rejects(
    () => pilot.drag({ source: { selector: "#thumb" }, offset: { x: 130 }, ...FAST }),
    /for offset \(130,0\)/,
  );
});

test("drag to a target element never scrolls and resolves only the press target", async () => {
  // Both elements below the fold: the target path dispatches directly on the
  // resolved elements, so it works without scrolling and must stay that way.
  // The first elementFromPoint call is the press target, not the drop point;
  // the rest are the per-move hit tests.
  const source = makeElement(rect(100, 1500, 20, 20));
  const target = makeElement(rect(400, 1600, 100, 100));
  const { pilot, fromPointCalls } = loadBridge({
    elements: { "#card": source, "#column": target },
  });

  const result = await pilot.drag({
    source: { selector: "#card" },
    target: { selector: "#column" },
    ...FAST,
  });

  assert.equal(result.ok, true);
  assert.equal(source.scrollCalls.length, 0);
  assert.equal(target.scrollCalls.length, 0);
  assert.deepEqual(fromPointCalls[0], { x: 110, y: 1510 });
  assert.ok(target.dispatched.some((e) => e.type === "drop"));
});

// ── the gesture must also drive JS drag libraries ────────────────────────────

test("drag presses, streams moves on document, and releases", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot, document: doc } = loadBridge({
    elements: { "#a": source, "#b": target },
  });

  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" }, steps: 4, ...FAST });

  // Library activation needs the press, then *several* moves past a distance
  // threshold, then the release — in that order.
  const docTypes = types(doc);
  assert.deepEqual(docTypes.filter((t) => t === "mousemove").length, 4);
  assert.deepEqual(docTypes.filter((t) => t === "pointermove").length, 4);
  // Pointer events lead, their compatibility mouse events follow — the order a
  // browser produces, and the one `click()` already uses.
  assert.deepEqual(docTypes.slice(0, 2), ["pointermove", "mousemove"]);
  assert.equal(docTypes.at(-2), "pointerup");
  assert.equal(docTypes.at(-1), "mouseup");
  assert.ok(
    docTypes.indexOf("mousemove") < docTypes.indexOf("mouseup"),
    "moves precede the release",
  );

  // Moves interpolate from the source center to the target center so a
  // threshold-based sensor sees real displacement.
  const moves = doc.dispatched.filter((e) => e.type === "mousemove");
  assert.deepEqual(
    { x: moves.at(-1).clientX, y: moves.at(-1).clientY },
    { x: 250, y: 250 },
  );
  assert.ok(moves[0].clientX > 50, "first move already left the start point");
  // A held button must be reported, or listeners treat the move as a hover.
  assert.equal(moves[0].buttons, 1);
});

test("drag presses the deepest node under the point, not the resolved container", async () => {
  // dnd-kit and friends attach listeners to an inner handle/card; events only
  // bubble upward, so pressing the container never reaches them.
  const handle = makeElement(rect(40, 40, 20, 20));
  const source = makeElement(rect(0, 0, 100, 100), { descendants: [handle] });
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot } = loadBridge({
    elements: { "#a": source, "#b": target },
    elementFromPoint: () => handle,
  });

  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" }, ...FAST });

  assert.deepEqual(types(handle).slice(0, 2), ["pointerdown", "mousedown"]);
  assert.ok(!types(source).includes("mousedown"), "container is not pressed directly");
  // The HTML5 sequence still belongs to the resolved source element.
  assert.ok(types(source).includes("dragstart"));
});

test("drag ignores an overlay covering the start point and presses the source", async () => {
  // A toast, backdrop or modal over the source is what elementFromPoint returns.
  // Pressing it would send the gesture somewhere unrelated while `drag` still
  // reported ok — the false green this action exists to remove.
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const overlay = makeElement(rect(0, 0, 800, 700));
  const { pilot } = loadBridge({
    elements: { "#a": source, "#b": target },
    elementFromPoint: () => overlay,
  });

  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" }, ...FAST });

  assert.ok(!types(overlay).includes("pointerdown"), "overlay is not pressed");
  assert.deepEqual(types(source).slice(0, 2), ["pointerdown", "mousedown"]);
});

test("drag falls back to the resolved source when nothing is hit-testable", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot } = loadBridge({
    elements: { "#a": source, "#b": target },
    elementFromPoint: () => null,
  });

  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" }, ...FAST });

  assert.deepEqual(types(source).slice(0, 2), ["pointerdown", "mousedown"]);
});

test("drag still emits the full HTML5 sequence in order", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot } = loadBridge({
    elements: { "#a": source, "#b": target },
  });

  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" }, ...FAST });

  assert.deepEqual(
    types(source).filter((t) => t.startsWith("drag")),
    ["dragstart", "dragleave", "dragend"],
  );
  assert.deepEqual(types(target), ["dragenter", "dragover", "drop"]);
});

test("drag reports whether an HTML5 handler claimed the drop", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const plain = makeElement(rect(200, 200, 100, 100));
  const claiming = makeElement(rect(200, 200, 100, 100), { cancelDrop: true });

  const first = loadBridge({ elements: { "#a": source, "#b": plain } });
  const ignored = await first.pilot.drag({
    source: { selector: "#a" },
    target: { selector: "#b" },
    ...FAST,
  });
  assert.equal(ignored.html5DropHandled, false);

  const second = loadBridge({ elements: { "#a": makeElement(rect(0, 0, 100, 100)), "#b": claiming } });
  const handled = await second.pilot.drag({
    source: { selector: "#a" },
    target: { selector: "#b" },
    ...FAST,
  });
  // `ok` only means the gesture was delivered; this flag is the one real signal
  // the bridge can observe about whether anything reacted.
  assert.equal(handled.ok, true);
  assert.equal(handled.html5DropHandled, true);
});

test("drag still emits pointer-typed events when the PointerEvent constructor is missing", async () => {
  // A WebView without `PointerEvent` must not silently lose the pointer stream:
  // dnd-kit's default sensor is PointerSensor, so mouse events alone would leave
  // the very libraries this gesture targets unable to activate. Listeners
  // dispatch by type string, so a MouseEvent typed `pointermove` still reaches
  // an addEventListener("pointermove", …).
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot, document: doc } = loadBridge({
    elements: { "#a": source, "#b": target },
    pointerEvents: false,
  });

  const result = await pilot.drag({
    source: { selector: "#a" },
    target: { selector: "#b" },
    steps: 2,
    ...FAST,
  });

  assert.equal(result.ok, true);
  assert.equal(types(doc).filter((t) => t === "pointermove").length, 2);
  assert.equal(types(doc).filter((t) => t === "mousemove").length, 2);
  const pointerMove = doc.dispatched.find((e) => e.type === "pointermove");
  assert.ok(pointerMove instanceof MouseEvent, "falls back to a MouseEvent, not a throw");
  assert.equal(pointerMove.pointerId, 1);
  assert.equal(pointerMove.pointerType, "mouse");
});

test("drag clamps a hostile or missing step count", async () => {
  const cases = [
    [undefined, 12],
    [0, 12],
    [-5, 12],
    ["nonsense", 12],
    [3.7, 3],
    [500, 60],
  ];

  for (const [steps, expected] of cases) {
    const source = makeElement(rect(0, 0, 100, 100));
    const target = makeElement(rect(200, 200, 100, 100));
    const { pilot, document: doc } = loadBridge({
      elements: { "#a": source, "#b": target },
    });

    const result = await pilot.drag({
      source: { selector: "#a" },
      target: { selector: "#b" },
      steps,
      ...FAST,
    });

    assert.equal(result.steps, expected, `steps=${steps}`);
    assert.equal(types(doc).filter((t) => t === "mousemove").length, expected);
  }
});

test("drag reports the gesture endpoints it used", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 300, 100, 100));
  const { pilot } = loadBridge({ elements: { "#a": source, "#b": target } });

  const result = await pilot.drag({
    source: { selector: "#a" },
    target: { selector: "#b" },
    ...FAST,
  });

  assert.deepEqual(result.from, { x: 50, y: 50 });
  assert.deepEqual(result.to, { x: 250, y: 350 });
});

test("drag does not pay a step delay after the last move", async () => {
  // The delay spaces moves apart, so N moves need N-1 gaps. Sleeping after the
  // last one only pushes the drop sequence back for nothing.
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot } = loadBridge({ elements: { "#a": source, "#b": target } });

  // Record what the gesture schedules rather than how long it takes. A wall
  // clock has no upper bound: a paused event loop would fail a correct run.
  const realSetTimeout = globalThis.setTimeout;
  const delays = [];
  globalThis.setTimeout = (fn, ms) => {
    delays.push(ms);
    return realSetTimeout(fn, 0);
  };
  try {
    await pilot.drag({
      source: { selector: "#a" },
      target: { selector: "#b" },
      steps: 3,
      stepDelayMs: 100,
      settleMs: 40,
    });
  } finally {
    globalThis.setTimeout = realSetTimeout;
  }

  // Two gaps for three moves, then the settle. A third 100 would mean the loop
  // slept after the last move.
  assert.deepEqual(delays, [100, 100, 40]);
});

test("drag waits between moves and settles after the release by default", async () => {
  const source = makeElement(rect(0, 0, 100, 100));
  const target = makeElement(rect(200, 200, 100, 100));
  const { pilot } = loadBridge({ elements: { "#a": source, "#b": target } });

  // Defaults: 12 steps x 16ms plus a 250ms settle. Async framework work (state
  // update, request, re-render) needs that beat before a caller asserts.
  const started = Date.now();
  await pilot.drag({ source: { selector: "#a" }, target: { selector: "#b" } });
  assert.ok(Date.now() - started >= 250, "gesture did not wait for the app to settle");
});
