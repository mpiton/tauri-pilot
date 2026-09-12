(() => {
  "use strict";

  if (window.__PILOT__) return;

  const idMap = new Map();
  let refCounter = 0;

  const _logs = [];
  let _logIdCounter = 0;
  const MAX_LOGS = 500;

  const _networkRequests = [];
  let _netIdCounter = 0;
  const MAX_REQUESTS = 200;

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
    P: "paragraph",
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

  function serializeArg(arg) {
    if (arg === null) return null;
    if (arg === undefined) return null;
    if (typeof arg === 'string' || typeof arg === 'number' || typeof arg === 'boolean') return arg;
    try {
      JSON.stringify(arg);
      return arg;
    } catch (_) {
      return String(arg);
    }
  }

  function extractSource() {
    try {
      const stack = new Error().stack;
      if (!stack) return null;
      // Skip frames: Error constructor, extractSource, console[level] wrapper
      const lines = stack.split('\n');
      for (let i = 3; i < lines.length; i++) {
        const line = lines[i];
        if (line && !line.includes('__PILOT__')) return line.trim();
      }
      return null;
    } catch (_) { return null; }
  }

  const _originalConsole = {
    log: console.log.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
    info: console.info.bind(console),
  };

  ['log', 'warn', 'error', 'info'].forEach(level => {
    console[level] = function(...args) {
      const entry = {
        id: ++_logIdCounter,
        timestamp: Date.now(),
        level: level,
        args: args.map(serializeArg),
        source: extractSource(),
      };
      _logs.push(entry);
      if (_logs.length > MAX_LOGS) _logs.shift();
      _originalConsole[level].apply(console, args);
    };
  });

  function consoleLogs(options) {
    let result = _logs.slice();
    if (options) {
      if (options.level) {
        result = result.filter(e => e.level === options.level);
      }
      if (options.sinceId) {
        result = result.filter(e => e.id > options.sinceId);
      } else if (options.since) {
        result = result.filter(e => e.timestamp > options.since);
      }
      if (options.last) {
        result = result.slice(-options.last);
      }
    }
    return result;
  }

  function clearLogs() {
    _logs.length = 0;
    return { cleared: true };
  }

  function bodySize(body) {
    if (!body) return 0;
    if (typeof body === "string") return body.length;
    if (body instanceof URLSearchParams) return body.toString().length;
    if (body instanceof Blob) return body.size;
    if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) return body.byteLength;
    return 0;
  }

  // Tauri convertFileSrc(cmd, "ipc") produces `ipc://localhost/<cmd>` on
  // Unix/macOS and `http(s)://ipc.localhost/<cmd>` on Windows/Android.
  // WebKit treats `ipc:` as a non-special scheme, so URL.pathname is
  // `//localhost/<cmd>` rather than `/<cmd>` and an exact-path check misses
  // the eval/hello callback (#156). Match the raw ipc:// string (do not
  // decode first, do not use URL()) and parse http(s) with URL so a
  // userinfo form like `https://ipc.localhost@attacker/...` is not skipped.
  function isPilotCallbackCommand(cmd) {
    let decoded;
    try {
      decoded = decodeURIComponent(cmd);
    } catch (_) {
      decoded = cmd;
    }
    return decoded === "plugin:pilot|__callback" || decoded === "plugin:pilot|callback";
  }

  function isPilotIpcUrl(url) {
    const text = String(url);
    if (/^ipc:/i.test(text)) {
      const match = text.match(/^ipc:\/\/localhost\/([^/?#]+)(?:[?#]|$)/i);
      return !!match && isPilotCallbackCommand(match[1]);
    }
    if (/^https?:/i.test(text)) {
      let parsed;
      try {
        parsed = new URL(text);
      } catch (_) {
        return false;
      }
      if (parsed.hostname !== "ipc.localhost") return false;
      const match = parsed.pathname.match(/^\/([^/]+)$/);
      return !!match && isPilotCallbackCommand(match[1]);
    }
    return false;
  }

  const _originalFetch = window.fetch.bind(window);
  window.fetch = function(input, init) {
    const method = (init && init.method) || (input && input.method) || "GET";
    const url = (typeof input === "string") ? input : (input && input.url) || String(input);
    // Pilot's own IPC (eval callbacks, the bridge hello) is not app traffic.
    if (isPilotIpcUrl(url)) return _originalFetch(input, init);
    const timestamp = Date.now();
    const requestSize = bodySize(init && init.body);
    return _originalFetch(input, init).then(function(response) {
      const duration_ms = Date.now() - timestamp;
      const status = response.status;
      const responseSize = parseInt(response.headers.get("Content-Length") || "0", 10) || 0;
      const entry = {
        id: ++_netIdCounter,
        timestamp: timestamp,
        method: method,
        url: url,
        status: status,
        duration_ms: duration_ms,
        error: null,
        request_size: requestSize,
        response_size: responseSize,
      };
      _networkRequests.push(entry);
      if (_networkRequests.length > MAX_REQUESTS) _networkRequests.shift();
      return response;
    }, function(err) {
      const duration_ms = Date.now() - timestamp;
      const entry = {
        id: ++_netIdCounter,
        timestamp: timestamp,
        method: method,
        url: url,
        status: 0,
        duration_ms: duration_ms,
        error: err ? err.message : "Network error",
        request_size: requestSize,
        response_size: 0,
      };
      _networkRequests.push(entry);
      if (_networkRequests.length > MAX_REQUESTS) _networkRequests.shift();
      throw err;
    });
  };

  const _origXhrOpen = XMLHttpRequest.prototype.open;
  const _origXhrSend = XMLHttpRequest.prototype.send;

  XMLHttpRequest.prototype.open = function(method, url) {
    const result = _origXhrOpen.apply(this, arguments);
    this._pilot = { method: String(method), url: String(url) };
    return result;
  };

  XMLHttpRequest.prototype.send = function(body) {
    if (this._pilot && isPilotIpcUrl(this._pilot.url)) {
      return _origXhrSend.apply(this, arguments);
    }
    if (this._pilot) {
      const pilot = this._pilot;
      const timestamp = Date.now();
      const requestSize = bodySize(body);
      let recorded = false;
      let onLoad, onError, onTimeout, onAbort;
      const cleanup = () => {
        this.removeEventListener("load", onLoad);
        this.removeEventListener("error", onError);
        this.removeEventListener("timeout", onTimeout);
        this.removeEventListener("abort", onAbort);
      };
      const pushEntry = (status, error, responseSize) => {
        if (recorded) return;
        recorded = true;
        cleanup();
        const entry = {
          id: ++_netIdCounter,
          timestamp: timestamp,
          method: pilot.method,
          url: pilot.url,
          status: status,
          duration_ms: Date.now() - timestamp,
          error: error,
          request_size: requestSize,
          response_size: responseSize,
        };
        _networkRequests.push(entry);
        if (_networkRequests.length > MAX_REQUESTS) _networkRequests.shift();
      };
      onLoad = () => {
        const cl = parseInt(this.getResponseHeader("Content-Length") || "0", 10) || 0;
        const r = this.response;
        const responseSize = (this.responseType === "" || this.responseType === "text")
          ? ((r && r.length) || cl)
          : (r instanceof ArrayBuffer ? r.byteLength : (r instanceof Blob ? r.size : cl));
        pushEntry(this.status, null, responseSize);
      };
      onError = () => { pushEntry(0, "Network error", 0); };
      onTimeout = () => { pushEntry(0, "Timeout", 0); };
      onAbort = () => { pushEntry(0, "Aborted", 0); };
      this.addEventListener("load", onLoad);
      this.addEventListener("error", onError);
      this.addEventListener("timeout", onTimeout);
      this.addEventListener("abort", onAbort);
      try {
        return _origXhrSend.apply(this, arguments);
      } catch (err) {
        cleanup();
        throw err;
      }
    }
    return _origXhrSend.apply(this, arguments);
  };

  function networkRequests(options) {
    let result = _networkRequests.slice();
    if (options) {
      if (options.filter) {
        result = result.filter(e => e.url.includes(options.filter));
      }
      if (options.failedOnly) {
        result = result.filter(e => e.status >= 400 || e.status === 0 || e.error);
      }
      if (options.sinceId) {
        result = result.filter(e => e.id > options.sinceId);
      }
      if (options.last) {
        result = result.slice(-options.last);
      }
    }
    return result;
  }

  function clearNetwork() {
    _networkRequests.length = 0;
    return { cleared: true };
  }

  function inputRole(el) {
    const t = (el.getAttribute("type") || "text").toLowerCase();
    switch (t) {
      case "hidden":
        return null;
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
    return ROLE_MAP[el.tagName] || fallbackRole(el);
  }

  // walk() emits a node only when getRole() is non-null. ROLE_MAP has no DIV
  // entry, so interactive hosts built on unmapped tags need a fallback (#155).
  function fallbackRole(el) {
    if (carriesContentEditable(el)) return "textbox";
    if (!isInteractiveElement(el)) return null;
    // Negative tabindex is a focus trap, not a widget. Skip unmapped hosts
    // whose only extra signal is tabindex < 0 (Radix DismissableLayer, etc.).
    const tab = parseInt(el.getAttribute("tabindex"), 10);
    if (
      tab < 0 &&
      String(el.getAttribute("draggable") || "").toLowerCase() !== "true" &&
      !el.hasAttribute("onclick") &&
      typeof el.onclick !== "function"
    ) {
      return null;
    }
    return "generic";
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
    if (tag === "INPUT") {
      const t = (el.getAttribute("type") || "text").toLowerCase();
      return t !== "hidden";
    }
    if (
      tag === "BUTTON" ||
      tag === "SELECT" ||
      tag === "TEXTAREA" ||
      tag === "A"
    ) {
      return true;
    }
    if (el.hasAttribute("tabindex")) return true;
    if (String(el.getAttribute("draggable") || "").toLowerCase() === "true") {
      return true;
    }
    if (carriesContentEditable(el)) return true;
    if (el.hasAttribute("onclick") || typeof el.onclick === "function") {
      return true;
    }
    const role = el.getAttribute("role");
    return role ? INTERACTIVE_ROLES.has(role) : false;
  }

  function snapshot(options) {
    const interactive = (options && options.interactive) || false;
    const selector = (options && options.selector) || null;
    const maxDepth = (options && options.depth != null) ? options.depth : 255;

    refCounter = 0;
    idMap.clear();

    var root;
    if (selector) {
      try {
        root = document.querySelector(selector);
      } catch (e) {
        throw new Error("Invalid selector: " + selector);
      }
    } else {
      root = document.body;
    }
    if (!root) return { elements: [] };

    const elements = [];

    function walk(node, currentDepth) {
      if (currentDepth > maxDepth) return;
      if (node.nodeType !== Node.ELEMENT_NODE) return;

      const role = getRole(node);
      const isInteractive = isInteractiveElement(node);

      if (interactive && !isInteractive) {
        for (const child of node.children) {
          walk(child, currentDepth + 1);
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
        // `value` is an IDL property whose type varies by element: a string for
        // form controls, but a number for `<li>` (ordinal), `<progress>`, and
        // `<meter>`. Coerce to string so the wire format matches the plugin's
        // `SnapshotElement.value: Option<String>` contract (#120). Multi-select
        // joins every selected option (#158); a plain `<li>` has none (#162).
        const nodeVal = elementValue(node);
        if (nodeVal !== undefined && nodeVal !== "") entry.value = String(nodeVal);
        if (node.tagName === "INPUT") {
          var inputType = (node.getAttribute("type") || "text").toLowerCase();
          if (inputType === "checkbox" || inputType === "radio") {
            entry.checked = node.checked;
          }
        }
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

  function resolveTarget(params) {
    if (params.ref) return requireEl(params.ref);
    if (params.selector) {
      var el = document.querySelector(params.selector);
      if (!el) throw new Error("No element matches selector: " + params.selector);
      return el;
    }
    if (params.x != null && params.y != null) {
      var el = document.elementFromPoint(params.x, params.y);
      if (!el) throw new Error("No element at (" + params.x + "," + params.y + ")");
      return el;
    }
    throw new Error("No ref, selector, or coordinates provided");
  }

  function dispatchPointerEvent(el, type, options) {
    const init = Object.assign({
      bubbles: true,
      cancelable: true,
      composed: true,
      pointerId: 1,
      pointerType: "mouse",
      isPrimary: true,
      button: 0,
      buttons: type === "pointerdown" ? 1 : 0,
      view: window,
      clientX: 0,
      clientY: 0,
    }, options || {});

    if (typeof PointerEvent === "function") {
      return el.dispatchEvent(new PointerEvent(type, init));
    }

    const event = new MouseEvent(type, init);
    try {
      Object.defineProperty(event, "pointerId", { value: init.pointerId });
      Object.defineProperty(event, "pointerType", { value: init.pointerType });
      Object.defineProperty(event, "isPrimary", { value: init.isPrimary });
    } catch (_) {}
    return el.dispatchEvent(event);
  }

  function click(params) {
    const el = resolveTarget(params);
    const rect = el.getBoundingClientRect();
    const x = params.x != null ? params.x : rect.left + rect.width / 2;
    const y = params.y != null ? params.y : rect.top + rect.height / 2;
    const downInit = {
      clientX: x,
      clientY: y,
      button: 0,
      buttons: 1,
      detail: 1,
      view: window,
    };
    const upInit = {
      clientX: x,
      clientY: y,
      button: 0,
      buttons: 0,
      detail: 1,
      view: window,
    };
    const mouseInit = function(options) {
      return Object.assign({
        bubbles: true,
        cancelable: true,
        composed: true,
      }, options);
    };

    const pointerDownOk = dispatchPointerEvent(el, "pointerdown", downInit);
    if (pointerDownOk) {
      const mouseDownOk = el.dispatchEvent(new MouseEvent("mousedown", mouseInit(downInit)));
      if (mouseDownOk && typeof el.focus === "function") {
        el.focus();
      }
    }
    dispatchPointerEvent(el, "pointerup", upInit);
    if (pointerDownOk) {
      el.dispatchEvent(new MouseEvent("mouseup", mouseInit(upInit)));
    }
    dispatchPointerEvent(el, "click", upInit);
    return { ok: true };
  }

  // Resolve the native `value` setter for the element's actual prototype.
  // Frameworks (React, Preact-signals, Vue) sometimes install an instance-level
  // setter that swallows programmatic writes; preferring the prototype setter
  // bypasses that override and keeps WebIDL [LegacyUnforgeable] brand checks
  // happy on <input>, <textarea>, and <select> alike (#85).
  function nativeValueSetter(el) {
    const proto = Object.getPrototypeOf(el);
    const desc = proto && Object.getOwnPropertyDescriptor(proto, "value");
    return desc && typeof desc.set === "function" ? desc.set : null;
  }

  function elementTag(el) {
    return el && el.tagName ? String(el.tagName).toLowerCase() : "";
  }

  function isValueElement(el) {
    const tag = elementTag(el);
    return tag === "input" || tag === "textarea";
  }

  // Realm-safe: `isContentEditable` is an instance property, not a constructor
  // check, so a contenteditable node from another window/iframe still matches.
  // Fall back to the contentEditable IDL string for hosts that only expose that.
  function isContentEditable(el) {
    if (!el) return false;
    if (el.isContentEditable === true) return true;
    const mode = el.contentEditable != null ? String(el.contentEditable).toLowerCase() : "";
    return mode === "true" || mode === "plaintext-only";
  }

  // Snapshot interactivity: only the host that carries contenteditable, not
  // descendants whose IDL `isContentEditable` is inherited (#155).
  function carriesContentEditable(el) {
    const mode = el.contentEditable != null ? String(el.contentEditable).toLowerCase() : "";
    if (mode === "true" || mode === "plaintext-only") return true;
    if (!el.hasAttribute || !el.hasAttribute("contenteditable")) return false;
    const attr = String(el.getAttribute("contenteditable") || "").toLowerCase();
    return attr === "true" || attr === "plaintext-only" || attr === "";
  }

  function requireEditable(el, action) {
    if (isValueElement(el) || isContentEditable(el)) return;
    if (action === "fill" && elementTag(el) === "select") return;
    const reported = (elementTag(el) || String(el)).slice(0, 64);
    if (action === "fill") {
      throw new Error("fill requires an <input>, <textarea>, <select>, or contenteditable element, got: " + reported);
    }
    throw new Error(action + " requires an <input>, <textarea>, or contenteditable element, got: " + reported);
  }

  function requireCheckable(el) {
    const tag = elementTag(el);
    const type = el && el.type != null ? String(el.type).toLowerCase() : "";
    if (tag === "input" && (type === "checkbox" || type === "radio")) return;
    const reported = (tag === "input" ? "input type=" + type : tag || String(el)).slice(0, 64);
    throw new Error('check requires an <input type="checkbox"> or <input type="radio">, got: ' + reported);
  }

  function ownerDoc(el) {
    return (el && el.ownerDocument) || document;
  }

  function tryExecCommand(el, command, value) {
    try {
      const doc = ownerDoc(el);
      return typeof doc.execCommand === "function" && doc.execCommand(command, false, value);
    } catch (_) {
      return false;
    }
  }

  function collapseToEnd(el) {
    try {
      const doc = ownerDoc(el);
      const range = doc.createRange();
      range.selectNodeContents(el);
      range.collapse(false);
      const view = doc.defaultView || window;
      const sel = view.getSelection && view.getSelection();
      if (!sel) return;
      sel.removeAllRanges();
      sel.addRange(range);
    } catch (_) {}
  }

  function fillContentEditable(el, value) {
    try {
      const doc = ownerDoc(el);
      const range = doc.createRange();
      range.selectNodeContents(el);
      const view = doc.defaultView || window;
      const sel = view.getSelection && view.getSelection();
      if (sel) {
        sel.removeAllRanges();
        sel.addRange(range);
        if (tryExecCommand(el, "insertText", value)) return true;
      }
    } catch (_) {}
    el.textContent = value;
    return false;
  }

  function typeContentEditable(el, text) {
    collapseToEnd(el);
    for (const ch of text) {
      el.dispatchEvent(new KeyboardEvent("keydown", { key: ch, bubbles: true }));
      if (!tryExecCommand(el, "insertText", ch)) {
        el.textContent = (el.textContent || "") + ch;
        el.dispatchEvent(new InputEvent("input", { data: ch, inputType: "insertText", bubbles: true }));
      }
      el.dispatchEvent(new KeyboardEvent("keyup", { key: ch, bubbles: true }));
    }
  }

  function applySelectOption(el, wantedRaw) {
    // Resolve the target option before mutating anything. Setting
    // `HTMLSelectElement.value` to a string that matches no option `value`
    // silently yields `value=""` / `selectedIndex=-1` per the DOM spec, so
    // "set then trust" reports success on a no-op (#113). Match the option
    // first — by `value`, then by visible label — and error if none matches so
    // a reported `ok` always means an option was actually selected.
    const wanted = String(wantedRaw);
    const options = Array.from(el.options || []);
    const matched =
      options.find((o) => o.value === wanted) ||
      options.find((o) => (o.text || "").trim() === wanted.trim());
    if (!matched) {
      throw new Error("select: no option matches " + JSON.stringify(wantedRaw));
    }
    const setter = nativeValueSetter(el);
    if (setter) {
      setter.call(el, matched.value);
    } else {
      el.value = matched.value;
    }
  }

  function fill(params) {
    const el = resolveTarget(params);
    requireEditable(el, "fill");
    el.focus();
    let wroteViaExec = false;
    if (elementTag(el) === "select") {
      applySelectOption(el, params.value);
    } else if (isValueElement(el)) {
      const setter = nativeValueSetter(el);
      if (setter) {
        setter.call(el, params.value);
      } else {
        el.value = params.value;
      }
    } else {
      wroteViaExec = fillContentEditable(el, params.value);
    }
    // insertText already fires a native `input` event. Dispatching again
    // would double-notify listeners; the textContent fallback does not.
    if (!wroteViaExec) {
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    }
    return { ok: true };
  }

  function typeText(params) {
    const el = resolveTarget(params);
    if (elementTag(el) === "select") {
      throw new Error("type cannot target a <select>; use fill or select");
    }
    requireEditable(el, "type");
    el.focus();
    if (!isValueElement(el)) {
      typeContentEditable(el, params.text);
      return { ok: true };
    }
    const setter = nativeValueSetter(el);
    for (const ch of params.text) {
      el.dispatchEvent(new KeyboardEvent("keydown", { key: ch, bubbles: true }));
      if (setter) {
        setter.call(el, el.value + ch);
      } else {
        el.value += ch;
      }
      el.dispatchEvent(new InputEvent("input", { data: ch, inputType: "insertText", bubbles: true }));
      el.dispatchEvent(new KeyboardEvent("keyup", { key: ch, bubbles: true }));
    }
    return { ok: true };
  }

  function select(params) {
    const el = resolveTarget(params);
    // The CLI/tool contract is "select acts on <select>". Before the
    // nativeValueSetter refactor, this guarantee fell out of the WebIDL brand
    // check on `HTMLSelectElement.prototype.value` (calling that setter on an
    // <input>/<textarea> threw). The new helper picks the setter from the
    // element's own prototype, so a misrouted selector would now silently
    // succeed against a non-<select> and report ok while no option was
    // actually selected. Re-introduce the type guard with a tag-based check
    // (realm-safe): an `instanceof` constructor check would be tied to the
    // host realm and would reject valid <select> elements coming from another
    // window/iframe realm, which is exactly the case nativeValueSetter was
    // built to support.
    const tag = el && el.tagName ? String(el.tagName).toLowerCase() : "";
    if (tag !== "select") {
      const reported = (tag || String(el)).slice(0, 64);
      throw new Error("select requires a <select> element, got: " + reported);
    }
    applySelectOption(el, params.value);
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function check(params) {
    const el = resolveTarget(params);
    requireCheckable(el);
    el.checked = !el.checked;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function overflowValue(style, axis) {
    if (!style) return "";
    return style[axis] || style.overflow || "";
  }

  function axisCanScroll(el, style, axis, scrollSize, clientSize) {
    var overflow = overflowValue(style, axis);
    return (overflow === "auto" || overflow === "scroll" || overflow === "overlay")
      && el[scrollSize] > el[clientSize];
  }

  function canScroll(el) {
    if (!el || el === window) return false;
    var style = null;
    if (typeof window.getComputedStyle === "function") {
      style = window.getComputedStyle(el);
    }
    if (!style) style = el.style;
    return axisCanScroll(el, style, "overflowY", "scrollHeight", "clientHeight")
      || axisCanScroll(el, style, "overflowX", "scrollWidth", "clientWidth");
  }

  // Coords resolve to the topmost node at the point, usually a child inside
  // the scroller. scrollTop/scrollBy on that child is a no-op.
  function nearestScrollTarget(el) {
    if (!el || el === window) return el;
    var node = el;
    while (node && node !== document && node !== document.documentElement && node !== document.body) {
      if (canScroll(node)) return node;
      node = node.parentElement;
    }
    return el;
  }

  function scroll(options) {
    const dir = (options && options.direction) || "down";
    const amount = (options && options.amount) || 300;
    // Same target shapes as click/fill/text: snapshot ref, CSS selector, or
    // coordinates. No target still means the page (`window`), which is why
    // this cannot call `resolveTarget` unconditionally (#157).
    const resolved = (options && (options.ref || options.selector || (options.x != null && options.y != null)))
      ? resolveTarget(options)
      : window;
    const target = nearestScrollTarget(resolved);

    if (dir === "top") {
      if (target === window) {
        target.scrollTo(window.scrollX, 0);
      } else {
        target.scrollTop = 0;
      }
      return { ok: true };
    }
    if (dir === "bottom") {
      if (target === window) {
        const docEl = document.documentElement;
        const body = document.body;
        const fullHeight = Math.max(
          docEl ? docEl.scrollHeight : 0,
          body ? body.scrollHeight : 0
        );
        const viewportHeight = docEl ? docEl.clientHeight : window.innerHeight;
        const max = fullHeight - viewportHeight;
        target.scrollTo(window.scrollX, Math.max(0, max));
      } else {
        target.scrollTop = Math.max(0, target.scrollHeight - target.clientHeight);
      }
      return { ok: true };
    }
    if (dir !== "up" && dir !== "down" && dir !== "left" && dir !== "right") {
      const safeDir = String(dir).slice(0, 64);
      throw new Error("Unknown scroll direction: " + safeDir + " (expected up|down|left|right|top|bottom)");
    }
    const dx = (dir === "left" ? -amount : dir === "right" ? amount : 0);
    const dy = (dir === "up" ? -amount : dir === "down" ? amount : 0);
    target.scrollBy(dx, dy);
    return { ok: true };
  }

  // A pointer event followed by the compatibility mouse event a browser would
  // synthesise from it — same order and same preventDefault gate as `click()`,
  // since a cancelled pointer event suppresses its mouse counterpart. Goes
  // through `dispatchPointerEvent`, so a WebView without the `PointerEvent`
  // constructor still gets a pointer-typed event (a MouseEvent with
  // pointerId/pointerType patched on) rather than nothing at all — dnd-kit's
  // default sensor is `PointerSensor`, so that fallback is the whole point.
  function dispatchGesturePair(node, pointerType, mouseType, x, y, buttons) {
    var init = { clientX: x, clientY: y, buttons: buttons, view: window };
    if (!dispatchPointerEvent(node, pointerType, init)) return false;
    return node.dispatchEvent(new MouseEvent(mouseType, Object.assign({
      bubbles: true,
      cancelable: true,
      composed: true,
      button: 0,
    }, init)));
  }

  // Deepest node under a viewport point, like a real pointer. Falls back to
  // `document` when nothing is hit-testable there.
  function nodeAtPoint(x, y) {
    return (document.elementFromPoint && document.elementFromPoint(x, y)) || document;
  }

  function pilotSleep(ms) {
    return new Promise(function (resolve) {
      setTimeout(resolve, ms);
    });
  }

  async function drag(params) {
    var source = resolveTarget(params.source || params);
    var sourceRect = source.getBoundingClientRect();
    var startX = sourceRect.left + sourceRect.width / 2;
    var startY = sourceRect.top + sourceRect.height / 2;

    var endX, endY, dropTarget;

    if (params.target) {
      dropTarget = resolveTarget(params.target);
      var targetRect = dropTarget.getBoundingClientRect();
      endX = targetRect.left + targetRect.width / 2;
      endY = targetRect.top + targetRect.height / 2;
    } else if (params.offset) {
      // elementFromPoint below is viewport-bound: a start point outside the
      // viewport would make the lookup miss (#130). Scroll the source into
      // view first, like a user would, then recompute the start point.
      // "instant" so a page-level `scroll-behavior: smooth` cannot defer the
      // scroll past the synchronous rect recompute.
      var docEl = document.documentElement;
      var viewportWidth = docEl.clientWidth;
      var viewportHeight = docEl.clientHeight;
      if (startX < 0 || startY < 0 || startX >= viewportWidth || startY >= viewportHeight) {
        source.scrollIntoView({ behavior: "instant", block: "center", inline: "center" });
        sourceRect = source.getBoundingClientRect();
        startX = sourceRect.left + sourceRect.width / 2;
        startY = sourceRect.top + sourceRect.height / 2;
      }
      var offsetX = params.offset.x || 0;
      var offsetY = params.offset.y || 0;
      endX = startX + offsetX;
      endY = startY + offsetY;
      dropTarget = document.elementFromPoint(endX, endY);
      if (!dropTarget) {
        var pointLabel = "(" + Math.round(endX) + "," + Math.round(endY) + ")";
        if (endX < 0 || endY < 0 || endX >= viewportWidth || endY >= viewportHeight) {
          throw new Error("Drop point " + pointLabel + " is outside the viewport (" +
            viewportWidth + "x" + viewportHeight + ") — reduce the offset");
        }
        throw new Error("No element at drop point " + pointLabel +
          " for offset (" + offsetX + "," + offsetY + ")");
      }
    } else {
      throw new Error("drag requires target or offset");
    }

    // Two families of drag implementation exist and they listen for different
    // things, so a gesture that only satisfies one silently does nothing in the
    // other:
    //
    //   * HTML5 native DnD (`draggable="true"`) wants dragstart/dragover/drop.
    //   * JS libraries (dnd-kit, sortable.js, interact.js, react-dnd's mouse
    //     backend) never see those. They activate on mousedown and then track
    //     *repeated* mousemove/pointermove events on `document`, usually behind a
    //     small distance threshold, and commit on mouseup. A single mousedown with
    //     no movement and no release cannot activate them.
    //
    // So emit both: a real press-move-release stream plus the HTML5 sequence.
    var steps = Number(params.steps);
    if (!isFinite(steps) || steps < 1) steps = 12;
    steps = Math.min(Math.floor(steps), 60);
    var stepDelay = Number(params.stepDelayMs);
    if (!isFinite(stepDelay) || stepDelay < 0) stepDelay = 16;
    var settleMs = Number(params.settleMs);
    if (!isFinite(settleMs) || settleMs < 0) settleMs = 250;

    var dt = typeof DataTransfer === "function" ? new DataTransfer() : new ClipboardEvent("").clipboardData;

    // Press on the deepest node under the point, not the resolved container: a
    // library's listeners are commonly attached to an inner handle or card, and
    // events only bubble upward, so pressing the ancestor never reaches them.
    var pressTarget = source;
    if (document.elementFromPoint) {
      var atPoint = document.elementFromPoint(startX, startY);
      // Only a hit inside the source counts. A toast, backdrop or any overlay
      // covering the start point would otherwise take the press, the source
      // would never move, and `drag` would still report ok — the exact false
      // green this action exists to remove.
      if (atPoint && (atPoint === source || source.contains(atPoint))) pressTarget = atPoint;
    }

    dispatchGesturePair(pressTarget, "pointerdown", "mousedown", startX, startY, 1);
    source.dispatchEvent(new DragEvent("dragstart", { clientX: startX, clientY: startY, dataTransfer: dt, bubbles: true }));

    // Each move targets the node under the point. Dispatching on `document`
    // instead would give the event a propagation path of `[window, document]`
    // and nothing else, so any listener on an element between the pressed node
    // and the document never fires — React 17+ delegates on its root container,
    // not on `document`. Hit-testing costs one lookup per step and still reaches
    // the document-level listeners libraries install, because these bubble.
    for (var i = 1; i <= steps; i++) {
      var moveX = startX + ((endX - startX) * i) / steps;
      var moveY = startY + ((endY - startY) * i) / steps;
      dispatchGesturePair(nodeAtPoint(moveX, moveY), "pointermove", "mousemove", moveX, moveY, 1);
      // The delay spaces the moves apart, so after the last one it separates
      // nothing and only pushes the drop sequence back.
      if (stepDelay > 0 && i < steps) await pilotSleep(stepDelay);
    }

    source.dispatchEvent(new DragEvent("dragleave", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true }));
    dropTarget.dispatchEvent(new DragEvent("dragenter", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true }));
    dropTarget.dispatchEvent(new DragEvent("dragover", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true }));
    // A cancelled drop event means an HTML5 handler claimed it (preventDefault).
    var html5DropHandled = !dropTarget.dispatchEvent(
      new DragEvent("drop", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true })
    );
    source.dispatchEvent(new DragEvent("dragend", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true }));

    dispatchGesturePair(nodeAtPoint(endX, endY), "pointerup", "mouseup", endX, endY, 0);

    // Library drops commonly run async work (state update, request, re-render), so
    // give it a beat before the caller asserts on the DOM.
    if (settleMs > 0) await pilotSleep(settleMs);

    // `ok` reports that the gesture was delivered — it cannot know whether the app
    // acted on it. Assert the expected effect separately.
    return {
      ok: true,
      from: { x: startX, y: startY },
      to: { x: endX, y: endY },
      steps: steps,
      html5DropHandled: html5DropHandled,
    };
  }

  function drop(params) {
    var el = resolveTarget(params);
    var rect = el.getBoundingClientRect();
    var x = rect.left + rect.width / 2;
    var y = rect.top + rect.height / 2;
    var dt = typeof DataTransfer === "function" ? new DataTransfer() : new ClipboardEvent("").clipboardData;

    if (params.files) {
      for (var i = 0; i < params.files.length; i++) {
        var f = params.files[i];
        var binary = atob(f.data);
        var bytes = new Uint8Array(binary.length);
        for (var j = 0; j < binary.length; j++) bytes[j] = binary.charCodeAt(j);
        var file = new File([bytes], f.name, { type: f.type || "application/octet-stream" });
        dt.items.add(file);
      }
    }

    el.dispatchEvent(new DragEvent("dragenter", { clientX: x, clientY: y, dataTransfer: dt, bubbles: true, cancelable: true }));
    el.dispatchEvent(new DragEvent("dragover", { clientX: x, clientY: y, dataTransfer: dt, bubbles: true, cancelable: true }));
    el.dispatchEvent(new DragEvent("drop", { clientX: x, clientY: y, dataTransfer: dt, bubbles: true, cancelable: true }));
    return { ok: true };
  }

  function text(params) {
    return resolveTarget(params).textContent || "";
  }

  function html(params) {
    if (params && (params.ref || params.selector)) {
      return resolveTarget(params).innerHTML;
    }
    return document.documentElement.innerHTML;
  }

  // Selected option values in tree order. `HTMLSelectElement.value` is only
  // the first; `forms.dump` already walks `.options` this way (#158).
  function selectedOptionValues(el) {
    const selected = [];
    const options = el && el.options;
    if (!options) return selected;
    for (let k = 0; k < options.length; k++) {
      if (options[k].selected) selected.push(options[k].value);
    }
    return selected;
  }

  // Display value for `value` / `snapshot`. Multi-select joins with `", "`
  // so the string matches the `forms` CLI (`skills = "rust, js"`). An `<li>`
  // has a value only with a `value` attribute: `HTMLLIElement.value` reflects
  // it and reads `0` when it is absent, even in an `<ol>` (#162). Other
  // elements keep their IDL `.value`.
  function elementValue(el) {
    if (!el) return undefined;
    const tag = String(el.tagName || "").toLowerCase();
    if (tag === "select" && el.multiple) return selectedOptionValues(el).join(", ");
    if (tag === "li" && !el.hasAttribute("value")) return undefined;
    return el.value;
  }

  function value(params) {
    return elementValue(resolveTarget(params)) || "";
  }

  function attrs(params) {
    const el = resolveTarget(params);
    const result = {};
    for (const attr of el.attributes) {
      result[attr.name] = attr.value;
    }
    return result;
  }

  function visible(params) {
    const el = resolveTarget(params);
    const style = getComputedStyle(el);
    const isVisible =
      style.display !== "none" &&
      style.visibility !== "hidden" &&
      style.opacity !== "0" &&
      (el.offsetWidth > 0 || el.offsetHeight > 0);
    return { visible: isVisible };
  }

  function count(params) {
    if (!params || !params.selector) {
      throw new Error("count requires a selector parameter");
    }
    return { count: document.querySelectorAll(params.selector).length };
  }

  function checked(params) {
    const el = resolveTarget(params);
    return { checked: !!el.checked };
  }

  function navigate(options) {
    const url = options && options.url;
    if (url) window.location.href = url;
    return { ok: true };
  }

  function url() {
    return window.location.href;
  }

  function title() {
    return document.title;
  }

  function state() {
    return {
      url: window.location.href,
      title: document.title,
      readyState: document.readyState,
      viewport: { width: window.innerWidth, height: window.innerHeight },
      scroll: { x: window.scrollX, y: window.scrollY },
    };
  }

  function evalScript(options) {
    var script = options && options.script;
    if (!script) throw new Error("No script provided");
    // Stage 1 — expression compile.
    // `{a:1}` keeps its object-literal semantics (not a labeled block) and
    // `class C {}` evaluates to the constructor. Keep compilation separate
    // from execution: a runtime SyntaxError from e.g. `JSON.parse('x')` must
    // propagate, not trigger a fallback — otherwise the script would run twice.
    var expr;
    try {
      expr = new Function("return (\n" + script + "\n)");
    } catch (e1) {
      if (!(e1 instanceof SyntaxError)) throw e1;
      // The newlines around `script` in every wrapper below isolate user
      // tokens from generated closing punctuation. Without them, a trailing
      // `// comment` on the last line of the user script swallows `))()` or
      // `})()` and the wrapper fails to compile.
      if (hasTopLevelAwait(script)) {
        // Stage 2 — async-expression compile (#79).
        // Handles top-level `await` in expression position, e.g.
        // `await Promise.resolve("hi")` or `await fetch(...).then(r => r.json())`.
        // Returns a Promise; the Rust wrapper already awaits it.
        try {
          var asyncExpr = new Function(
            "return (async () => (\n" + script + "\n))()"
          );
          return asyncExpr();
        } catch (e2) {
          if (!(e2 instanceof SyntaxError)) throw e2;
        }
        // Stage 3 — async-statement IIFE (#79).
        // Top-level `await` is not allowed in plain script context, so when
        // the user script does not fit an expression but does contain
        // `await`, we wrap it in an async statement IIFE. The user must use
        // `return` to surface a value; otherwise the result is `null`.
        try {
          var asyncStmt = new Function(
            "return (async () => {\n" + script + "\n})()"
          );
          return asyncStmt();
        } catch (e3) {
          if (!(e3 instanceof SyntaxError)) throw e3;
          throw new SyntaxError(
            "top-level await detected but the script could not be auto-wrapped. " +
              "Wrap explicitly: (async () => { /* ...; */ return value; })() — " +
              "see docs/reference/cli.md"
          );
        }
      }
      // Stage 4 — statement fallback. Indirect eval runs in global script
      // context and returns the completion value of the last expression (#46).
      var indirectEval = eval;
      return indirectEval(script);
    }
    return expr();
  }

  // Heuristic top-level `await` detector. Strips comments and single/double
  // quoted strings, masks property accesses (`obj.await`), then peels
  // nested `function`/arrow-with-block bodies so an `await` buried in a
  // nested function does not trigger top-level detection — otherwise a
  // statement script like `async function f(){ await 1; } f(); 1+1`
  // would be mis-routed to the async-statement wrapper and lose its
  // completion value.
  //
  // Three deliberate non-strips, each documented because the alternative
  // is worse:
  //
  //   * Template literals are NOT stripped. Stripping them with a single-pass
  //     regex cannot balance nested `${...}` braces, and it also drops a real
  //     `` `${await x}` ``. Leaving them in only causes false positives on a
  //     literal like `` `await` ``, which is harmless: the script still runs
  //     wrapped in an async IIFE, only the completion-value contract changes
  //     (the user must use an explicit `return` to surface a value, which is
  //     documented in cli.md).
  //   * Regex literals (`/await/`) are NOT stripped either. A naive
  //     `\/.../[flags]*` match also swallows division expressions like
  //     `a / await foo / c`, which would silently hide a real top-level
  //     `await` and break the auto-wrap fallback. False positives from a
  //     literal `/await/` regex are again harmless wraps.
  //   * Methods inside `class` bodies are NOT recognised — the function-body
  //     strip only matches `function`/arrow blocks. A class with an `await`
  //     inside an `async` method would be flagged. Niche enough that
  //     dragging in keyword-aware parsing isn't worth it.
  //
  // For scripts larger than 100 KB the strip pass is skipped to bound
  // worst-case scan time; the raw `await` test is used instead.
  function hasTopLevelAwait(src) {
    if (src.length > 100000) return /\bawait\b/.test(src);
    // Strip quoted strings BEFORE comments, otherwise a URL like
    // `"http://example.com"` looks like a `//` line comment and the rest
    // of the line — including any real `await` — gets deleted, producing a
    // false negative. Same for `"/* not a comment */"` block markers
    // embedded in a string.
    var stripped = src
      .replace(/'(?:[^'\\]|\\.)*'/g, "''")
      .replace(/"(?:[^"\\]|\\.)*"/g, '""')
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .replace(/\/\/[^\n]*/g, "")
      .replace(/\.\s*await\b/g, ".__prop");
    // Peel innermost `function`/arrow bodies, both block-bodied
    // (`() => { ... }`) and concise (`() => expr`). Each iteration matches
    // bodies with no nested braces, so doubly-nested functions take two
    // passes. Cap the iteration count so a pathological input cannot loop
    // forever. Concise arrow bodies stop at any of `;,){}\n` to avoid
    // chewing through the rest of the script.
    for (var k = 0; k < 6; k++) {
      var prev = stripped;
      stripped = stripped
        .replace(/\bfunction\s*\*?\s*[\w$]*\s*\([^()]*\)\s*\{[^{}]*\}/g, "fn()")
        .replace(/\([^()]*\)\s*=>\s*\{[^{}]*\}/g, "fn()")
        .replace(/\b[\w$]+\s*=>\s*\{[^{}]*\}/g, "fn()")
        .replace(/\([^()]*\)\s*=>\s*[^{};,)\n]+/g, "fn()")
        .replace(/\b[\w$]+\s*=>\s*[^{};,)\n]+/g, "fn()");
      if (stripped === prev) break;
    }
    return /\bawait\b/.test(stripped);
  }

  function waitFor(options) {
    var selector = options && options.selector;
    var ref = options && options.ref;
    var gone = (options && options.gone) || false;
    // Use a `!= null` check (matching `watch` below) rather than `|| 10000` so
    // an explicit `timeout: 0` resolves immediately instead of silently
    // expanding to 10 s — the latter desynchronised the Rust channel padded
    // via `BRIDGE_TIMEOUT_BUFFER_MS` and surfaced the generic "eval timed out"
    // instead of the bridge's own rejection.
    var timeout = (options && options.timeout != null) ? options.timeout : 10000;

    if (!selector && !ref) {
      return Promise.reject(
        new Error("waitFor requires 'selector' or 'ref' (use --selector for CSS, @id for snapshot ref)")
      );
    }

    return new Promise(function (res, rej) {
      function check() {
        if (selector) return document.querySelector(selector);
        if (ref) return idMap.get(ref) || null;
        return null;
      }

      var result = gone ? { gone: true } : { found: true };
      var target = selector || ref;
      var timeoutMsg = gone
        ? "Timeout waiting for " + target + " to disappear"
        : "Timeout waiting for " + target;

      var el = check();
      if (!gone && el) return res(result);
      if (gone && !el) return res(result);

      var timer = setTimeout(function () {
        observer.disconnect();
        rej(new Error(timeoutMsg));
      }, timeout);

      var observer = new MutationObserver(function () {
        var found = check();
        if (!gone && found) {
          observer.disconnect();
          clearTimeout(timer);
          res(result);
        } else if (gone && !found) {
          observer.disconnect();
          clearTimeout(timer);
          res(result);
        }
      });

      observer.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
      });
    });
  }

  var MAX_WATCH_ENTRIES = 200;

  function summarizeNode(node) {
    var entry = { tag: node.tagName.toLowerCase() };
    if (node.id) entry.id = node.id;
    if (node.className && typeof node.className === 'string' && node.className.trim()) entry.class = node.className.trim();
    var text = Array.from(node.childNodes)
      .filter(function(n) { return n.nodeType === Node.TEXT_NODE; })
      .map(function(n) { return n.textContent || ''; })
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
    if (text) entry.text = text.substring(0, 80);
    return entry;
  }

  function watch(options) {
    var selector = options && options.selector;
    var timeout = (options && options.timeout != null) ? options.timeout : 10000;
    var stable = (options && options.stable != null) ? options.stable : 300;
    var requireMutation = !!(options && options.requireMutation);

    var root;
    if (selector) {
      root = document.querySelector(selector);
      if (!root) throw new Error("watch: no element matches selector: " + selector);
    } else {
      root = document.body;
    }

    return new Promise(function (res, rej) {
      var changes = { added: [], removed: [], modified: [], truncated: false };
      var stableTimer = null;
      var timeoutTimer = null;
      var settled = false;

      function finish() {
        if (settled) return;
        settled = true;
        clearTimeout(timeoutTimer);
        observer.disconnect();
        res(changes);
      }

      function resetStableTimer() {
        clearTimeout(stableTimer);
        stableTimer = setTimeout(finish, stable);
      }

      timeoutTimer = setTimeout(function () {
        if (settled) return;
        settled = true;
        clearTimeout(stableTimer);
        observer.disconnect();
        if (changes.added.length > 0 || changes.removed.length > 0 || changes.modified.length > 0) {
          res(changes);
        } else {
          rej(new Error("watch timeout: no DOM changes within " + timeout + "ms"));
        }
      }, timeout);

      // With requireMutation we skip starting the stable timer until the first
      // mutation is seen; without it we start immediately so stable windows can
      // resolve even when the DOM is idle.
      if (!requireMutation) {
        resetStableTimer();
      }

      function pushCapped(arr, entry) {
        if (arr.length < MAX_WATCH_ENTRIES) {
          arr.push(entry);
        } else {
          changes.truncated = true;
        }
      }

      var observer = new MutationObserver(function (mutations) {
        for (var i = 0; i < mutations.length; i++) {
          var mutation = mutations[i];
          if (mutation.type === 'childList') {
            for (var j = 0; j < mutation.addedNodes.length; j++) {
              var node = mutation.addedNodes[j];
              if (node.nodeType === Node.ELEMENT_NODE) {
                pushCapped(changes.added, summarizeNode(node));
              }
            }
            for (var k = 0; k < mutation.removedNodes.length; k++) {
              var removedNode = mutation.removedNodes[k];
              if (removedNode.nodeType === Node.ELEMENT_NODE) {
                pushCapped(changes.removed, summarizeNode(removedNode));
              }
            }
          } else if (mutation.type === 'attributes') {
            var target = mutation.target;
            var attrValue = target.getAttribute(mutation.attributeName);
            var entry = {
              tag: target.tagName.toLowerCase(),
              attribute: mutation.attributeName,
            };
            if (attrValue === null) {
              entry.removed = true;
            } else {
              entry.value = attrValue;
            }
            pushCapped(changes.modified, entry);
          } else if (mutation.type === 'characterData') {
            var parent = mutation.target.parentElement;
            if (parent) {
              pushCapped(changes.modified, {
                tag: parent.tagName.toLowerCase(),
                text: (mutation.target.textContent || '').replace(/\s+/g, ' ').trim().substring(0, 80),
              });
            }
          }
        }
        resetStableTimer();
      });

      observer.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        characterData: true,
      });
    });
  }

  // html-to-image copies computed styles onto the clone. When
  // `getComputedStyle(el).cssText` is empty — WebKit and Blink both — it falls
  // back to one `setProperty` per name in `getComputedStyle(documentElement)`,
  // which on a Tailwind v4 page is ~2200 names (~1760 of them custom
  // properties). WebKit re-serializes the whole style attribute on every
  // `setProperty`, so that copy is quadratic: minutes of 100% CPU on a few
  // hundred nodes, and the webview stays wedged past the RPC timeout (#146).
  // Custom properties are dead weight here — computed values arrive with their
  // `var()` already resolved — so we hand html-to-image the painted properties
  // only. Anything not in this list is lost from the capture.
  var STYLE_PROPERTIES = [
    // Box + layout
    "display", "position", "top", "right", "bottom", "left", "float", "clear",
    "z-index", "width", "height", "min-width", "min-height", "max-width",
    "max-height", "box-sizing", "aspect-ratio", "margin-top", "margin-right",
    "margin-bottom", "margin-left", "padding-top", "padding-right",
    "padding-bottom", "padding-left", "overflow-x", "overflow-y", "visibility",
    "opacity", "vertical-align", "content-visibility", "clip",
    // Flex + grid
    "flex-direction", "flex-wrap", "flex-grow", "flex-shrink", "flex-basis",
    "justify-content", "justify-items", "justify-self", "align-content",
    "align-items", "align-self", "order", "row-gap", "column-gap",
    "grid-template-columns", "grid-template-rows", "grid-template-areas",
    "grid-auto-flow", "grid-auto-columns", "grid-auto-rows",
    "grid-column-start", "grid-column-end", "grid-row-start", "grid-row-end",
    "column-count", "column-width",
    // Background + border
    "background-color", "background-image", "background-position",
    "background-size", "background-repeat", "background-clip",
    "background-origin", "background-attachment", "-webkit-background-clip",
    "border-top-width", "border-right-width", "border-bottom-width",
    "border-left-width", "border-top-style", "border-right-style",
    "border-bottom-style", "border-left-style", "border-top-color",
    "border-right-color", "border-bottom-color", "border-left-color",
    "border-top-left-radius", "border-top-right-radius",
    "border-bottom-right-radius", "border-bottom-left-radius",
    "border-collapse", "border-spacing", "table-layout", "outline-color",
    "outline-style", "outline-width", "outline-offset", "box-shadow",
    "border-image-source", "border-image-slice", "border-image-width",
    "border-image-outset", "border-image-repeat",
    // Paint effects
    "filter", "backdrop-filter", "mix-blend-mode", "background-blend-mode",
    "isolation", "clip-path", "mask-image", "mask-size", "mask-position",
    "mask-repeat", "mask-mode", "mask-composite", "transform",
    "transform-origin", "transform-style", "translate", "rotate", "scale",
    "perspective", "perspective-origin", "backface-visibility",
    // Text
    "color", "font-family", "font-size", "font-weight", "font-style",
    "font-variant", "font-stretch", "font-feature-settings",
    "font-variation-settings", "line-height", "letter-spacing", "word-spacing",
    "text-align", "text-indent", "text-transform", "text-shadow",
    "text-overflow", "text-decoration-line", "text-decoration-color",
    "text-decoration-style", "text-decoration-thickness",
    "text-underline-offset", "-webkit-text-fill-color",
    "-webkit-text-stroke-width", "-webkit-text-stroke-color",
    "-webkit-text-security", "-webkit-font-smoothing",
    "-webkit-line-clamp", "-webkit-box-orient", "white-space", "word-break",
    "overflow-wrap", "hyphens", "direction", "unicode-bidi", "writing-mode",
    "text-orientation", "tab-size", "list-style-type", "list-style-position",
    "list-style-image", "content", "counter-reset", "counter-increment",
    "counter-set",
    // Replaced content + SVG
    "object-fit", "object-position", "image-rendering", "fill", "fill-opacity",
    "fill-rule", "stroke", "stroke-width", "stroke-opacity", "stroke-linecap",
    "stroke-linejoin", "stroke-dasharray", "stroke-dashoffset", "clip-rule",
    "stop-color", "stop-opacity", "d", "text-anchor", "dominant-baseline",
    "paint-order", "marker-start", "marker-mid", "marker-end",
    // Form controls
    "appearance", "-webkit-appearance", "accent-color",
  ];

  async function screenshot(options) {
    var selector = options && options.selector;
    var el = selector ? document.querySelector(selector) : document.documentElement;
    if (!el) throw new Error("Element not found: " + selector);
    if (typeof htmlToImage === "undefined" || !htmlToImage.toPng) {
      throw new Error("html-to-image library not loaded. Bundle it into bridge.js for screenshot support.");
    }
    var renderOptions = { pixelRatio: 1, includeStyleProperties: STYLE_PROPERTIES };
    if (!selector) {
      // html-to-image sizes the capture from clientWidth/clientHeight, which
      // for documentElement is the viewport — the render always starts at the
      // document origin, so anything below the fold is silently cropped
      // (#129). Pass the full scroll dimensions to capture the whole page.
      var body = document.body;
      renderOptions.width = Math.max(el.scrollWidth || 0, body ? body.scrollWidth || 0 : 0);
      renderOptions.height = Math.max(el.scrollHeight || 0, body ? body.scrollHeight || 0 : 0);
    }
    var dataUrl = await htmlToImage.toPng(el, renderOptions);
    return dataUrl;
  }

  function storageGet(params) {
    if (typeof params.key !== "string") {
      throw new Error("storageGet requires a string key");
    }
    var storage = params.session ? sessionStorage : localStorage;
    var val = storage.getItem(params.key);
    if (val === null) {
      return { found: false };
    }
    return { found: true, value: val };
  }

  function storageSet(params) {
    if (typeof params.key !== "string" || typeof params.value !== "string") {
      throw new Error("storageSet requires string key and value");
    }
    var storage = params.session ? sessionStorage : localStorage;
    storage.setItem(params.key, params.value);
    return { ok: true };
  }

  var MAX_STORAGE_ENTRIES = 500;

  function storageList(params) {
    var storage = params.session ? sessionStorage : localStorage;
    var total = storage.length;
    var len = Math.min(total, MAX_STORAGE_ENTRIES);
    var entries = [];
    for (var i = 0; i < len; i++) {
      var key = storage.key(i);
      entries.push({ key: key, value: storage.getItem(key) });
    }
    entries.sort(function (a, b) {
      return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
    });
    return { entries: entries, truncated: total > MAX_STORAGE_ENTRIES };
  }

  function storageClear(params) {
    var storage = params.session ? sessionStorage : localStorage;
    storage.clear();
    return { cleared: true };
  }

  var MAX_FORMS = 100;
  var MAX_FIELDS_PER_FORM = 500;

  function formDump(params) {
    var forms;
    var totalForms;
    if (params && params.selector) {
      var found = document.querySelector(params.selector);
      if (!found) {
        throw new Error("Form not found: " + params.selector);
      }
      if (found.tagName.toLowerCase() !== "form") {
        throw new Error("Selector matched a <" + found.tagName.toLowerCase() + ">, expected a <form>");
      }
      forms = [found];
      totalForms = 1;
    } else {
      var all = document.querySelectorAll("form");
      totalForms = all.length;
      forms = [];
      var formLimit = Math.min(totalForms, MAX_FORMS);
      for (var fi = 0; fi < formLimit; fi++) {
        forms.push(all[fi]);
      }
    }

    var result = [];
    for (var i = 0; i < forms.length; i++) {
      var form = forms[i];
      var fields = [];
      var elements = form.querySelectorAll("input, select, textarea");
      var fieldLimit = Math.min(elements.length, MAX_FIELDS_PER_FORM);
      for (var j = 0; j < fieldLimit; j++) {
        var el = elements[j];
        var tag = el.tagName.toLowerCase();
        var elType = el.type || null;
        var fieldVal;
        if (tag === "select" && el.multiple) {
          fieldVal = selectedOptionValues(el);
        } else {
          fieldVal = el.value;
        }
        var field = {
          tag: tag,
          type: elType,
          name: el.name || "",
          value: fieldVal,
        };
        if (elType === "checkbox" || elType === "radio") {
          field.checked = el.checked;
        }
        fields.push(field);
      }
      var formEntry = {
        id: form.id || "",
        name: form.getAttribute("name") || "",
        action: form.action || "",
        method: form.method || "get",
        fields: fields,
      };
      if (elements.length > MAX_FIELDS_PER_FORM) {
        formEntry.fieldsTruncated = true;
      }
      result.push(formEntry);
    }
    var truncated = totalForms > MAX_FORMS;
    return { forms: result, truncated: truncated };
  }

  window.__PILOT__ = {
    snapshot: snapshot,
    resolve: resolve,
    click: click,
    fill: fill,
    type: typeText,
    select: select,
    check: check,
    scroll: scroll,
    text: text,
    html: html,
    value: value,
    attrs: attrs,
    navigate: navigate,
    url: url,
    title: title,
    state: state,
    eval: evalScript,
    wait: waitFor,
    screenshot: screenshot,
    consoleLogs: consoleLogs,
    clearLogs: clearLogs,
    networkRequests: networkRequests,
    clearNetwork: clearNetwork,
    visible: visible,
    count: count,
    checked: checked,
    watch: watch,
    drag: drag,
    drop: drop,
    storageGet: storageGet,
    storageSet: storageSet,
    storageList: storageList,
    storageClear: storageClear,
    formDump: formDump,
  };

  // Tell the plugin this origin can answer (#153). The ACL denies
  // `__callback` to origins without the pilot permission, and a denied page
  // can run the bridge but never deliver a result, so its silence here is the
  // signal. Id 0 is HELLO_ID in eval.rs, never used by an eval request.
  try {
    window.__TAURI_INTERNALS__
      .invoke("plugin:pilot|__callback", { id: 0, result: location.href })
      .catch(function() {});
  } catch (_) {}
})();
