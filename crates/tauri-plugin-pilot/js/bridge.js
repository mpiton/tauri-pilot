(() => {
  "use strict";

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

  const _originalFetch = window.fetch.bind(window);
  window.fetch = function(input, init) {
    const method = (init && init.method) || (input && input.method) || "GET";
    const url = (typeof input === "string") ? input : (input && input.url) || String(input);
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
        if (node.value !== undefined && node.value !== "") entry.value = node.value;
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

  function fill(params) {
    const el = resolveTarget(params);
    el.focus();
    const setter = nativeValueSetter(el);
    if (setter) {
      setter.call(el, params.value);
    } else {
      el.value = params.value;
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function typeText(params) {
    const el = resolveTarget(params);
    el.focus();
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
    const setter = nativeValueSetter(el);
    if (setter) {
      setter.call(el, params.value);
    } else {
      el.value = params.value;
    }
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function check(params) {
    const el = resolveTarget(params);
    el.checked = !el.checked;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function scroll(options) {
    const dir = (options && options.direction) || "down";
    const amount = (options && options.amount) || 300;
    const ref = options && options.ref;
    const target = ref ? requireEl(ref) : window;

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

  function drag(params) {
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
      endX = startX + (params.offset.x || 0);
      endY = startY + (params.offset.y || 0);
      dropTarget = document.elementFromPoint(endX, endY);
      if (!dropTarget) throw new Error("No element at offset (" + params.offset.x + "," + params.offset.y + ")");
    } else {
      throw new Error("drag requires target or offset");
    }

    var dt = typeof DataTransfer === "function" ? new DataTransfer() : new ClipboardEvent("").clipboardData;
    source.dispatchEvent(new MouseEvent("mousedown", { clientX: startX, clientY: startY, bubbles: true }));
    source.dispatchEvent(new DragEvent("dragstart", { clientX: startX, clientY: startY, dataTransfer: dt, bubbles: true }));
    source.dispatchEvent(new DragEvent("dragleave", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true }));
    dropTarget.dispatchEvent(new DragEvent("dragenter", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true }));
    dropTarget.dispatchEvent(new DragEvent("dragover", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true }));
    dropTarget.dispatchEvent(new DragEvent("drop", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true, cancelable: true }));
    source.dispatchEvent(new DragEvent("dragend", { clientX: endX, clientY: endY, dataTransfer: dt, bubbles: true }));
    return { ok: true };
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

  function value(params) {
    return resolveTarget(params).value || "";
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

      var el = check();
      if (!gone && el) return res({ found: true });
      if (gone && !el) return res({ found: true });

      var timer = setTimeout(function () {
        observer.disconnect();
        rej(new Error("Timeout waiting for " + (selector || ref)));
      }, timeout);

      var observer = new MutationObserver(function () {
        var found = check();
        if (!gone && found) {
          observer.disconnect();
          clearTimeout(timer);
          res({ found: true });
        } else if (gone && !found) {
          observer.disconnect();
          clearTimeout(timer);
          res({ found: true });
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

  async function screenshot(options) {
    var selector = options && options.selector;
    var el = selector ? document.querySelector(selector) : document.documentElement;
    if (!el) throw new Error("Element not found: " + selector);
    if (typeof htmlToImage === "undefined" || !htmlToImage.toPng) {
      throw new Error("html-to-image library not loaded. Bundle it into bridge.js for screenshot support.");
    }
    var dataUrl = await htmlToImage.toPng(el, { pixelRatio: 1 });
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
          var selected = [];
          for (var k = 0; k < el.options.length; k++) {
            if (el.options[k].selected) {
              selected.push(el.options[k].value);
            }
          }
          fieldVal = selected;
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
})();
