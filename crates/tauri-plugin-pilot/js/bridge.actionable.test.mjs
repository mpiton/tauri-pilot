// Dependency-free behavioural tests for the bridge actionability guard (#324).
//
// `click`, `fill`, `type` and `select` used to act on disabled and readonly
// controls and report ok: the value changed, the events fired and a disabled
// button's onclick ran. A user can do none of that, so a test driving a
// disabled form passed while the real app blocks it. Each action must now fail
// naming the reason and leave the element untouched, with no event fired.
//
// Like Playwright's actionability checks, `aria-disabled="true"` on the
// target or an ancestor (the nearest explicit value wins) counts as disabled.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.actionable.test.mjs

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

const FORM_CONTROLS = new Set(["BUTTON", "INPUT", "SELECT", "TEXTAREA"]);

// Element mock. `matches(":disabled")` follows the HTML rules the guard relies
// on: a form control or <fieldset> with its own `disabled`, or inside a
// disabled <fieldset>; an <option> with its own `disabled`, or inside a
// disabled <optgroup>. Only `:disabled` is supported.
class El {
  constructor(tag, props = {}, children = []) {
    this.tagName = tag.toUpperCase();
    this.nodeType = 1;
    this._attrs = props.attrs || {};
    this.disabled = props.disabled === true;
    if ("readOnly" in props) this.readOnly = props.readOnly;
    this._value = props.value != null ? String(props.value) : "";
    this.text = props.text || "";
    this.selected = props.selected === true;
    this.parentElement = null;
    this.children = children;
    this.events = [];
    this.focused = false;
    this.clicked = 0;
    for (const child of children) child.parentElement = this;
  }
  get value() {
    if (this.tagName !== "SELECT") return this._value;
    const sel = this.options.find((o) => o.selected);
    return sel ? sel.value : "";
  }
  set value(v) {
    if (this.tagName !== "SELECT") {
      this._value = String(v);
      return;
    }
    let hit = false;
    for (const o of this.options) {
      o.selected = !hit && o.value === String(v);
      if (o.selected) hit = true;
    }
  }
  get options() {
    const out = [];
    for (const child of this.children) {
      if (child.tagName === "OPTION") out.push(child);
      if (child.tagName === "OPTGROUP") out.push(...child.children);
    }
    return out;
  }
  getAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this._attrs, name) ? this._attrs[name] : null;
  }
  hasAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this._attrs, name);
  }
  matches(selector) {
    if (selector !== ":disabled") throw new SyntaxError("unsupported selector: " + selector);
    if (this.tagName === "OPTION") {
      return this.disabled || (this.parentElement?.tagName === "OPTGROUP" && this.parentElement.disabled);
    }
    if (!FORM_CONTROLS.has(this.tagName) && this.tagName !== "FIELDSET") return false;
    if (this.disabled) return true;
    for (let node = this.parentElement; node; node = node.parentElement) {
      if (node.tagName === "FIELDSET" && node.disabled) return true;
    }
    return false;
  }
  getBoundingClientRect() {
    return { left: 0, top: 0, width: 10, height: 10 };
  }
  focus() {
    this.focused = true;
  }
  click() {
    this.clicked += 1;
  }
  dispatchEvent(event) {
    this.events.push(event.type);
    return true;
  }
}

