// Dependency-free behavioural tests for recorded locators (#276).
//
// `record` used to save the bare snapshot ref of each step (`"ref": "e9"`).
// Refs live in one document and are renumbered by every snapshot, so a replay
// either failed with `Unknown ref` (fresh page, case A) or acted on whatever
// the new snapshot called `e9` and reported ok (case B).
//
// The bridge now answers `locate` with a stable selector and a fingerprint
// (tag, role, name) for each ref, computed before the action runs, and
// `resolveTarget` replays a step carrying `expect` strictly: the selector must
// match exactly one element and that element must match the fingerprint.
//
// The DOM below is a small tree with a real `querySelectorAll` for the
// selector subset the locator emits (`#id`, `tag`, `[attr="v"]`,
// `:nth-of-type(n)`, ` > `), so uniqueness is computed from the tree.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.locator.test.mjs

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

class El {
  constructor(tag, attrs = {}, children = [], text = "") {
    this.tagName = tag.toUpperCase();
    this.nodeType = 1;
    this._attrs = { ...attrs };
    this.children = [];
    this.parentElement = null;
    this._text = text;
    this.checked = false;
    for (const child of children) this.append(child);
  }
  append(child) {
    child.parentElement = this;
    this.children.push(child);
    return child;
  }
  insertBefore(child, ref) {
    child.parentElement = this;
    this.children.splice(this.children.indexOf(ref), 0, child);
    return child;
  }
  remove() {
    const siblings = this.parentElement.children;
    siblings.splice(siblings.indexOf(this), 1);
    this.parentElement = null;
  }
  get childNodes() {
    return this.children.length > 0
      ? this.children
      : this._text
        ? [{ nodeType: 3, nodeValue: this._text, textContent: this._text }]
        : [];
  }
  get textContent() {
    return this._text + this.children.map((c) => c.textContent).join("");
  }
  getAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this._attrs, name) ? this._attrs[name] : null;
  }
  hasAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this._attrs, name);
  }
  setAttribute(name, value) {
    this._attrs[name] = String(value);
  }
  get type() {
    return (this.getAttribute("type") || "").toLowerCase();
  }
  get id() {
    return this.getAttribute("id") || "";
  }
  focus() {}
  getBoundingClientRect() {
    return { left: 0, top: 0, width: 10, height: 10 };
  }
  dispatchEvent() {
    return true;
  }
  click() {
    if (this.type === "radio") this.checked = true;
    else if (this.type === "checkbox") this.checked = !this.checked;
  }
}

const unescapeCss = (s) =>
  s.replace(/\\([0-9a-f]{1,6}) ?|\\(.)/gi, (_, hex, ch) =>
    hex ? String.fromCodePoint(parseInt(hex, 16)) : ch,
  );

