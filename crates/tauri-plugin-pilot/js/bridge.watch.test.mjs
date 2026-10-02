// Dependency-free behavioural tests for bridge `watch` (#304).
//
// `textContent = ...` replaces an element's text node through a `childList`
// mutation. `watch` used to keep only element nodes from those mutations, so
// the change ended a `--require-mutation` wait and was then reported as
// "No DOM changes detected."
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.watch.test.mjs

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

const ELEMENT_NODE = 1;
const TEXT_NODE = 3;

function textNode(data) {
  return { nodeType: TEXT_NODE, textContent: data, parentElement: null };
}

function element(tagName, children = [], id = "") {
  const el = { nodeType: ELEMENT_NODE, tagName, id, className: "", childNodes: children };
  for (const child of children) child.parentElement = el;
  return el;
}

function loadBridge() {
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
  let observerCb;
  globalThis.Node = { ELEMENT_NODE, TEXT_NODE };
  globalThis.window = { fetch() {} };
  globalThis.document = { querySelector() { return null; }, body: {} };
  globalThis.MutationObserver = class {
    constructor(cb) {
      observerCb = cb;
    }
    observe() {}
    disconnect() {}
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return {
    pilot: globalThis.window.__PILOT__,
    fire(mutations) {
      observerCb(mutations);
    },
  };
}

// What `div.textContent = text` delivers: one childList record on the div
// that removes the old text node and adds the new one.
function replaceText(div, text) {
  const oldNode = div.childNodes[0];
  const newNode = textNode(text);
  newNode.parentElement = div;
  div.childNodes = [newNode];
  return {
    type: "childList",
    target: div,
    addedNodes: [newNode],
    removedNodes: oldNode ? [oldNode] : [],
  };
}

test("watch reports a textContent replacement as a text change on the parent (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV", [textNode("before")], "deferred-target");
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  fire([replaceText(div, "text-only")]);
  assert.deepEqual(await pending, {
    added: [],
    removed: [],
    modified: [{ tag: "div", text: "text-only" }],
    truncated: false,
  });
});

test("watch --require-mutation never resolves with an empty summary after a text-only change (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV", [textNode("before")]);
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  // Clearing the text removes the text node and adds nothing.
  div.childNodes = [];
  fire([{ type: "childList", target: div, addedNodes: [], removedNodes: [textNode("before")] }]);
  const changes = await pending;
  assert.ok(
    changes.added.length + changes.removed.length + changes.modified.length > 0,
    `empty summary: ${JSON.stringify(changes)}`,
  );
  assert.deepEqual(changes.modified, [{ tag: "div", text: "" }]);
});

test("watch reports text set on an empty element (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV");
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  fire([replaceText(div, "first")]);
  assert.deepEqual((await pending).modified, [{ tag: "div", text: "first" }]);
});

test("watch records one text change per target per batch (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV", [textNode("a")]);
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  fire([replaceText(div, "b"), replaceText(div, "c")]);
  assert.deepEqual((await pending).modified, [{ tag: "div", text: "c" }]);
});

test("watch --require-mutation keeps waiting past mutations it does not report (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV");
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  // Blank text on an empty element, then a comment node (e.g. Vue's
  // `<!--v-if-->`): neither yields a summary entry.
  fire([replaceText(div, "   ")]);
  fire([{ type: "childList", target: div, addedNodes: [{ nodeType: 8, textContent: "v-if" }], removedNodes: [] }]);
  const early = await Promise.race([
    pending.then((changes) => JSON.stringify(changes)),
    new Promise((resolve) => setTimeout(() => resolve("pending"), 20)),
  ]);
  assert.equal(early, "pending", "watch settled on mutations it does not report");
  fire([replaceText(div, "ready")]);
  assert.deepEqual(await pending, {
    added: [],
    removed: [],
    modified: [{ tag: "div", text: "ready" }],
    truncated: false,
  });
});

