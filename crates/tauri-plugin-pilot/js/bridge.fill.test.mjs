// Dependency-free behavioural tests for bridge `fill` / `type` (#154).
//
// Those actions used to assign `el.value` on any target. On a <div> that
// creates an expando and reports ok while the visible text is unchanged.
// They must reject non-editable targets and actually edit contenteditable hosts.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.fill.test.mjs

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

function makeValueEl(tag, value = "") {
  const proto = {};
  Object.defineProperty(proto, "value", {
    get() { return this._value; },
    set(v) { this._value = String(v); },
  });
  const el = Object.create(proto);
  el.tagName = tag;
  el._value = value;
  el.events = [];
  el.focus = function () {};
  el.dispatchEvent = function (event) {
    this.events.push(event.type);
    return true;
  };
  return el;
}

function makeHost({
  tag = "DIV",
  editable = false,
  text = "original text",
  isContentEditable,
  contentEditable,
  ownerDocument,
} = {}) {
  const el = {
    tagName: tag,
    textContent: text,
    isContentEditable: isContentEditable ?? editable,
    contentEditable: contentEditable ?? (editable ? "true" : "false"),
    events: [],
    focus() {},
    dispatchEvent(event) {
      this.events.push(event.type);
      return true;
    },
  };
  if (ownerDocument) el.ownerDocument = ownerDocument;
  return el;
}

function makeEditingDocument(execCommand, onSelectNodeContents) {
  const selection = {
    removeAllRanges() {},
    addRange() {},
  };
  const view = { getSelection() { return selection; } };
  return {
    createRange() {
      return {
        selectNodeContents(node) {
          if (onSelectNodeContents) onSelectNodeContents(node);
        },
        collapse() {},
      };
    },
    defaultView: view,
    execCommand,
  };
}