// One compound selector of the subset the locator emits.
const COMPOUND =
  /^(?:#((?:\\[0-9a-f]{1,6} ?|\\.|[^\s>[:\\])+)|([a-z][a-z0-9]*)?(?:\[([a-z-]+)="((?:\\.|[^"\\])*)"\])?(?::nth-of-type\((\d+)\))?)$/i;

function matchesCompound(el, compound) {
  const m = COMPOUND.exec(compound);
  if (!m || compound === "") throw new SyntaxError("unsupported selector: " + compound);
  const [, id, tag, attr, value, nth] = m;
  if (id !== undefined) return el.getAttribute("id") === unescapeCss(id);
  if (tag && el.tagName !== tag.toUpperCase()) return false;
  if (attr && el.getAttribute(attr) !== unescapeCss(value)) return false;
  if (nth) {
    const sameTag = el.parentElement
      ? el.parentElement.children.filter((c) => c.tagName === el.tagName)
      : [el];
    if (sameTag.indexOf(el) + 1 !== Number(nth)) return false;
  }
  return true;
}

function matches(el, selector) {
  const parts = selector.split(" > ");
  let node = el;
  for (let i = parts.length - 1; i >= 0; i--) {
    if (!node || !matchesCompound(node, parts[i])) return false;
    node = node.parentElement;
  }
  return true;
}

function descendants(root) {
  const out = [];
  (function walk(node) {
    out.push(node);
    node.children.forEach(walk);
  })(root);
  return out;
}

function loadBridge(html) {
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
    documentElement: html,
    body: html.children[0],
    getElementById(id) {
      return descendants(html).find((el) => el.id === id) || null;
    },
    querySelectorAll(selector) {
      return descendants(html).filter((el) => matches(el, selector));
    },
    querySelector(selector) {
      return this.querySelectorAll(selector)[0] || null;
    },
  };
  function XMLHttpRequestStub() {}
  XMLHttpRequestStub.prototype.open = function () {};
  XMLHttpRequestStub.prototype.send = function () {};
  globalThis.XMLHttpRequest = XMLHttpRequestStub;
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

// The pilot-test-app shape from the issue: a login form with "Remember me",
// then a plan picker whose radios share one `name`.
function page() {
  const remember = new El("input", { type: "checkbox", name: "remember", "aria-label": "Remember me" });
  const free = new El("input", { type: "radio", name: "plan", "aria-label": "Free" });
  const pro = new El("input", { type: "radio", name: "plan", "aria-label": "Pro" });
  const fieldset = new El("fieldset", {}, [free, pro]);
  const user = new El("input", { name: "user", placeholder: "User" });
  const save = new El("button", { "data-testid": "save" }, [], "Save");
  const email = new El("input", { id: "email", "aria-label": "Email" });
  const form = new El("form", {}, [new El("h2", {}, [], "Login"), user, remember]);
  const body = new El("body", {}, [form, fieldset, save, email]);
  const html = new El("html", {}, [body]);
  return { html, body, form, fieldset, user, remember, free, pro, save, email };
}

// The ref a snapshot taken with `options` gives the element named `name`.
function refOf(pilot, name, options) {
  const entry = pilot.snapshot(options).elements.find((e) => e.name === name);
  assert.ok(entry, name + " is in the snapshot");
  return entry.ref;
}

function located(pilot, ref) {
  return pilot.locate({ refs: { self: ref } }).self;
}

test("locate prefers a unique #id", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Email", { interactive: true });
  assert.deepEqual(located(pilot, ref), {
    selector: "#email",
    expect: { tag: "input", role: "textbox", name: "Email" },
  });
});

test("locate skips an id another element shares", () => {
  const p = page();
  p.save.setAttribute("id", "dup");
  p.body.append(new El("div", { id: "dup" }));
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Save", { interactive: true });
  assert.equal(located(pilot, ref).selector, '[data-testid="save"]');
});

test("locate uses a unique tag[name] when there is no id or test id", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "User", { interactive: true });
  assert.equal(located(pilot, ref).selector, 'input[name="user"]');
});

test("locate falls back to a CSS path when the name is shared", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  assert.deepEqual(located(pilot, ref), {
    selector: "body > fieldset > input:nth-of-type(2)",
    expect: { tag: "input", role: "radio", name: "Pro" },
  });
});

test("a CSS path is not recorded when another element shares the fingerprint", () => {
  // Repeated rows carry identical buttons. A positional path to Beta's
  // "Delete" would pass the fingerprint check on Alpha's once a row is
  // inserted first, so the step must be reported as having no locator.
  const row = (label) => new El("li", {}, [new El("button", {}, [], "Delete")], label);
  const list = new El("ul", {}, [row("Alpha"), row("Beta")]);
  const html = new El("html", {}, [new El("body", {}, [list])]);
  const pilot = loadBridge(html);
  const ref = pilot.snapshot({ interactive: true }).elements.filter((e) => e.name === "Delete")[1].ref;
  assert.deepEqual(located(pilot, ref), {
    expect: { tag: "button", role: "button", name: "Delete" },
  });
});

test("a twin outside the path's unique-id anchor does not cost the selector", () => {
  // `#right > button` can never reach the button in #left, so the path
  // still identifies the element.
  const left = new El("div", { id: "left" }, [new El("button", {}, [], "Delete")]);
  const right = new El("div", { id: "right" }, [new El("button", {}, [], "Delete")]);
  const html = new El("html", {}, [new El("body", {}, [left, right])]);
  const pilot = loadBridge(html);
  const ref = pilot.snapshot({ interactive: true }).elements.filter((e) => e.name === "Delete")[1].ref;
  assert.equal(located(pilot, ref).selector, "#right > button");
});

test("the CSS path is anchored on the nearest ancestor with a unique id", () => {
  const p = page();
  p.fieldset.setAttribute("id", "plans");
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  assert.equal(located(pilot, ref).selector, "#plans > input:nth-of-type(2)");
});

