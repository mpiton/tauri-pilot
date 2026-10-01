// Dependency-free behavioural tests for the bridge `snapshot` (#120, #155, #162).
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// *real* file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation. The mock reproduces the DOM quirk behind #120:
// `HTMLLIElement.value` is an IDL `long` (a number, default `0`), so every
// `<li>` makes the bridge capture a JSON integer while the plugin types
// `SnapshotElement.value` as `Option<String>` — `diff` then aborts with
// `invalid type: integer 0, expected a string`.
//
// #162: that `0` is the default of the reflected `value` attribute, not the
// item's ordinal, so the string fix printed `value="0"` on every plain `<li>`.
//
// #155: walk() only emits a node when getRole() is non-null, and ROLE_MAP has
// no DIV entry. Interactive divs (draggable, contenteditable, click handlers)
// were therefore dropped from both the full snapshot and `snapshot -i`.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.snapshot.test.mjs

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

// Minimal element mock. Only the surface `snapshot`/`getRole`/`getName` read:
// tagName, nodeType, children, childNodes, attributes, textContent, `labels`,
// and the `value` IDL property (which the test sets explicitly, mirroring real
// DOM types).
function makeEl(tag, props = {}) {
  const children = props.children || [];
  const el = {
    tagName: tag.toUpperCase(),
    nodeType: 1, // Node.ELEMENT_NODE
    children,
    childNodes:
      children.length > 0 ? children : props.text ? [textNode(props.text)] : [],
    textContent: props.text || "",
    _attrs: props.attrs || {},
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(this._attrs, name)
        ? this._attrs[name]
        : null;
    },
    hasAttribute(name) {
      return Object.prototype.hasOwnProperty.call(this._attrs, name);
    },
  };
  if ("value" in props) el.value = props.value;
  if ("disabled" in props) el.disabled = props.disabled;
  if ("isContentEditable" in props) el.isContentEditable = props.isContentEditable;
  if ("contentEditable" in props) el.contentEditable = props.contentEditable;
  if ("onclick" in props) el.onclick = props.onclick;
  return el;
}

function textNode(text) {
  return { nodeType: 3, nodeValue: text, textContent: text };
}

// A <label> built from mixed text and element children, with `textContent`
// computed like the DOM does (every descendant's text, controls included).
function makeLabel(parts, attrs = {}) {
  const childNodes = parts.map((p) => (typeof p === "string" ? textNode(p) : p));
  const label = makeEl("label", { attrs });
  label.childNodes = childNodes;
  label.children = childNodes.filter((n) => n.nodeType === 1);
  label.textContent = childNodes.map((n) => n.textContent).join("");
  return label;
}

// A <select> whose `textContent` is the text of all its options, as in the DOM.
function makeSelect(optionTexts, props = {}) {
  const options = optionTexts.map((t) => makeEl("option", { text: t }));
  return makeEl("select", {
    ...props,
    children: options,
    text: optionTexts.join(" "),
  });
}

function named(elements, name) {
  return elements.find((e) => e.name === name);
}