function loadBridge(target) {
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
  class FakeEvent {
    constructor(type, init) {
      this.type = type;
      Object.assign(this, init || {});
    }
  }
  globalThis.MouseEvent = class MouseEvent extends FakeEvent {};
  globalThis.PointerEvent = class PointerEvent extends FakeEvent {};
  globalThis.KeyboardEvent = class KeyboardEvent extends FakeEvent {};
  globalThis.InputEvent = class InputEvent extends FakeEvent {};
  globalThis.Event = class Event extends FakeEvent {};
  globalThis.window = { fetch() {} };
  globalThis.document = {
    querySelector() {
      return target;
    },
    elementFromPoint() {
      return target;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function assertUntouched(el, value) {
  assert.deepEqual(el.events, [], "no event fired");
  assert.equal(el.focused, false, "not focused");
  assert.equal(el.clicked, 0, "not clicked");
  if (value !== undefined) assert.equal(el.value, value, "value unchanged");
}

test("click on a disabled button fails and fires no event", () => {
  const btn = new El("button", { disabled: true });
  const pilot = loadBridge(btn);
  assert.throws(() => pilot.click({ selector: "#tmp-btn" }), /^Error: click: target is disabled$/);
  assertUntouched(btn);
});

test("click on a button inside a disabled fieldset fails", () => {
  const btn = new El("button");
  new El("fieldset", { disabled: true }, [new El("div", {}, [btn])]);
  const pilot = loadBridge(btn);
  assert.throws(() => pilot.click({ selector: "button" }), /^Error: click: target is disabled$/);
  assertUntouched(btn);
});

test("click on a child of a disabled button fails, by selector and by point", () => {
  // The synthetic click would bubble from the child to the button and run its
  // onclick; a user's click there does nothing.
  for (const params of [{ selector: "#submit span" }, { x: 5, y: 5 }]) {
    const span = new El("span");
    const btn = new El("button", { disabled: true }, [new El("svg", {}, [span])]);
    assert.throws(() => loadBridge(span).click(params), /^Error: click: target is disabled$/);
    assertUntouched(span);
    assertUntouched(btn);
  }
});

test("click on a child of an enabled button or of a disabled fieldset still clicks", () => {
  // A disabled <fieldset> blocks its controls, not a plain child element.
  const inButton = new El("span");
  new El("button", {}, [inButton]);
  assert.deepEqual(loadBridge(inButton).click({ selector: "span" }), { ok: true });
  assert.ok(inButton.events.includes("click"));

  const inFieldset = new El("span");
  new El("fieldset", { disabled: true }, [inFieldset]);
  assert.deepEqual(loadBridge(inFieldset).click({ selector: "span" }), { ok: true });
  assert.ok(inFieldset.events.includes("click"));
});

test("click on an enabled button still dispatches the click", () => {
  const btn = new El("button");
  const pilot = loadBridge(btn);
  assert.deepEqual(pilot.click({ selector: "button" }), { ok: true });
  assert.ok(btn.events.includes("click"));
});

test("click on an aria-disabled non-native control fails", () => {
  const div = new El("div", { attrs: { role: "button", "aria-disabled": "true" } });
  const pilot = loadBridge(div);
  assert.throws(() => pilot.click({ selector: "[role=button]" }), /^Error: click: target is disabled$/);
  assertUntouched(div);
});

test("click fails inside an aria-disabled ancestor unless a nearer one says false", () => {
  const blocked = new El("div", { attrs: { role: "menuitem" } });
  new El("div", { attrs: { role: "menu", "aria-disabled": "true" } }, [blocked]);
  assert.throws(() => loadBridge(blocked).click({ selector: "x" }), /^Error: click: target is disabled$/);
  assertUntouched(blocked);

  const reopened = new El("div", { attrs: { role: "menuitem", "aria-disabled": "false" } });
  new El("div", { attrs: { "aria-disabled": "true" } }, [reopened]);
  assert.deepEqual(loadBridge(reopened).click({ selector: "x" }), { ok: true });
  assert.ok(reopened.events.includes("click"));
});

test("click reads aria-disabled through a role list or a capitalised role", () => {
  // WAI-ARIA role attributes are token lists; the role is matched without case.
  for (const role of ["button link", "Button", " button "]) {
    const div = new El("div", { attrs: { role, "aria-disabled": "true" } });
    assert.throws(() => loadBridge(div).click({ selector: "x" }), /^Error: click: target is disabled$/, role);
    assertUntouched(div);
  }
});

test("click ignores aria-disabled on a target whose role does not support it", () => {
  // Playwright reads aria-disabled only for roles that support it.
  const plain = new El("div");
  new El("div", { attrs: { "aria-disabled": "true" } }, [plain]);
  assert.deepEqual(loadBridge(plain).click({ selector: "x" }), { ok: true });
  assert.ok(plain.events.includes("click"));
});

test("fill on a disabled input or textarea fails and leaves the value", () => {
  for (const tag of ["input", "textarea"]) {
    const el = new El(tag, { disabled: true, value: "orig" });
    const pilot = loadBridge(el);
    assert.throws(() => pilot.fill({ selector: tag, value: "changed" }), /^Error: fill: target is disabled$/);
    assertUntouched(el, "orig");
  }
});

test("fill and type on a readonly input or textarea fail and leave the value", () => {
  for (const tag of ["input", "textarea"]) {
    const el = new El(tag, { readOnly: true, value: "ro" });
    const pilot = loadBridge(el);
    assert.throws(() => pilot.fill({ selector: "#tmp-ro", value: "changed" }), /^Error: fill: target is readonly$/);
    assert.throws(() => pilot.type({ selector: "#tmp-ro", text: "X" }), /^Error: type: target is readonly$/);
    assertUntouched(el, "ro");
  }
});

test("fill and type on an aria-readonly textbox host fail", () => {
  // A rich-text editor in read mode keeps its contenteditable host and sets
  // aria-readonly, as Playwright's editable check reads it.
  const host = new El("div", { attrs: { role: "textbox", "aria-readonly": "true" } });
  host.isContentEditable = true;
  host.contentEditable = "true";
  host.textContent = "orig";
  const pilot = loadBridge(host);
  assert.throws(() => pilot.fill({ selector: "x", value: "new" }), /^Error: fill: target is readonly$/);
  assert.throws(() => pilot.type({ selector: "x", text: "new" }), /^Error: type: target is readonly$/);
  assert.equal(host.textContent, "orig");
  assertUntouched(host);
});

test("fill on a <select> ignores aria-readonly, as a native control", () => {
  // Like Playwright: a native control reads only its native readonly state.
  const sel = makeSelect({ attrs: { "aria-readonly": "true" } }, [
    new El("option", { value: "a", text: "a" }),
    new El("option", { value: "b", text: "b" }),
  ]);
  assert.deepEqual(loadBridge(sel).fill({ selector: "select", value: "b" }), { ok: true });
  assert.equal(sel.value, "b");
});

test("fill still writes a textbox host with aria-readonly false", () => {
  const host = new El("div", { attrs: { role: "textbox", "aria-readonly": "false" } });
  host.isContentEditable = true;
  host.contentEditable = "true";
  host.textContent = "orig";
  assert.deepEqual(loadBridge(host).fill({ selector: "x", value: "new" }), { ok: true });
  assert.equal(host.textContent, "new");
});

test("type on a disabled textarea fails and leaves the value", () => {
  const el = new El("textarea", { disabled: true, value: "t" });
  const pilot = loadBridge(el);
  assert.throws(() => pilot.type({ selector: "#tmp-ta", text: "X" }), /^Error: type: target is disabled$/);
  assertUntouched(el, "t");
});

test("fill on an input inside a disabled fieldset fails", () => {
  const el = new El("input", { value: "orig" });
  new El("fieldset", { disabled: true }, [el]);
  const pilot = loadBridge(el);
  assert.throws(() => pilot.fill({ selector: "input", value: "x" }), /^Error: fill: target is disabled$/);
  assertUntouched(el, "orig");
});

test("fill and type on an aria-disabled contenteditable host fail", () => {
  const host = new El("div", { attrs: { role: "textbox", "aria-disabled": "true" } });
  host.isContentEditable = true;
  host.contentEditable = "true";
  host.textContent = "orig";
  const pilot = loadBridge(host);
  assert.throws(() => pilot.fill({ selector: "x", value: "new" }), /^Error: fill: target is disabled$/);
  assert.throws(() => pilot.type({ selector: "x", text: "new" }), /^Error: type: target is disabled$/);
  assert.equal(host.textContent, "orig");
  assertUntouched(host);
});

test("fill writes an input type whose readonly HTML ignores", () => {
  // HTML ignores `readonly` on a range input: a user can still drag it.
  const el = new El("input", { attrs: { type: "range", readonly: "" }, readOnly: true, value: "1" });
  const pilot = loadBridge(el);
  assert.deepEqual(pilot.fill({ selector: "input", value: "5" }), { ok: true });
  assert.equal(el.value, "5");
});

test("fill and type still write an enabled, editable input", () => {
  const el = new El("input", { readOnly: false, value: "a" });
  const pilot = loadBridge(el);
  assert.deepEqual(pilot.type({ selector: "input", text: "b" }), { ok: true });
  assert.equal(el.value, "ab");
  assert.deepEqual(pilot.fill({ selector: "input", value: "c" }), { ok: true });
  assert.equal(el.value, "c");
});

function makeSelect(props, children) {
  const sel = new El("select", props, children);
  sel.options[0].selected = true;
  return sel;
}

test("select on a disabled <select> fails and keeps the selection", () => {
  const sel = makeSelect({ disabled: true }, [
    new El("option", { value: "a", text: "a" }),
    new El("option", { value: "b", text: "b" }),
  ]);
  const pilot = loadBridge(sel);
  assert.throws(() => pilot.select({ selector: "#tmp-sel", value: "b" }), /^Error: select: target is disabled$/);
  assertUntouched(sel, "a");
});

test("select and fill reject a disabled option and keep the selection", () => {
  const sel = makeSelect({}, [
    new El("option", { value: "user", text: "User" }),
    new El("option", { value: "locked", text: "Locked", disabled: true }),
  ]);
  const pilot = loadBridge(sel);
  assert.throws(
    () => pilot.select({ selector: "select[name=role]", value: "locked" }),
    /^Error: select: option "locked" is disabled$/,
  );
  assert.throws(
    () => pilot.fill({ selector: "select[name=role]", value: "Locked" }),
    /^Error: fill: option "Locked" is disabled$/,
  );
  assertUntouched(sel, "user");
});

test("select rejects an option inside a disabled optgroup", () => {
  const sel = makeSelect({}, [
    new El("option", { value: "user", text: "User" }),
    new El("optgroup", { disabled: true }, [new El("option", { value: "root", text: "Root" })]),
  ]);
  const pilot = loadBridge(sel);
  assert.throws(() => pilot.select({ selector: "select", value: "root" }), /^Error: select: option "root" is disabled$/);
  assert.equal(sel.value, "user");
  assert.deepEqual(sel.events, []);
});

test("select on a multi-select names every disabled option", () => {
  const sel = makeSelect({}, [
    new El("option", { value: "a", text: "a" }),
    new El("option", { value: "b", text: "b", disabled: true }),
    new El("option", { value: "c", text: "c", disabled: true }),
  ]);
  sel.multiple = true;
  const pilot = loadBridge(sel);
  assert.throws(
    () => pilot.select({ selector: "select", value: ["a", "b", "c"] }),
    /^Error: select: options "b", "c" are disabled$/,
  );
  assert.deepEqual(sel.options.map((o) => o.selected), [true, false, false]);
});

test("check on a disabled checkbox names the reason and does not click", () => {
  const cb = new El("input", { attrs: { type: "checkbox" }, disabled: true });
  cb.type = "checkbox";
  cb.checked = false;
  const pilot = loadBridge(cb);
  assert.throws(() => pilot.check({ selector: "#tmp-cb" }), /^Error: check: target is disabled$/);
  assertUntouched(cb);
  assert.equal(cb.checked, false);
});
