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

function makeHost({ tag = "DIV", editable = false, text = "original text" } = {}) {
  return {
    tagName: tag,
    textContent: text,
    isContentEditable: editable,
    contentEditable: editable ? "true" : "false",
    events: [],
    focus() {},
    dispatchEvent(event) {
      this.events.push(event.type);
      return true;
    },
  };
}

function loadBridge({ queryResult, execCommand } = {}) {
  Object.assign(console, REAL_CONSOLE);
  class FakeEvent {
    constructor(type, init) {
      this.type = type;
      Object.assign(this, init || {});
    }
  }
  globalThis.KeyboardEvent = class KeyboardEvent extends FakeEvent {};
  globalThis.InputEvent = class InputEvent extends FakeEvent {};
  globalThis.window = { fetch() {} };
  globalThis.document = {
    querySelector(selector) {
      if (queryResult === undefined) {
        throw new Error("unexpected querySelector(" + selector + ")");
      }
      return queryResult;
    },
  };
  if (execCommand) globalThis.document.execCommand = execCommand;
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("fill sets value on input, textarea, and select", () => {
  for (const tag of ["INPUT", "TEXTAREA", "SELECT"]) {
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
  const el = makeHost({ editable: true });
  const execCommand = (cmd, _ui, value) => {
    calls.push([cmd, value]);
    if (cmd === "insertText") el.textContent = value;
    return true;
  };
  const pilot = loadBridge({ queryResult: el, execCommand });
  assert.deepEqual(pilot.fill({ selector: "#editor", value: "hello" }), { ok: true });
  assert.deepEqual(calls, [["selectAll", undefined], ["insertText", "hello"]]);
  assert.equal(el.textContent, "hello");
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