// Fresh globals + a fresh bridge instance for each test (the IIFE early-returns
// if `window.__PILOT__` already exists, so `window` must be new every time).
function loadBridge(body) {
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
    body,
    getElementById() {
      return null;
    },
    querySelector() {
      return null;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;

  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

test("snapshot omits value for an <li> without a value attribute (#120, #162)", () => {
  // `HTMLLIElement.value` reflects the `value` attribute and reads `0` (a
  // number) when it is absent, in a <ul> and in an <ol> alike.
  const li = (text) => makeEl("li", { text, value: 0 });
  const ul = makeEl("ul", { children: [li("a"), li("b")] });
  const ol = makeEl("ol", { children: [li("c"), li("d")] });
  const body = makeEl("body", { children: [ul, ol] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const items = elements.filter((e) => e.role === "listitem");

  assert.equal(items.length, 4, "all four <li> should be captured");
  for (const item of items) {
    assert.equal("value" in item, false, `${item.name} must carry no value key`);
  }
});

test("snapshot coerces an author-set <li value> to a string, 0 included", () => {
  const zero = makeEl("li", { text: "zeroth", value: 0, attrs: { value: "0" } });
  const two = makeEl("li", { text: "second", value: 2, attrs: { value: "2" } });
  const list = makeEl("ol", { children: [zero, two] });
  const body = makeEl("body", { children: [list] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();

  assert.equal(named(elements, "zeroth").value, "0");
  assert.equal(named(elements, "second").value, "2");
});

test("snapshot reads <li value> as written, not the reflected long (#162)", () => {
  // `.value` reads 0 when the attribute is empty or not an integer.
  const blank = makeEl("li", { text: "blank", value: 0, attrs: { value: "" } });
  const lang = makeEl("li", { text: "french", value: 0, attrs: { value: "fr" } });
  const list = makeEl("ul", { children: [blank, lang] });
  const body = makeEl("body", { children: [list] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();

  assert.equal("value" in named(elements, "blank"), false);
  assert.equal(named(elements, "french").value, "fr");
});

test("snapshot preserves a genuine string value unchanged", () => {
  const input = makeEl("input", { value: "hello", attrs: { type: "text" } });
  const body = makeEl("body", { children: [input] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const field = elements.find((e) => e.value === "hello");

  assert.ok(field, "the input value should still be captured");
  assert.equal(field.value, "hello");
});

test("snapshot flags a password input as sensitive and keeps its raw value (#279)", () => {
  const user = makeEl("input", { value: "user@example.com", attrs: { type: "email" } });
  const pass = makeEl("input", { value: "s3cret!", attrs: { type: "Password" } });
  const body = makeEl("body", { children: [user, pass] });
  const pilot = loadBridge(body);

  const { elements } = pilot.snapshot();
  const secret = elements.find((e) => e.value === "s3cret!");
  const plain = elements.find((e) => e.value === "user@example.com");

  assert.ok(secret, "the password value stays in the raw payload for --json");
  assert.equal(secret.sensitive, true);
  assert.ok(plain, "the email input should be captured");
  assert.equal(plain.sensitive, undefined, "only password inputs are flagged");
});

test("snapshot includes a draggable card and assigns a usable ref (#155)", () => {
  const card = makeEl("div", {
    text: "Kanban card",
    attrs: { draggable: "true", id: "card-1" },
  });
  const layout = makeEl("div", { children: [card] });
  const body = makeEl("body", { children: [layout] });
  const fullPilot = loadBridge(body);
  const full = fullPilot.snapshot().elements;
  const interactive = loadBridge(body).snapshot({ interactive: true }).elements;

  const fromFull = named(full, "Kanban card");
  const fromInteractive = named(interactive, "Kanban card");
  assert.ok(fromFull, "full snapshot must emit the draggable card");
  assert.ok(fromInteractive, "snapshot -i must emit the draggable card");
  assert.equal(fromFull.role, "generic");
  assert.equal(fromInteractive.role, "generic");
  assert.equal(typeof fromFull.ref, "string");
  assert.equal(fullPilot.resolve(fromFull.ref), card);
  assert.equal(
    full.filter((e) => e !== fromFull).length,
    0,
    "plain layout divs must stay out of the full snapshot",
  );
});

test("snapshot -i includes contenteditable, onclick attribute, and onclick property (#155)", () => {
  const editor = makeEl("div", {
    text: "Rich text",
    isContentEditable: true,
    contentEditable: "true",
  });
  const plaintext = makeEl("div", {
    text: "Plain host",
    contentEditable: "plaintext-only",
  });
  const attrEditor = makeEl("div", {
    text: "Attr editor",
    attrs: { contenteditable: "true" },
  });
  const attrClick = makeEl("div", {
    text: "Attr click",
    attrs: { onclick: "doThing()" },
  });
  const propClick = makeEl("div", {
    text: "Prop click",
    onclick() {},
  });
  const tabbable = makeEl("div", {
    text: "Tab target",
    attrs: { tabindex: "0" },
  });
  const skipTarget = makeEl("div", {
    text: "Skip wrapper",
    attrs: { tabindex: "-1" },
  });
  const inert = makeEl("div", { text: "Just a box" });
  const notDrag = makeEl("div", {
    text: "Not draggable",
    attrs: { draggable: "false" },
  });
  const body = makeEl("body", {
    children: [
      editor,
      plaintext,
      attrEditor,
      attrClick,
      propClick,
      tabbable,
      skipTarget,
      inert,
      notDrag,
    ],
  });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  const editorEl = named(interactive, "Rich text");
  const plainEl = named(interactive, "Plain host");
  const attrEl = named(interactive, "Attr click");
  const propEl = named(interactive, "Prop click");
  const tabEl = named(interactive, "Tab target");
  assert.ok(editorEl, "contenteditable host must appear in snapshot -i");
  assert.equal(editorEl.role, "textbox");
  assert.ok(plainEl, "plaintext-only contenteditable must appear in snapshot -i");
  assert.equal(plainEl.role, "textbox");
  const attrEditorEl = named(interactive, "Attr editor");
  assert.ok(attrEditorEl, "contenteditable attribute alone must appear in snapshot -i");
  assert.equal(attrEditorEl.role, "textbox");
  assert.equal(pilot.resolve(attrEditorEl.ref), attrEditor);
  assert.ok(attrEl, "onclick attribute must make a div appear in snapshot -i");
  assert.equal(attrEl.role, "generic");
  assert.ok(propEl, "onclick property must make a div appear in snapshot -i");
  assert.equal(propEl.role, "generic");
  assert.ok(tabEl, "tabindex must still emit a previously unmapped div");
  assert.equal(tabEl.role, "generic");
  assert.equal(pilot.resolve(editorEl.ref), editor);
  assert.equal(pilot.resolve(attrEl.ref), attrClick);
  assert.equal(pilot.resolve(propEl.ref), propClick);
  assert.equal(pilot.resolve(tabEl.ref), tabbable);
  assert.equal(
    named(interactive, "Skip wrapper"),
    undefined,
    "unmapped tabindex=-1 wrappers must stay out of snapshot -i",
  );
  assert.equal(
    named(interactive, "Just a box"),
    undefined,
    "inert divs must stay out of snapshot -i",
  );
  assert.equal(
    named(interactive, "Not draggable"),
    undefined,
    "draggable=false must not count as interactive",
  );
});

test("snapshot -i lists the contenteditable host, not inherited descendants (#155)", () => {
  const paragraph = makeEl("p", {
    text: "Inner paragraph",
    isContentEditable: true,
    contentEditable: "inherit",
  });
  const island = makeEl("div", {
    text: "Widget island",
    contentEditable: "false",
    attrs: { contenteditable: "false" },
  });
  const editor = makeEl("div", {
    text: "Editor",
    isContentEditable: true,
    contentEditable: "true",
    children: [paragraph, island],
  });
  const body = makeEl("body", { children: [editor] });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  assert.ok(named(interactive, "Editor"), "the editable host must appear");
  assert.equal(
    named(interactive, "Inner paragraph"),
    undefined,
    "inherited contenteditable on inner nodes must not flood snapshot -i",
  );
  assert.equal(
    named(interactive, "Widget island"),
    undefined,
    "contenteditable=false islands must stay out of snapshot -i",
  );
  assert.equal(pilot.resolve(named(interactive, "Editor").ref), editor);
});

test("snapshot -i keeps an explicit-role host with tabindex=-1 and drops an unmapped -1 wrapper (#155)", () => {
  const dialog = makeEl("div", {
    text: "Modal",
    attrs: { role: "dialog", tabindex: "-1" },
  });
  const wrapper = makeEl("div", {
    text: "Dismissable layer",
    attrs: { tabindex: "-1" },
  });
  const tabbable = makeEl("div", {
    text: "Tab target",
    attrs: { tabindex: "0" },
  });
  const body = makeEl("body", { children: [dialog, wrapper, tabbable] });
  const interactive = loadBridge(body).snapshot({ interactive: true }).elements;

  const dialogEl = named(interactive, "Modal");
  assert.ok(dialogEl, "explicit role=dialog must still appear with tabindex=-1");
  assert.equal(dialogEl.role, "dialog");
  assert.equal(
    named(interactive, "Dismissable layer"),
    undefined,
    "unmapped tabindex=-1 wrappers must stay out of snapshot -i",
  );
  const tabEl = named(interactive, "Tab target");
  assert.ok(tabEl, "tabindex=0 host must still appear");
  assert.equal(tabEl.role, "generic");
});

test("snapshot -i still lists native controls and skips a wrapping layout div (#155)", () => {
  const button = makeEl("button", { text: "Save" });
  const wrapper = makeEl("div", { children: [button] });
  const body = makeEl("body", { children: [wrapper] });
  const pilot = loadBridge(body);
  const interactive = pilot.snapshot({ interactive: true }).elements;

  assert.equal(interactive.length, 1);
  assert.equal(interactive[0].role, "button");
  assert.equal(interactive[0].name, "Save");
  assert.equal(pilot.resolve(interactive[0].ref), button);
});

// #277: getName never read `el.labels`, so a control named by its <label> came
// out unnamed, and a <select> was named after the text of all its options.

test("snapshot names a select from its wrapping label, not its options (#277)", () => {
  const select = makeSelect(["Dark", "Light", "System"], { value: "dark" });
  const label = makeLabel(["Theme\n  ", select, "\n"]);
  select.labels = [label];
  const body = makeEl("body", { children: [label] });
  const pilot = loadBridge(body);

  const interactive = pilot.snapshot({ interactive: true }).elements;

  assert.equal(interactive.length, 1);
  assert.equal(interactive[0].role, "combobox");
  assert.equal(interactive[0].name, "Theme");
  assert.equal(pilot.resolve(interactive[0].ref), select);
});

test("snapshot names a checkbox from the text after it in its label (#277)", () => {
  const box = makeEl("input", {
    attrs: { type: "checkbox", name: "notifications" },
    value: "on",
  });
  box.checked = true;
  const label = makeLabel(["\n  ", box, " Enable notifications\n"]);
  box.labels = [label];
  const body = makeEl("body", { children: [label] });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.role, "checkbox");
  assert.equal(entry.name, "Enable notifications");
  assert.equal(entry.checked, true);
});

test("snapshot keeps another control's options out of a shared label (#277)", () => {
  const qty = makeEl("input", { attrs: { type: "number" } });
  const unit = makeSelect(["Small", "Medium", "Large"], { value: "Small" });
  const label = makeLabel(["Size ", qty, " ", unit]);
  // With no `for`, a label is associated with its first labelable
  // descendant only, so the DOM gives it to `qty` and not to `unit`.
  qty.labels = [label];
  unit.labels = [];
  const body = makeEl("body", { children: [label] });
  const pilot = loadBridge(body);

  const [qtyEl, unitEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(qtyEl.name, "Size");
  assert.equal("name" in unitEl, false);
});

test("snapshot keeps a textarea's text out of a shared label (#277)", () => {
  const bio = makeEl("input", { attrs: { type: "text" } });
  const notes = makeEl("textarea", { text: "draft text" });
  const label = makeLabel(["Bio ", bio, " ", notes]);
  bio.labels = [label];
  notes.labels = [];
  const body = makeEl("body", { children: [label] });
  const pilot = loadBridge(body);

  const [bioEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(bioEl.name, "Bio");
});

test("snapshot names a labelled button from its label then its own text (#277)", () => {
  // HTML-AAM: a button's associated <label> comes before its subtree, and the
  // label's text includes the button's own text.
  const button = makeEl("button", { text: "OK" });
  const label = makeLabel(["Confirm ", button]);
  button.labels = [label];
  const body = makeEl("body", { children: [label] });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.role, "button");
  assert.equal(entry.name, "Confirm OK");
});

test("snapshot drops hidden label text from the name (#277)", () => {
  const email = makeEl("input", { attrs: { type: "email" } });
  const star = makeEl("span", { text: "*", attrs: { "aria-hidden": "true" } });
  const emailLabel = makeLabel(["Email ", star, " ", email]);
  email.labels = [emailLabel];
  const phone = makeEl("input", { attrs: { type: "tel" } });
  const hint = makeEl("span", { text: "(optional)", attrs: { hidden: "" } });
  const phoneLabel = makeLabel(["Phone ", hint, " ", phone]);
  phone.labels = [phoneLabel];
  const body = makeEl("body", { children: [emailLabel, phoneLabel] });
  const pilot = loadBridge(body);

  const [emailEl, phoneEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(emailEl.name, "Email", "aria-hidden text is not part of the name");
  assert.equal(phoneEl.name, "Phone", "hidden text is not part of the name");
});

test("snapshot drops CSS-hidden label text from the name (#277)", () => {
  const email = makeEl("input", { attrs: { type: "email" } });
  const star = makeEl("span", { text: "*", attrs: { class: "req" } });
  star.computed = { display: "none", visibility: "visible" };
  const emailLabel = makeLabel(["Email ", star, " ", email]);
  email.labels = [emailLabel];
  // visibility is inherited but can be overridden, so a visible child of a
  // visibility:hidden span still counts, unlike a display:none subtree.
  const shown = makeEl("span", { text: "number" });
  shown.computed = { display: "inline", visibility: "visible" };
  const veiled = makeEl("span", { text: "(optional) " });
  veiled.childNodes = [textNode("(optional) "), shown];
  veiled.computed = { display: "inline", visibility: "hidden" };
  const phone = makeEl("input", { attrs: { type: "tel" } });
  const phoneLabel = makeLabel(["Phone ", veiled, " ", phone]);
  phone.labels = [phoneLabel];
  const caps = makeEl("input", { attrs: { type: "text" } });
  const capsStar = makeEl("span", { text: "*", attrs: { "aria-hidden": "TRUE" } });
  const capsLabel = makeLabel(["Name ", capsStar, " ", caps]);
  caps.labels = [capsLabel];
  const body = makeEl("body", { children: [emailLabel, phoneLabel, capsLabel] });
  const pilot = loadBridge(body);
  globalThis.window.getComputedStyle = (el) =>
    el.computed || { display: "inline", visibility: "visible" };

  const [emailEl, phoneEl, capsEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(emailEl.name, "Email", "display:none text is not part of the name");
  assert.equal(phoneEl.name, "Phone number", "visibility:hidden text is not either");
  assert.equal(capsEl.name, "Name", "aria-hidden is matched case-insensitively");
});

test("snapshot keeps a button's text ahead of its title (#277)", () => {
  const button = makeEl("button", { text: "Close", attrs: { title: "Close dialog" } });
  button.labels = [];
  const body = makeEl("body", { children: [button] });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.name, "Close");
});

test("snapshot names an input from its title before its placeholder (#277)", () => {
  // HTML-AAM for text inputs and textareas: label, then title, then placeholder.
  const input = makeEl("input", {
    attrs: { type: "search", title: "Search the docs", placeholder: "e.g. tauri" },
  });
  input.labels = [];
  const notes = makeEl("textarea", {
    attrs: { title: "Release notes", placeholder: "Write here" },
  });
  notes.labels = [];
  const body = makeEl("body", { children: [input, notes] });
  const pilot = loadBridge(body);

  const [inputEl, notesEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(inputEl.name, "Search the docs");
  assert.equal(notesEl.name, "Release notes");
});

test("snapshot names an input from a <label for> elsewhere in the page (#277)", () => {
  const label = makeLabel(["Email address"], { for: "email" });
  const input = makeEl("input", {
    attrs: { type: "email", id: "email", placeholder: "you@example.com" },
  });
  input.labels = [label];
  const body = makeEl("body", {
    children: [label, makeEl("div", { children: [input] })],
  });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.role, "textbox");
  assert.equal(entry.name, "Email address", "a label wins over the placeholder");
});

test("snapshot keeps aria-label ahead of a label (#277)", () => {
  const label = makeLabel(["Visible label"]);
  const input = makeEl("input", { attrs: { "aria-label": "Aria name" } });
  input.labels = [label];
  const body = makeEl("body", { children: [input] });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.name, "Aria name");
});

test("snapshot falls back to title, then leaves an unlabelled control unnamed (#277)", () => {
  const titled = makeEl("input", { attrs: { title: "Search the docs" } });
  titled.labels = [];
  const bare = makeEl("input", {});
  bare.labels = [];
  const select = makeSelect(["English", "Francais"], { value: "fr" });
  select.labels = [];
  const body = makeEl("body", { children: [titled, bare, select] });
  const pilot = loadBridge(body);

  const [titledEl, bareEl, selectEl] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(titledEl.name, "Search the docs");
  assert.equal("name" in bareEl, false, "an input with no label has no name");
  assert.equal(selectEl.role, "combobox");
  assert.equal(
    "name" in selectEl,
    false,
    "a select must not be named after its options",
  );
});

test("snapshot leaves an input with only a sibling label unnamed (#277)", () => {
  // <label>Email</label><br><input>: no `for`, so the DOM associates nothing
  // and browsers do not name the input either.
  const label = makeLabel(["Email"]);
  const input = makeEl("input", { attrs: { type: "text" } });
  input.labels = [];
  const body = makeEl("body", { children: [label, makeEl("br"), input] });
  const pilot = loadBridge(body);

  const [entry] = pilot.snapshot({ interactive: true }).elements;

  assert.equal(entry.role, "textbox");
  assert.equal("name" in entry, false);
});
