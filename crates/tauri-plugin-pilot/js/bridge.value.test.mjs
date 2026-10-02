// Dependency-free behavioural tests for bridge `value` / `snapshot` on
// `<select multiple>` (#158).
//
// `HTMLSelectElement.value` is the first selected option only. `forms.dump`
// already walks selected options; `value` and `snapshot` used the IDL
// property, so a multi-select with `rust` and `js` both selected reported
// only `rust`. The three commands must agree: `value` and `snapshot` join
// selected option values with `", "`, matching the `forms` CLI display.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.value.test.mjs

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

function attrs(el) {
  return {
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(el._attrs, name)
        ? el._attrs[name]
        : null;
    },
    hasAttribute(name) {
      return Object.prototype.hasOwnProperty.call(el._attrs, name);
    },
  };
}

// Spec-faithful `.value`: first selected option, or "" if none. That is the
// IDL getter the bug was reading. Selected options live on `.options`.
function makeSelect({ multiple, options, name = "skills" }) {
  const opts = options.map((o) => ({ value: o.value, selected: !!o.selected }));
  const el = {
    tagName: "SELECT",
    nodeType: 1,
    children: [],
    textContent: opts.map((o) => o.value).join(" "),
    name,
    type: multiple ? "select-multiple" : "select-one",
    multiple: !!multiple,
    _attrs: { name },
    get options() {
      return opts;
    },
    get value() {
      const sel = opts.find((o) => o.selected);
      return sel ? sel.value : "";
    },
  };
  return Object.assign(el, attrs(el));
}

function makeForm(fields) {
  return {
    tagName: "FORM",
    id: "",
    name: "",
    action: "",
    method: "get",
    getAttribute(name) {
      return this[name] || "";
    },
    querySelectorAll() {
      return fields;
    },
  };
}

function makeBody(children) {
  return {
    tagName: "BODY",
    nodeType: 1,
    children,
    getAttribute() {
      return null;
    },
    hasAttribute() {
      return false;
    },
  };
}