test("ids and attribute values are CSS-escaped", () => {
  const p = page();
  p.email.setAttribute("id", "user.email");
  p.save.setAttribute("data-testid", 'say "hi"');
  const pilot = loadBridge(p.html);
  assert.equal(located(pilot, refOf(pilot, "Email")).selector, "#user\\.email");
  assert.equal(located(pilot, refOf(pilot, "Save")).selector, '[data-testid="say \\"hi\\""]');
});

test("an id with a leading digit is escaped as a code point", () => {
  const p = page();
  p.email.setAttribute("id", "1st");
  const pilot = loadBridge(p.html);
  assert.equal(located(pilot, refOf(pilot, "Email")).selector, "#\\31 st");
});

test("attribute values with line breaks still give a valid selector", () => {
  // `\r` or `\f` left raw in a CSS string is a parse error, which would
  // drop a unique test id and leave the step without a locator.
  const p = page();
  p.save.setAttribute("data-testid", "a\r\nb\fc");
  const pilot = loadBridge(p.html);
  const { selector } = located(pilot, refOf(pilot, "Save"));
  assert.equal(selector, '[data-testid="a\\d \\a b\\c c"]');
});

test("locate returns the fingerprint without a selector when nothing is unique", () => {
  // A node the page detached after the snapshot (a re-render) is still the
  // ref's element, but no selector can match it, so no candidate counts.
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  p.pro.remove();
  assert.deepEqual(located(pilot, ref), {
    expect: { tag: "input", role: "radio", name: "Pro" },
  });
});

test("locate covers drag source and target refs", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const source = refOf(pilot, "Save", { interactive: true });
  const target = refOf(pilot, "Email", { interactive: true });
  const out = pilot.locate({ refs: { source, target } });
  assert.equal(out.source.selector, '[data-testid="save"]');
  assert.equal(out.target.selector, "#email");
});

test("a stale drag ref does not cost the other ref its locator", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const source = refOf(pilot, "Save", { interactive: true });
  const out = pilot.locate({ refs: { source, target: "e999" } });
  assert.equal(out.source.selector, '[data-testid="save"]');
  assert.equal(out.target, undefined);
});

test("case A: a recorded step replays on a fresh document with no snapshot", () => {
  const recordedPage = page();
  let pilot = loadBridge(recordedPage.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  const { selector, expect } = located(pilot, ref);

  const fresh = page();
  pilot = loadBridge(fresh.html);
  assert.deepEqual(pilot.check({ selector, expect }), { ok: true });
  assert.equal(fresh.pro.checked, true);
  assert.equal(fresh.free.checked, false);
});

test("case B: a renumbering snapshot does not move the recorded step", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  const step = { ref, ...located(pilot, ref) };

  // A full snapshot numbers headings and forms too, so the recorded ref now
  // names another element.
  const moved = pilot.snapshot({}).elements.find((e) => e.ref === ref);
  assert.notEqual(moved.name, "Pro", "precondition: the ref names another element");

  // Even with the stale ref still in the params, the selector wins.
  assert.deepEqual(pilot.check(step), { ok: true });
  assert.equal(p.pro.checked, true);
  assert.equal(p.remember.checked, false);
});

test("a ref-only step with a fingerprint fails loudly once the ref moved", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Pro", { interactive: true });
  const { expect } = located(pilot, ref);
  pilot.snapshot({});
  assert.throws(
    () => pilot.check({ ref, expect }),
    (err) =>
      err.message.includes("Ref " + ref) &&
      err.message.includes('recorded <input role="radio" name="Pro">'),
  );
  assert.equal(p.pro.checked, false);
  assert.equal(p.remember.checked, false);
});

test("a fingerprint mismatch fails the step and names what differed", () => {
  const p = page();
  let pilot = loadBridge(p.html);
  const { selector, expect } = located(pilot, refOf(pilot, "Pro", { interactive: true }));

  // A new plan inserted before "Pro" takes its :nth-of-type slot.
  const fresh = page();
  const team = new El("input", { type: "radio", name: "plan", "aria-label": "Team" });
  fresh.fieldset.insertBefore(team, fresh.pro);
  pilot = loadBridge(fresh.html);
  assert.throws(
    () => pilot.check({ selector, expect }),
    (err) =>
      err.message.includes(selector) &&
      err.message.includes('<input role="radio" name="Team">') &&
      err.message.includes('recorded <input role="radio" name="Pro">'),
  );
  assert.equal(team.checked, false);
  assert.equal(fresh.pro.checked, false);
});

