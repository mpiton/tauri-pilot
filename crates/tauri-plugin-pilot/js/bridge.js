(() => {
  "use strict";

  const idMap = new Map();
  let refCounter = 0;

  const ROLE_MAP = {
    A: "link",
    BUTTON: "button",
    SELECT: "combobox",
    TEXTAREA: "textbox",
    IMG: "img",
    H1: "heading",
    H2: "heading",
    H3: "heading",
    H4: "heading",
    H5: "heading",
    H6: "heading",
    UL: "list",
    OL: "list",
    LI: "listitem",
    TABLE: "table",
    TR: "row",
    TH: "columnheader",
    TD: "cell",
    NAV: "navigation",
    MAIN: "main",
    ASIDE: "complementary",
    FORM: "form",
    DIALOG: "dialog",
    DETAILS: "group",
  };

  const INTERACTIVE_ROLES = new Set([
    "button",
    "link",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "textbox",
    "combobox",
  ]);

  function inputRole(el) {
    const t = (el.getAttribute("type") || "text").toLowerCase();
    switch (t) {
      case "checkbox":
        return "checkbox";
      case "radio":
        return "radio";
      case "range":
        return "slider";
      case "submit":
      case "reset":
      case "button":
        return "button";
      default:
        return "textbox";
    }
  }

  function getRole(el) {
    const explicit = el.getAttribute("role");
    if (explicit) return explicit;
    if (el.tagName === "INPUT") return inputRole(el);
    return ROLE_MAP[el.tagName] || null;
  }

  function getName(el) {
    const label = el.getAttribute("aria-label");
    if (label) return label.trim().slice(0, 50);

    const labelledBy = el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const parts = labelledBy
        .split(/\s+/)
        .map((id) => {
          const ref = document.getElementById(id);
          return ref ? ref.textContent : "";
        })
        .filter(Boolean);
      if (parts.length > 0) return parts.join(" ").trim().slice(0, 50);
    }

    if (el.tagName === "IMG") {
      const alt = el.getAttribute("alt");
      if (alt) return alt.trim().slice(0, 50);
    }

    if (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT") {
      const placeholder = el.getAttribute("placeholder");
      if (placeholder) return placeholder.trim().slice(0, 50);
    }

    const text = el.textContent || "";
    const trimmed = text.replace(/\s+/g, " ").trim();
    return trimmed.slice(0, 50) || null;
  }

  function isInteractiveElement(el) {
    const tag = el.tagName;
    if (
      tag === "INPUT" ||
      tag === "BUTTON" ||
      tag === "SELECT" ||
      tag === "TEXTAREA" ||
      tag === "A"
    ) {
      return true;
    }
    if (el.hasAttribute("tabindex")) return true;
    const role = el.getAttribute("role");
    return role ? INTERACTIVE_ROLES.has(role) : false;
  }

  function snapshot(options) {
    const interactive = (options && options.interactive) || false;
    const selector = (options && options.selector) || null;
    const maxDepth = (options && options.depth != null) ? options.depth : 255;

    refCounter = 0;
    idMap.clear();

    const root = selector ? document.querySelector(selector) : document.body;
    if (!root) return { elements: [] };

    const elements = [];

    function walk(node, currentDepth) {
      if (currentDepth > maxDepth) return;
      if (node.nodeType !== Node.ELEMENT_NODE) return;

      const role = getRole(node);
      const isInteractive = isInteractiveElement(node);

      if (interactive && !isInteractive) {
        for (const child of node.children) {
          walk(child, currentDepth);
        }
        return;
      }

      if (role) {
        refCounter++;
        const ref = "e" + refCounter;
        idMap.set(ref, node);

        const entry = { ref: ref, role: role, depth: currentDepth };
        const name = getName(node);
        if (name) entry.name = name;
        if (node.value !== undefined && node.value !== "") entry.value = node.value;
        if (node.checked !== undefined) entry.checked = node.checked;
        if (node.disabled) entry.disabled = true;
        elements.push(entry);
      }

      for (const child of node.children) {
        walk(child, currentDepth + 1);
      }
    }

    walk(root, 0);
    return { elements: elements };
  }

  function resolve(ref) {
    return idMap.get(ref) || null;
  }

  function requireEl(ref) {
    const el = idMap.get(ref);
    if (!el) throw new Error("Unknown ref: " + ref);
    return el;
  }

  function click(ref) {
    const el = requireEl(ref);
    el.focus();
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return { ok: true };
  }

  function fill(ref, value) {
    const el = requireEl(ref);
    el.focus();
    const setter =
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value") ||
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    if (setter && setter.set) {
      setter.set.call(el, value);
    } else {
      el.value = value;
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function typeText(ref, text) {
    const el = requireEl(ref);
    el.focus();
    for (const ch of text) {
      el.dispatchEvent(new KeyboardEvent("keydown", { key: ch, bubbles: true }));
      el.dispatchEvent(new InputEvent("input", { data: ch, inputType: "insertText", bubbles: true }));
      el.dispatchEvent(new KeyboardEvent("keyup", { key: ch, bubbles: true }));
    }
    return { ok: true };
  }

  function press(key) {
    const target = document.activeElement || document.body;
    target.dispatchEvent(new KeyboardEvent("keydown", { key: key, bubbles: true }));
    target.dispatchEvent(new KeyboardEvent("keyup", { key: key, bubbles: true }));
    return { ok: true };
  }

  function select(ref, value) {
    const el = requireEl(ref);
    el.value = value;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function check(ref) {
    const el = requireEl(ref);
    el.checked = !el.checked;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function scroll(options) {
    const dir = (options && options.direction) || "down";
    const amount = (options && options.amount) || 300;
    const ref = options && options.ref;
    const target = ref ? requireEl(ref) : window;
    const dx = (dir === "left" ? -amount : dir === "right" ? amount : 0);
    const dy = (dir === "up" ? -amount : dir === "down" ? amount : 0);
    target.scrollBy(dx, dy);
    return { ok: true };
  }

  window.__PILOT__ = {
    snapshot: snapshot,
    resolve: resolve,
    click: click,
    fill: fill,
    type: typeText,
    press: press,
    select: select,
    check: check,
    scroll: scroll,
  };
})();
