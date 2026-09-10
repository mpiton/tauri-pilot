// Dependency-free behavioural tests for the bridge `screenshot` action (#129).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. `htmlToImage.toPng` is mocked to capture the
// node and options it receives: #129 is about the options (without explicit
// width/height, html-to-image crops `document.documentElement` to the viewport
// and silently loses everything below the fold).
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.screenshot.test.mjs

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

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
// Returns the pilot API plus the toPng calls captured by the mock.
function loadBridge({ queryResult, documentElement, body } = {}) {
  Object.assign(console, REAL_CONSOLE);

  const calls = [];
  globalThis.htmlToImage = {
    async toPng(node, options) {
      calls.push({ node, options });
      return "data:image/png;base64,AAAA";
    },
  };

  globalThis.window = { fetch() {} };
  globalThis.document = {
    documentElement: documentElement || {},
    body: body || {},
    querySelector(selector) {
      if (queryResult === undefined) {
        throw new Error("unexpected querySelector(" + selector + ")");
      }
      return queryResult;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;

  // Indirect eval runs in global scope; the IIFE then resolves bare `window`,
  // `document`, etc. against the globals set above.
  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, calls };
}

test("screenshot without selector captures the full document height", async () => {
  // Document taller and wider than the viewport: scroll dimensions are the
  // full layout size, client dimensions are the viewport crop we must avoid.
  const documentElement = {
    scrollWidth: 800,
    scrollHeight: 2400,
    clientWidth: 800,
    clientHeight: 700,
  };
  const body = { scrollWidth: 820, scrollHeight: 2500 };
  const { pilot, calls } = loadBridge({ documentElement, body });

  await pilot.screenshot({});

  assert.equal(calls.length, 1);
  assert.equal(calls[0].node, documentElement);
  // max(html, body) on both axes so nothing below the fold is lost.
  assert.equal(calls[0].options.width, 820);
  assert.equal(calls[0].options.height, 2500);
  assert.equal(calls[0].options.pixelRatio, 1);
});

test("screenshot without selector tolerates a missing body", async () => {
  const documentElement = { scrollWidth: 800, scrollHeight: 2400 };
  const { pilot, calls } = loadBridge({ documentElement, body: null });

  await pilot.screenshot({});

  assert.equal(calls[0].options.width, 800);
  assert.equal(calls[0].options.height, 2400);
});

test("screenshot with selector does not override element dimensions", async () => {
  const el = { scrollWidth: 300, scrollHeight: 40 };
  const { pilot, calls } = loadBridge({ queryResult: el });

  await pilot.screenshot({ selector: "#panel" });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].node, el);
  // html-to-image must size the capture from the element's own layout.
  assert.equal("width" in calls[0].options, false);
  assert.equal("height" in calls[0].options, false);
  assert.equal(calls[0].options.pixelRatio, 1);
  // #146 hits element captures too — the CLI and the MCP tool both take this
  // path for `--selector`, so the curated list must be passed here as well.
  assert.ok(Array.isArray(calls[0].options.includeStyleProperties));
});

test("screenshot with unmatched selector throws element not found", async () => {
  const { pilot } = loadBridge({ queryResult: null });

  await assert.rejects(
    () => pilot.screenshot({ selector: "#missing" }),
    /Element not found: #missing/,
  );
});

// #146: html-to-image's per-property clone path (taken whenever
// `getComputedStyle(el).cssText` is empty — WebKit and Blink both) copies every
// name returned by `getComputedStyle(document.documentElement)` onto every
// cloned element. On a Tailwind v4 page that list is ~2200 names, ~1760 of them
// custom properties, and WebKit re-serializes the whole style attribute on each
// `setProperty` — quadratic, minutes of wedged webview. The bridge must hand
// html-to-image a curated list instead.
test("screenshot restricts the style properties html-to-image copies", async () => {
  const documentElement = { scrollWidth: 800, scrollHeight: 600 };
  const { pilot, calls } = loadBridge({ documentElement, body: null });

  await pilot.screenshot({});

  const props = calls[0].options.includeStyleProperties;
  assert.ok(Array.isArray(props), "includeStyleProperties must be passed");
  // A few hundred names is already quadratic pain on WebKit; keep it small.
  assert.ok(props.length < 200, `expected a curated list, got ${props.length}`);
  assert.equal(props.filter((p) => p.startsWith("--")).length, 0);
  // Anything the page actually paints has to survive the clone. At least one
  // name per block of STYLE_PROPERTIES, so deleting a whole category fails
  // here instead of silently flattening every capture.
  for (const required of [
    "display", // box + layout
    "content-visibility", // ... and the two that hide a subtree
    "clip",
    "flex-direction", // flex + grid
    "background-color", // background + border
    "border-top-width",
    "border-image-source",
    "transform", // paint effects
    "transform-origin",
    "mask-size",
    "color", // text
    "font-family",
    "content",
    "-webkit-text-security", // author-masked fields must stay masked
    "fill", // replaced content + SVG
    "text-anchor",
    "appearance", // form controls
  ]) {
    assert.ok(props.includes(required), `missing ${required}`);
  }
  assert.equal(new Set(props).size, props.length, "duplicate property names");
});

// The whole fix rests on one expression in the vendored bundle:
// `t.includeStyleProperties ? ... : f(getComputedStyle(documentElement))`.
// html-to-image silently ignores options it does not know, so a version bump
// that renames or drops it would put the ~2200-name enumeration back on every
// cloned element with the suite still green. Pin the option to the artifact.
test("the vendored html-to-image still reads includeStyleProperties", () => {
  const vendor = readFileSync(
    join(here, "vendor", "html-to-image.iife.js"),
    "utf8",
  );
  // `includeStyleProperties?` rather than the bare name: the option has to be
  // *read as a condition*, which is the shape of the expression that picks it
  // over the full enumeration. The minifier renames the parameter, never the
  // property, so this survives a rebuild of the same upstream code and fails
  // if a version bump drops the branch.
  assert.ok(
    vendor.includes("includeStyleProperties?"),
    "vendored html-to-image no longer reads includeStyleProperties as a " +
      "condition: re-check scripts/build-html-to-image.sh against the " +
      "upstream pin (#146)",
  );
});