function loadBridge({ body, queryResult, forms } = {}) {
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
  globalThis.Node = { ELEMENT_NODE: 1, TEXT_NODE: 3 };
  globalThis.window = { fetch() {} };
  globalThis.document = {
    body: body || makeBody([]),
    getElementById() {
      return null;
    },
    querySelector() {
      return queryResult ?? null;
    },
    querySelectorAll(selector) {
      if (selector === "form") return forms || [];
      return [];
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function skillsSelect(selected) {
  return makeSelect({
    multiple: true,
    options: [
      { value: "rust", selected: selected.includes("rust") },
      { value: "js", selected: selected.includes("js") },
      { value: "go", selected: selected.includes("go") },
    ],
  });
}

test("value reports every selected option of a multi-select (#158)", () => {
  const el = skillsSelect(["rust", "js"]);
  const pilot = loadBridge({ queryResult: el });
  assert.equal(pilot.value({ selector: "select[name=skills]" }), "rust, js");
});

test("snapshot reports every selected option of a multi-select (#158)", () => {
  const el = skillsSelect(["rust", "js"]);
  const { elements } = loadBridge({ body: makeBody([el]) }).snapshot();
  // A multi-select is a listbox (#307).
  const listbox = elements.find((e) => e.role === "listbox");
  assert.ok(listbox, "the <select> must appear in the snapshot");
  assert.equal(typeof listbox.value, "string");
  assert.equal(listbox.value, "rust, js");
});

test("value and snapshot match the forms dump of a multi-select (#158)", () => {
  const el = skillsSelect(["rust", "js"]);
  const pilot = loadBridge({
    body: makeBody([el]),
    queryResult: el,
    forms: [makeForm([el])],
  });
  const field = pilot.formDump().forms[0].fields[0];
  assert.deepEqual(field.value, ["rust", "js"]);
  const joined = field.value.join(", ");
  assert.equal(pilot.value({ selector: "select[name=skills]" }), joined);
  const snap = pilot.snapshot().elements.find((e) => e.role === "listbox");
  assert.equal(snap.value, joined);
});

test("value of a single-select is still the selected option", () => {
  const el = makeSelect({
    multiple: false,
    options: [
      { value: "rust", selected: true },
      { value: "js", selected: false },
    ],
  });
  const pilot = loadBridge({ queryResult: el });
  assert.equal(pilot.value({ selector: "select" }), "rust");
});

test("value of a multi-select with nothing selected is empty", () => {
  const el = skillsSelect([]);
  const pilot = loadBridge({ queryResult: el });
  assert.equal(pilot.value({ selector: "select[name=skills]" }), "");
});

test("value of a multi-select with one option has no separator", () => {
  const el = skillsSelect(["js"]);
  const pilot = loadBridge({ queryResult: el });
  assert.equal(pilot.value({ selector: "select[name=skills]" }), "js");
});

// `HTMLLIElement.value` is a `long`: 3 for `value="3"`, 0 when absent (#162).
function makeLi(_attrs, value) {
  const el = { tagName: "LI", nodeType: 1, children: [], textContent: "", value, _attrs };
  return Object.assign(el, attrs(el));
}

test("value of an <li> is its value attribute as a string (#162)", () => {
  const liValue = (el) => loadBridge({ queryResult: el }).value({ selector: "li" });
  assert.equal(liValue(makeLi({ value: "3" }, 3)), "3");
  assert.equal(liValue(makeLi({ value: "0" }, 0)), "0");
  assert.equal(liValue(makeLi({}, 0)), "");
});

// #326: a contenteditable host or a role=textbox / role=searchbox widget has
// no `.value`, and #303 stopped naming it after its text, so `value`,
// `snapshot` and `diff` saw nothing. Its text, whitespace collapsed, is its value.
function makeHost({ _attrs = {}, contentEditable, text }) {
  const el = { tagName: "DIV", nodeType: 1, children: [], textContent: text, _attrs };
  if (contentEditable !== undefined) el.contentEditable = contentEditable;
  return Object.assign(el, attrs(el));
}

test("value of a contenteditable host is its text, whitespace collapsed (#326)", () => {
  const el = makeHost({ contentEditable: "true", text: "  Draft\n  bold \t text " });
  assert.equal(loadBridge({ queryResult: el }).value({ selector: "#ce1" }), "Draft bold text");
});

test("value of a role=textbox or role=searchbox widget is its text (#326)", () => {
  const box = makeHost({ _attrs: { role: "textbox", tabindex: "0" }, text: "role text" });
  const search = makeHost({ _attrs: { role: " searchbox ", tabindex: "0" }, text: "query" });
  assert.equal(loadBridge({ queryResult: box }).value({ selector: "#rt1" }), "role text");
  assert.equal(loadBridge({ queryResult: search }).value({ selector: "#sb1" }), "query");
});

test("value of a plain div stays empty (#326)", () => {
  const el = makeHost({ contentEditable: "inherit", text: "just text" });
  assert.equal(loadBridge({ queryResult: el }).value({ selector: "div" }), "");
});

test("snapshot reports a textbox host's text as its value, not its name (#326)", () => {
  const ce = makeHost({ contentEditable: "true", text: "Draft bold text" });
  const box = makeHost({ _attrs: { role: "textbox", tabindex: "0" }, text: "role text" });
  const search = makeHost({ _attrs: { role: "searchbox", tabindex: "0" }, text: "query" });
  const empty = makeHost({ contentEditable: "true", text: "  " });
  const { elements } = loadBridge({ body: makeBody([ce, box, search, empty]) }).snapshot({
    interactive: true,
  });
  assert.deepEqual(
    elements.map((e) => [e.role, e.name, e.value]),
    [
      ["textbox", undefined, "Draft bold text"],
      ["textbox", undefined, "role text"],
      ["searchbox", undefined, "query"],
      ["textbox", undefined, undefined],
    ],
  );
});

test("value of a form control with role=textbox or contenteditable stays its IDL value (#326)", () => {
  const el = makeSelect({ multiple: false, options: [{ value: "rust", selected: true }] });
  el._attrs.role = "textbox";
  el.textContent = "Rust";
  assert.equal(loadBridge({ queryResult: el }).value({ selector: "select" }), "rust");
  const button = makeHost({ contentEditable: "true", text: "Go now" });
  button.tagName = "BUTTON";
  button.value = "go";
  assert.equal(loadBridge({ queryResult: button }).value({ selector: "button" }), "go");
});
