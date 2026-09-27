// Behavioural tests for the vendored html-to-image bundle (#255).
//
// html-to-image clones the DOM and serializes the clone into an SVG. The
// serializer only sees attributes, but checkboxes, radios and options keep
// their live state in the `checked` / `selected` properties. Unless the clone
// step copies that state into attributes, the screenshot shows the state from
// the initial HTML. These tests run the *real* vendored bundle against a
// minimal DOM mock and inspect the tree handed to XMLSerializer.
//
// Run: node --test crates/tauri-plugin-pilot/js/html-to-image.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const VENDOR_SRC = readFileSync(join(here, "vendor/html-to-image.iife.js"), "utf8");

class Element {
  constructor(tagName, attrs = {}, props = {}) {
    this.tagName = tagName.toUpperCase();
    this.attributes = new Map(Object.entries(attrs));
    this.childNodes = [];
    this.style = { getPropertyValue: () => "", setProperty() {}, getPropertyPriority: () => "" };
    Object.assign(this, props);
  }
  get children() {
    return this.childNodes;
  }
  appendChild(child) {
    this.childNodes.push(child);
    return child;
  }
  getAttribute(name) {
    return this.attributes.has(name) ? this.attributes.get(name) : null;
  }
  hasAttribute(name) {
    return this.attributes.has(name);
  }
  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }
  removeAttribute(name) {
    this.attributes.delete(name);
  }
  toggleAttribute(name, force) {
    if (force) this.setAttribute(name, "");
    else this.removeAttribute(name);
    return force;
  }
  // Like the real cloneNode, the copy starts from the attributes: live
  // properties such as `checked` do not reach the serialized markup.
  cloneNode() {
    return new this.constructor(this.tagName, Object.fromEntries(this.attributes));
  }
}
class HTMLInputElement extends Element {}
class HTMLSelectElement extends Element {}
class HTMLOptionElement extends Element {}

function el(Type, tagName, attrs, props, children = []) {
  const node = new Type(tagName, attrs, props);
  for (const child of children) node.appendChild(child);
  return node;
}

// Load the bundle into fresh globals and render `root` through toSvg. Returns
// the cloned tree that html-to-image passed to XMLSerializer.
async function renderClone(root) {
  const noStyle = { cssText: "", getPropertyValue: () => "", getPropertyPriority: () => "" };
  Object.assign(globalThis, {
    Element,
    HTMLInputElement,
    HTMLSelectElement,
    HTMLOptionElement,
    HTMLTextAreaElement: class extends Element {},
    HTMLCanvasElement: class extends Element {},
    HTMLVideoElement: class extends Element {},
    HTMLIFrameElement: class extends Element {},
    HTMLImageElement: class extends Element {},
    SVGImageElement: class extends Element {},
    window: { getComputedStyle: () => noStyle },
    document: { createElementNS: (_ns, tagName) => new Element(tagName) },
  });
  let serialized;
  globalThis.XMLSerializer = class {
    serializeToString(svg) {
      serialized = svg;
      return "";
    }
  };
  (0, eval)(VENDOR_SRC);
  await globalThis.htmlToImage.toSvg(root, {
    width: 100,
    height: 100,
    skipFonts: true,
    includeStyleProperties: [],
  });
  // svg > foreignObject > cloned root
  return serialized.childNodes[0].childNodes[0];
}

function find(root, value) {
  if (root.getAttribute("value") === value) return root;
  for (const child of root.childNodes) {
    const hit = find(child, value);
    if (hit) return hit;
  }
  return null;
}

test("clone carries the live checked state of radios and checkboxes", async () => {
  // Initial HTML: "free" and "remember" checked. At runtime the user picked
  // "pro" and unticked "remember".
  const form = el(Element, "form", {}, {}, [
    el(HTMLInputElement, "input", { type: "radio", value: "free", checked: "" }, { value: "free", checked: false }),
    el(HTMLInputElement, "input", { type: "radio", value: "pro" }, { value: "pro", checked: true }),
    el(HTMLInputElement, "input", { type: "checkbox", value: "remember", checked: "" }, { value: "remember", checked: false }),
  ]);

  const clone = await renderClone(form);

  assert.equal(find(clone, "free").hasAttribute("checked"), false);
  assert.equal(find(clone, "pro").hasAttribute("checked"), true);
  assert.equal(find(clone, "remember").hasAttribute("checked"), false);
});

test("clone carries the live selected option of a select", async () => {
  // Initial HTML selects "yearly"; at runtime the user picked "monthly". A
  // leftover `selected` on the later option would win when the SVG is parsed.
  const monthly = el(HTMLOptionElement, "option", { value: "monthly" }, { value: "monthly", selected: true });
  const yearly = el(HTMLOptionElement, "option", { value: "yearly", selected: "" }, { value: "yearly", selected: false });
  const select = el(HTMLSelectElement, "select", {}, { value: "monthly" }, [monthly, yearly]);

  const clone = await renderClone(select);

  assert.equal(find(clone, "monthly").hasAttribute("selected"), true);
  assert.equal(find(clone, "yearly").hasAttribute("selected"), false);
});
