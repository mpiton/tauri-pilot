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

  function click(params) {
    const el = resolveTarget(params);
    el.focus();
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return { ok: true };
  }

  function fill(params) {
    const el = resolveTarget(params);
    el.focus();
    const setter =
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value") ||
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    if (setter && setter.set) {
      setter.set.call(el, params.value);
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
    const setter =
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value") ||
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    for (const ch of params.text) {
      el.dispatchEvent(new KeyboardEvent("keydown", { key: ch, bubbles: true }));
      if (setter && setter.set) {
        setter.set.call(el, el.value + ch);
      } else {
        el.value += ch;
      }
      el.dispatchEvent(new InputEvent("input", { data: ch, inputType: "insertText", bubbles: true }));
      el.dispatchEvent(new KeyboardEvent("keyup", { key: ch, bubbles: true }));
    }
    return { ok: true };
  }

  function press(params) {
    var key = params.key || params;
    const target = document.activeElement || document.body;
    target.dispatchEvent(new KeyboardEvent("keydown", { key: key, bubbles: true }));
    target.dispatchEvent(new KeyboardEvent("keyup", { key: key, bubbles: true }));
    return { ok: true };
  }

  function select(params) {
    const el = resolveTarget(params);
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value");
    if (setter && setter.set) {
      setter.set.call(el, params.value);
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
    const dx = (dir === "left" ? -amount : dir === "right" ? amount : 0);
    const dy = (dir === "up" ? -amount : dir === "down" ? amount : 0);
    target.scrollBy(dx, dy);
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
    return new Function("return (" + script + ")")();
  }

  function waitFor(options) {
    var selector = options && options.selector;
    var ref = options && options.target;
    var gone = (options && options.gone) || false;
    var timeout = (options && options.timeout) || 10000;

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
  };
})();
