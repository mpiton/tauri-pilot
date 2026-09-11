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
  Object.assign(console, REAL_CONSOLE);
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
  const combobox = elements.find((e) => e.role === "combobox");
  assert.ok(combobox, "the <select> must appear in the snapshot");
  assert.equal(typeof combobox.value, "string");
  assert.equal(combobox.value, "rust, js");
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
  const snap = pilot.snapshot().elements.find((e) => e.role === "combobox");
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