test("watch --require-mutation times out when only unreported mutations happen (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV");
  const pending = pilot.watch({ timeout: 30, stable: 0, requireMutation: true });
  fire([replaceText(div, "   ")]);
  await assert.rejects(pending, /watch timeout: no DOM changes within 30ms/);
});

// Resolves to the summary, or to "timeout" when `ms` passes first. The watch
// timeout resolves too once anything was recorded, so these tests use a long
// watch timeout and a short race to prove the stable timer armed.
function settleWithin(pending, ms) {
  return Promise.race([pending, new Promise((resolve) => setTimeout(() => resolve("timeout"), ms))]);
}

for (const [kind, record] of [
  ["an added element", (div) => ({ type: "childList", target: div, addedNodes: [element("SPAN")], removedNodes: [] })],
  ["a removed element", (div) => ({ type: "childList", target: div, addedNodes: [], removedNodes: [element("SPAN")] })],
  ["a text change", (div) => replaceText(div, "x")],
]) {
  test(`watch --require-mutation settles on the stable window after ${kind} (#304)`, async () => {
    const { pilot, fire } = loadBridge();
    const div = element("DIV");
    const pending = pilot.watch({ timeout: 5000, stable: 0, requireMutation: true });
    fire([record(div)]);
    assert.notEqual(await settleWithin(pending, 200), "timeout");
  });
}

test("watch without --require-mutation restarts the stable window on unreported mutations", async () => {
  const { pilot, fire } = loadBridge();
  const div = element("DIV");
  const pending = pilot.watch({ timeout: 5000, stable: 100 });
  await new Promise((resolve) => setTimeout(resolve, 60));
  fire([replaceText(div, "   ")]);
  // Without the restart the window would close at 100ms.
  assert.equal(await settleWithin(pending, 70), "timeout");
  assert.deepEqual(await pending, { added: [], removed: [], modified: [], truncated: false });
});

test("watch reports only the target's own text when it mixes text and element children (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const oldChildren = [textNode("Count: "), element("B", [textNode("3")]), textNode(" items")];
  const newChildren = [textNode("Count: "), element("B", [textNode("4")]), textNode(" items")];
  const p = element("P", newChildren);
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  // `p.innerHTML = "Count: <b>4</b> items"` over "Count: <b>3</b> items".
  fire([{ type: "childList", target: p, addedNodes: newChildren, removedNodes: oldChildren }]);
  assert.deepEqual(await pending, {
    added: [{ tag: "b", text: "4" }],
    removed: [{ tag: "b", text: "3" }],
    modified: [{ tag: "p", text: "Count: items" }],
    truncated: false,
  });
});

test("watch still reports added and removed elements, not their parent's text", async () => {
  const { pilot, fire } = loadBridge();
  const span = element("SPAN", [textNode("x")], "added-span");
  const old = element("P", [textNode("gone")]);
  const div = element("DIV", [span]);
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  fire([{ type: "childList", target: div, addedNodes: [span], removedNodes: [old] }]);
  assert.deepEqual(await pending, {
    added: [{ tag: "span", id: "added-span", text: "x" }],
    removed: [{ tag: "p", text: "gone" }],
    modified: [],
    truncated: false,
  });
});

test("watch ignores whitespace-only text nodes around added elements (#304)", async () => {
  const { pilot, fire } = loadBridge();
  const li = element("LI", [textNode("a")]);
  const ul = element("UL", [textNode("\n  "), li, textNode("\n")]);
  const pending = pilot.watch({ timeout: 1000, stable: 0, requireMutation: true });
  // `ul.innerHTML = "\n  <li>a</li>\n"` on formatted markup.
  fire([{ type: "childList", target: ul, addedNodes: ul.childNodes, removedNodes: [] }]);
  assert.deepEqual(await pending, {
    added: [{ tag: "li", text: "a" }],
    removed: [],
    modified: [],
    truncated: false,
  });
});