test("the fingerprint check compares the role on its own", () => {
  // Same tag, same name: only the role tells the radio from a checkbox.
  const p = page();
  const pilot = loadBridge(p.html);
  p.email.setAttribute("type", "checkbox");
  assert.throws(
    () => pilot.check({
      selector: "#email",
      expect: { tag: "input", role: "radio", name: "Email" },
    }),
    /found <input role="checkbox" name="Email">, recorded <input role="radio" name="Email">/,
  );
  assert.equal(p.email.checked, false);
});

test("the fingerprint check compares the tag on its own", () => {
  // Same role, same name: only the tag tells a <button> from a role=button div.
  const p = page();
  const pilot = loadBridge(p.html);
  assert.throws(
    () => pilot.click({
      selector: '[data-testid="save"]',
      expect: { tag: "div", role: "button", name: "Save" },
    }),
    /found <button role="button" name="Save">, recorded <div role="button" name="Save">/,
  );
});

test("a recorded selector matching several elements fails the step", () => {
  const p = page();
  let pilot = loadBridge(p.html);
  const { selector, expect } = located(pilot, refOf(pilot, "User", { interactive: true }));
  assert.equal(selector, 'input[name="user"]');

  // Both carry the same fingerprint, so only the uniqueness check stops this.
  const fresh = page();
  fresh.body.insertBefore(new El("input", { name: "user", placeholder: "User" }), fresh.form);
  pilot = loadBridge(fresh.html);
  assert.throws(
    () => pilot.fill({ selector, expect, value: "x" }),
    /Recorded selector input\[name="user"\] matches 2 elements, expected exactly 1/,
  );
});

test("a recorded selector matching nothing fails the step", () => {
  const p = page();
  let pilot = loadBridge(p.html);
  const { selector, expect } = located(pilot, refOf(pilot, "Email"));
  const fresh = page();
  fresh.email.remove();
  pilot = loadBridge(fresh.html);
  assert.throws(
    () => pilot.click({ selector, expect }),
    /No element matches recorded selector #email/,
  );
});

test("a recorded selector matching nothing does not fall back to a live ref", () => {
  // The ref still resolves in this document, to another element with the
  // same fingerprint. Falling back to it is how the wrong element gets hit.
  const p = page();
  const pilot = loadBridge(p.html);
  const ref = refOf(pilot, "Email", { interactive: true });
  const { selector, expect } = located(pilot, ref);
  p.email.setAttribute("id", "contact");
  assert.throws(
    () => pilot.click({ ref, selector, expect }),
    /No element matches recorded selector #email/,
  );
});

test("drag resolves a recorded source and target strictly", async () => {
  const p = page();
  let pilot = loadBridge(p.html);
  const refs = pilot.snapshot({ interactive: true }).elements;
  const out = pilot.locate({
    refs: {
      source: refs.find((e) => e.name === "Save").ref,
      target: refs.find((e) => e.name === "Email").ref,
    },
  });
  const fresh = page();
  fresh.email.setAttribute("aria-label", "Phone");
  pilot = loadBridge(fresh.html);
  await assert.rejects(
    pilot.drag({ source: out.source, target: out.target }),
    /#email.*recorded <input role="textbox" name="Email">/,
  );
});

test("a recorded selector the page rejects fails the step and names it", () => {
  // A hand-edited recording: the selector does not parse.
  const p = page();
  const pilot = loadBridge(p.html);
  assert.throws(
    () => pilot.click({ selector: "[", expect: { tag: "input", role: "textbox", name: "Email" } }),
    /Invalid recorded selector: \[/,
  );
});

test("a malformed fingerprint fails the step instead of a vague mismatch", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  for (const expect of [true, false, {}, { tag: 3 }]) {
    assert.throws(
      () => pilot.click({ selector: "#email", expect }),
      /Invalid recorded fingerprint: expected an object with a string "tag"/,
    );
  }
});

test("a plain selector without a fingerprint keeps the first-match behaviour", () => {
  const p = page();
  const pilot = loadBridge(p.html);
  assert.deepEqual(pilot.check({ selector: 'input[name="plan"]' }), { ok: true });
  assert.equal(p.free.checked, true);
});