function loadBridge({ queryResult, execCommand, onSelectNodeContents } = {}) {
  Object.assign(console, REAL_CONSOLE);
  class FakeEvent {
    constructor(type, init) {
      this.type = type;
      Object.assign(this, init || {});
    }
  }
  globalThis.KeyboardEvent = class KeyboardEvent extends FakeEvent {};
  globalThis.InputEvent = class InputEvent extends FakeEvent {};
  const selection = {
    removeAllRanges() {},
    addRange() {},
  };
  globalThis.window = {
    fetch() {},
    getSelection() { return selection; },
  };
  globalThis.document = {
    querySelector(selector) {
      if (queryResult === undefined) {
        throw new Error("unexpected querySelector(" + selector + ")");
      }
      return queryResult;
    },
    createRange() {
      return {
        selectNodeContents(node) {
          if (onSelectNodeContents) onSelectNodeContents(node);
        },
        collapse() {},
      };
    },
    defaultView: globalThis.window,
  };
  if (execCommand) globalThis.document.execCommand = execCommand;
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("fill sets value on input and textarea", () => {
  for (const tag of ["INPUT", "TEXTAREA"]) {
    const el = makeValueEl(tag);
    const pilot = loadBridge({ queryResult: el });
    assert.deepEqual(pilot.fill({ selector: tag.toLowerCase(), value: "x" }), { ok: true });
    assert.equal(el.value, "x");
    assert.ok(el.events.includes("input") && el.events.includes("change"));
  }
});

test("fill and type throw on a non-editable target", () => {
  const el = makeHost();
  const pilot = loadBridge({ queryResult: el });
  assert.throws(() => pilot.fill({ selector: "#qa-div", value: "SHOULD-FAIL" }), /contenteditable/i);
  assert.throws(() => pilot.type({ selector: "#qa-div", text: "SHOULD-FAIL" }), /contenteditable/i);
  assert.equal(el.textContent, "original text");
  assert.equal(el.value, undefined);
});

test("fill replaces contenteditable text when execCommand is unavailable", () => {
  const el = makeHost({ editable: true });
  const pilot = loadBridge({ queryResult: el });
  assert.deepEqual(pilot.fill({ selector: "#editor", value: "hello" }), { ok: true });
  assert.equal(el.textContent, "hello");
  assert.ok(el.events.includes("input") && el.events.includes("change"));
});

test("fill uses insertText on contenteditable when execCommand works", () => {
  const calls = [];
  const selected = [];
  const el = makeHost({ editable: true });
  const execCommand = (cmd, _ui, value) => {
    calls.push([cmd, value]);
    if (cmd === "insertText") el.textContent = value;
    return true;
  };
  const pilot = loadBridge({
    queryResult: el,
    execCommand,
    onSelectNodeContents: (node) => selected.push(node),
  });
  assert.deepEqual(pilot.fill({ selector: "#editor", value: "hello" }), { ok: true });
  assert.deepEqual(calls, [["insertText", "hello"]]);
  assert.deepEqual(selected, [el]);
  assert.equal(el.textContent, "hello");
});

test("type uses insertText on contenteditable when execCommand works", () => {
  const calls = [];
  const el = makeHost({ editable: true, text: "ab" });
  const execCommand = (cmd, _ui, value) => {
    calls.push([cmd, value]);
    if (cmd === "insertText") el.textContent = (el.textContent || "") + value;
    return true;
  };
  const pilot = loadBridge({ queryResult: el, execCommand });
  assert.deepEqual(pilot.type({ selector: "#editor", text: "cd" }), { ok: true });
  assert.deepEqual(calls, [["insertText", "c"], ["insertText", "d"]]);
  assert.equal(el.textContent, "abcd");
});

test("fill and type accept plaintext-only and IDL-true contenteditable hosts", () => {
  for (const hostOpts of [
    { isContentEditable: false, contentEditable: "plaintext-only" },
    { isContentEditable: false, contentEditable: "true" },
  ]) {
    const filled = makeHost({ text: "ab", ...hostOpts });
    const fillPilot = loadBridge({ queryResult: filled });
    assert.deepEqual(fillPilot.fill({ selector: "#editor", value: "hello" }), { ok: true });
    assert.equal(filled.textContent, "hello");

    const typed = makeHost({ text: "ab", ...hostOpts });
    const typePilot = loadBridge({ queryResult: typed });
    assert.deepEqual(typePilot.type({ selector: "#editor", text: "c" }), { ok: true });
    assert.equal(typed.textContent, "abc");
  }
});

test("fill and type run insertText on the target ownerDocument", () => {
  const parentCalls = [];
  const ownerCalls = [];
  const ownerDoc = makeEditingDocument((cmd, _ui, value) => {
    ownerCalls.push([cmd, value]);
    return true;
  });
  const el = makeHost({ editable: true, text: "ab", ownerDocument: ownerDoc });
  const parentExec = (cmd, _ui, value) => {
    parentCalls.push([cmd, value]);
    return true;
  };

  const fillPilot = loadBridge({ queryResult: el, execCommand: parentExec });
  assert.deepEqual(fillPilot.fill({ selector: "#editor", value: "hello" }), { ok: true });
  assert.deepEqual(parentCalls, []);
  assert.deepEqual(ownerCalls, [["insertText", "hello"]]);

  parentCalls.length = 0;
  ownerCalls.length = 0;
  const typed = makeHost({ editable: true, text: "ab", ownerDocument: ownerDoc });
  const typePilot = loadBridge({ queryResult: typed, execCommand: parentExec });
  assert.deepEqual(typePilot.type({ selector: "#editor", text: "c" }), { ok: true });
  assert.deepEqual(parentCalls, []);
  assert.deepEqual(ownerCalls, [["insertText", "c"]]);
});

test("type appends on input and contenteditable", () => {
  const input = makeValueEl("INPUT", "ab");
  const pilotIn = loadBridge({ queryResult: input });
  assert.deepEqual(pilotIn.type({ selector: "input", text: "c" }), { ok: true });
  assert.equal(input.value, "abc");

  const host = makeHost({ editable: true, text: "ab" });
  const pilotEd = loadBridge({ queryResult: host });
  assert.deepEqual(pilotEd.type({ selector: "#editor", text: "c" }), { ok: true });
  assert.equal(host.textContent, "abc");
});
