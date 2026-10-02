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

  // Caps for a logged value, so one call cannot hold a huge graph in the
  // 500-entry buffer: nesting depth, items kept per container, objects copied
  // per argument in all, and characters kept per string inside a container.
  const LOG_MAX_DEPTH = 8;
  const LOG_MAX_ITEMS = 100;
  const LOG_MAX_OBJECTS = 1000;
  const LOG_MAX_STRING = 10000;

  // Console arguments are copied when they are logged, not when `logs` reads
  // them: a live reference shows whatever state the object has by then and
  // keeps it (and any DOM node it reaches) alive in the buffer (#274).
  function serializeArg(arg) {
    if (typeof arg === 'string') return arg;
    try {
      return snapshotValue(arg, 0, { ancestors: [], objectsLeft: LOG_MAX_OBJECTS });
    } catch (_) {
      return '[unprintable]';
    }
  }

  // `[object Tag]` for `value`, or '' when even that throws (a revoked proxy).
  function brandTag(value) {
    try {
      return Object.prototype.toString.call(value);
    } catch (_) {
      return '';
    }
  }

  // 'Map', 'Set' or null. Brand checks through the size getters work across
  // realms and cannot be faked with Symbol.toStringTag.
  const _mapSize = Object.getOwnPropertyDescriptor(Map.prototype, 'size').get;
  const _setSize = Object.getOwnPropertyDescriptor(Set.prototype, 'size').get;
  const _regExpSource = Object.getOwnPropertyDescriptor(RegExp.prototype, 'source').get;
  // The %TypedArray% intrinsics read internal slots: the tag getter answers
  // the constructor name for a typed array from any realm and undefined for
  // anything else (a DataView included), whatever Symbol.toStringTag or an
  // own `length` claim.
  const _typedArrayProto = Object.getPrototypeOf(Uint8Array.prototype);
  const _typedArrayTag = Object.getOwnPropertyDescriptor(_typedArrayProto, Symbol.toStringTag).get;
  const _typedArrayLength = Object.getOwnPropertyDescriptor(_typedArrayProto, 'length').get;
  function typedArrayName(value) {
    try {
      return _typedArrayTag.call(value) || null;
    } catch (_) {
      return null;
    }
  }

  function collectionKind(value) {
    // A failed brand check throws, and throwing for every plain object made
    // each logged object cost far more than copying it. The tag is only a
    // filter: the getters below still decide.
    const tag = brandTag(value);
    if (tag !== '[object Map]' && tag !== '[object Set]') return null;
    try {
      _mapSize.call(value);
      return 'Map';
    } catch (_) {}
    try {
      _setSize.call(value);
      return 'Set';
    } catch (_) {}
    return null;
  }

  // The node type, or null for anything that is not a DOM node. Calling the
  // Node.prototype getter is a brand check: it works for a node from an
  // iframe, which fails `instanceof Node`, and throws for an object that
  // merely has a `nodeType` field.
  function domNodeType(value) {
    try {
      if (typeof Node !== 'function') return null;
      // Same filter as collectionKind: skip the throwing getter for values
      // that cannot be nodes.
      if (!('nodeType' in value)) return null;
      const desc = Object.getOwnPropertyDescriptor(Node.prototype, 'nodeType');
      if (!desc || typeof desc.get !== 'function') return null;
      const type = desc.get.call(value);
      return typeof type === 'number' ? type : null;
    } catch (_) {
      return null;
    }
  }

  // `<button#save.primary>` for an element, the node name (`#text`,
  // `#document`) for any other node.
  function describeNode(node, type) {
    if (type !== 1) return String(node.nodeName);
    let text = '<' + String(node.localName || node.nodeName).toLowerCase();
    if (node.id) text += '#' + node.id;
    // SVG elements carry an SVGAnimatedString in className.
    const cls = typeof node.getAttribute === 'function' ? node.getAttribute('class') : node.className;
    if (typeof cls === 'string') {
      cls.split(/\s+/).forEach(c => { if (c) text += '.' + c; });
    }
    return text + '>';
  }

  // JSON-safe copy of `value`. Errors, nodes and other values JSON would turn
  // into {} or drop become short strings, Map and Set become tagged objects,
  // and the same rules apply at every level. `state.ancestors` holds the
  // objects on the current path, so a cycle is cut but a shared reference is
  // copied at each place it appears.
  // `key` is the property name (or array index) `value` was read from, '' at
  // the top: JSON.stringify hands it to toJSON, so it is passed on too.
  function snapshotValue(value, depth, state, key) {
    if (value === null) return null;
    if (value === undefined) return 'undefined';
    const type = typeof value;
    if (type === 'string') return capString(value);
    if (type === 'boolean') return value;
    // JSON has no NaN or Infinity and would write null.
    if (type === 'number') return Number.isFinite(value) ? value : String(value);
    if (type === 'bigint') return String(value) + 'n';
    if (type === 'symbol') return capString(String(value));
    if (type === 'function') return capString('[Function' + (value.name ? ' ' + value.name : '') + ']');
    // Same guard as describeReason: an object wearing only the Error tag is
    // copied as an object.
    const error = isError(value) && (value.name !== undefined || value.message !== undefined);
    const label = error ? capString(describeReason(value)) : null;
    let errorKeys = null;
    if (error) {
      errorKeys = Object.keys(value);
      // `new Error(msg, { cause })` makes cause an own non-enumerable field.
      if (errorKeys.indexOf('cause') === -1 && Object.prototype.hasOwnProperty.call(value, 'cause')) {
        errorKeys.push('cause');
      }
      // A bare Error stays a plain "Name: message" string.
      if (errorKeys.length === 0) return label;
    } else {
      const nodeType = domNodeType(value);
      if (nodeType !== null) return capString(describeNode(value, nodeType));
    }
    if (state.ancestors.indexOf(value) !== -1) return '[Circular]';
    const kind = error ? null : collectionKind(value);
    if (depth >= LOG_MAX_DEPTH || state.objectsLeft <= 0) {
      if (error) return label;
      if (kind) return '[' + kind + ']';
      return Array.isArray(value) ? '[Array]' : '[Object]';
    }
    state.objectsLeft--;
    state.ancestors.push(value);
    try {
      if (error) {
        // An Error's own fields (`code`, `status`, `cause`) are what JSON
        // used to carry, so they are kept next to the "Name: message" lead.
        const out = { __type: 'Error', message: label };
        copyKeys(out, value, errorKeys.filter(k => k !== '__type' && k !== 'message'), depth + 1, state);
        return out;
      }
      return snapshotObject(value, depth + 1, state, kind, key === undefined ? '' : key);
    } finally {
      state.ancestors.pop();
    }
  }

  // `value` cut after LOG_MAX_STRING characters, with the count it dropped.
  function capString(value) {
    if (value.length <= LOG_MAX_STRING) return value;
    // A V8 slice keeps its whole parent alive, so a 10 MB string cut to
    // 10,000 characters would still hold 10 MB in the buffer. Joining the
    // characters builds a flat copy that does not reference the parent.
    const head = Array.prototype.join.call(value.slice(0, LOG_MAX_STRING), '');
    return head + '... (' + (value.length - LOG_MAX_STRING) + ' more chars)';
  }

  // Copies `keys` of `value` (first LOG_MAX_ITEMS) into `out`, with a '...'
  // marker for the rest.
  function copyKeys(out, value, keys, depth, state) {
    for (let i = 0; i < keys.length && i < LOG_MAX_ITEMS; i++) {
      let v;
      try {
        v = snapshotValue(value[keys[i]], depth, state, keys[i]);
      } catch (_) {
        v = '[unreadable]';
      }
      // A plain assignment of an own `__proto__` key would hit the inherited
      // setter and drop the field.
      Object.defineProperty(out, keys[i], { value: v, enumerable: true, configurable: true, writable: true });
    }
    if (keys.length > LOG_MAX_ITEMS) out['...'] = (keys.length - LOG_MAX_ITEMS) + ' more keys';
  }

  function snapshotObject(value, depth, state, kind, key) {
    const copy = (v, k) => {
      try {
        return snapshotValue(v, depth, state, k);
      } catch (_) {
        return '[unreadable]';
      }
    };
    if (kind) {
      const isMapValue = kind === 'Map';
      const size = (isMapValue ? _mapSize : _setSize).call(value);
      const iter = (isMapValue ? Map.prototype.entries : Set.prototype.values).call(value);
      const items = [];
      // Stop at the cap rather than walking every entry.
      for (let step = iter.next(); !step.done && items.length < LOG_MAX_ITEMS; step = iter.next()) {
        items.push(isMapValue ? [copy(step.value[0]), copy(step.value[1])] : copy(step.value));
      }
      const out = isMapValue
        ? { __type: 'Map', size: size, entries: items }
        : { __type: 'Set', size: size, values: items };
      if (size > items.length) out.truncated = size - items.length;
      return out;
    }
    if (Array.isArray(value)) {
      const len = value.length;
      const out = [];
      // A hole is null, as JSON.stringify writes it, so it stays distinct
      // from a real undefined.
      for (let i = 0; i < len && i < LOG_MAX_ITEMS; i++) out.push(i in value ? copy(value[i], String(i)) : null);
      if (len > LOG_MAX_ITEMS) out.push('... ' + (len - LOG_MAX_ITEMS) + ' more items');
      return out;
    }
    // Dates and other objects that define their own JSON form keep it; one
    // whose toJSON (or toJSON getter) throws is copied key by key instead.
    let toJSON;
    try {
      toJSON = value.toJSON;
    } catch (_) {}
    if (typeof toJSON === 'function') {
      try {
        return copy(toJSON.call(value, key), key);
      } catch (_) {}
    }
    const tag = brandTag(value);
    // A typed array is checked before Object.keys, which would list every
    // index of a large one.
    const typedName = typedArrayName(value);
    if (typedName) {
      const len = _typedArrayLength.call(value);
      const values = [];
      for (let i = 0; i < len && i < LOG_MAX_ITEMS; i++) values.push(copy(value[i], String(i)));
      const out = { __type: typedName, length: len, values: values };
      if (len > values.length) out.truncated = len - values.length;
      return out;
    }
    const keys = Object.keys(value);
    // Built-ins that keep their state in internal slots have no keys and
    // would be {}: say what they are instead.
    if (keys.length === 0 && tag !== '[object Object]' && tag !== '') {
      if (tag === '[object RegExp]') {
        // The source getter is the brand check: it throws for an object that
        // only wears the RegExp tag.
        try {
          _regExpSource.call(value);
          return capString(RegExp.prototype.toString.call(value));
        } catch (_) {}
      }
      return capString('[' + tag.slice(8, -1) + ']');
    }
    const out = {};
    copyKeys(out, value, keys, depth, state);
    return out;
  }

  function extractSource() {
    try {
      const stack = new Error().stack;
      if (!stack) return null;
      // Skip the two location-bearing frames that are always ours:
      // extractSource itself and the console[level] wrapper.
      return firstAppFrame(stack, 2);
    } catch (_) { return null; }
  }

  const _originalConsole = {
    log: console.log.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
    info: console.info.bind(console),
  };

  function pushLog(level, args, source) {
    const entry = {
      id: ++_logIdCounter,
      timestamp: Date.now(),
      level: level,
      args: args.map(serializeArg),
      source: source || null,
    };
    _logs.push(entry);
    if (_logs.length > MAX_LOGS) _logs.shift();
    return entry;
  }

  // Capture has to survive page code that replaces console.* later. A plain
  // `console[level] = wrapper` is dropped the moment anything assigns over it
  // without chaining, which extension-heavy apps do routinely -- and then
  // `logs` stays empty for the rest of the session with no way to tell.
  //
  // Installing an accessor instead keeps capture permanently in front and
  // reads an assignment as "call this next", so replacements stack. Reading
  // console[level] hands back an entry point for the chain as it stands right
  // then; a replacement that saved an earlier reference keeps the one it
  // saved, so calling it continues down the chain from where that replacement
  // sits instead of re-entering at the top.
  //
  // That is what makes the ordinary "save it and call it back" shape
  // terminate, and it has to hold when the call back happens from a timer or
  // a promise, long after the original call returned. A re-entry counter
  // cannot do that part: by then it has unwound, the deferred call looks like
  // a fresh one, and it goes around the loop again.
  //
  // Recording is a separate question. A call that arrives while a view is
  // already running is part of a call recorded further up the stack -- a
  // wrapper handing on to what it saved, or one level forwarding to another
  // -- so only the outermost call records. Any other call is a fresh line,
  // including a logger's `const log = console.log` from init after the page
  // replaced console.log. A saved reference called back from a timer records
  // a second time: from outside, it is indistinguishable from that logger.
  //
  // Recognising Pilot's own functions off a marker property would take the
  // page's word for it: page code can set the marker too, so a replacement
  // could claim to be one and get itself dropped or filed under another
  // level. Identity in a map the page cannot reach is not forgeable. Each
  // view maps to its chain and depth, which is all it takes to name the
  // function it calls.
  const _consoleViews = new WeakMap();
  // Pilot views currently on the stack, across every level.
  let _consoleActive = 0;
  const _consoleHeals = [];

  ['log', 'warn', 'error', 'info'].forEach(level => {
    // Entries are never removed or reordered, since a saved view may still
    // route through any of them. Assigning a function already in the chain
    // reuses its entry, so only distinct functions grow it. The ceiling is a
    // normalizer that re-binds on every pass (`console.log =
    // console.log.bind(console)`): each bind is a new function, so each pass
    // adds an entry -- the same extra layer it would add without Pilot.
    const chain = [_originalConsole[level]];
    // Depth of the view the getter hands out.
    let current = 1;
    const views = [];

    // One view per chain depth, created once, so identity stays stable:
    // `console.log === console.log` still holds, and a saved reference keeps
    // pointing at the same function.
    function viewAt(depth) {
      if (views[depth]) return views[depth];
      const view = function(...args) {
        const outermost = _consoleActive === 0;
        _consoleActive++;
        try {
          // extractSource() must be called from this frame: it skips the two
          // location-bearing frames that are always ours (itself and this view).
          if (outermost) pushLog(level, args, extractSource());
          return chain[depth - 1].apply(console, args);
        } finally {
          _consoleActive--;
        }
      };
      _consoleViews.set(view, { chain, depth });
      views[depth] = view;
      return view;
    }

    const accessor = {
      configurable: true,
      enumerable: true,
      get() { return viewAt(current); },
      set(next) {
        if (typeof next !== 'function') return;
        // A view stands for the function it calls, never for itself:
        // stacking one would file a call twice, or under two levels. Resolve
        // it through its own chain and depth, not the current tail, so
        // `console.log = saved` puts back exactly what `saved` called, and
        // `console.log = console.warn` takes whatever warn's view ran --
        // stale or not. An entry never changes, so pointing at one can never
        // close a loop between two aliased levels.
        const owner = _consoleViews.get(next);
        const target = owner ? owner.chain[owner.depth - 1] : next;
        const at = chain.indexOf(target);
        if (at === -1) chain.push(target);
        current = at === -1 ? chain.length : at + 1;
      },
    };

    function install() {
      Object.defineProperty(console, level, accessor);
    }

    try {
      install();
    } catch (_) {
      // Unconfigurable console: fall back to the plain assignment, which is
      // still better than no capture at all.
      try {
        console[level] = viewAt(1);
      } catch (_) {
        // Frozen console, and the file is strict, so the assignment throws
        // too. Losing capture on one level is bad; letting it abort the IIFE
        // would leave no window.__PILOT__ at all and take the whole plugin
        // down with it. Record the loss so `logs` can explain an empty buffer
        // instead of repeating the silence this change exists to remove.
        pushLog('error', ['tauri-pilot: console.' + level + ' capture unavailable (console is frozen)'], null);
      }
    }

    // configurable has to stay true (React's dev build redefines console.*),
    // so page code can still replace the accessor with a plain data property.
    // React does exactly that while it builds a component stack, then puts
    // back what it read, and the next plain assignment displaces capture as
    // in #190. Nothing fires on a redefine, so heal whenever the buffer is
    // read: chain whatever is installed now and put the accessor back. Lines
    // logged in between are lost.
    _consoleHeals.push(() => {
      try {
        const descriptor = Object.getOwnPropertyDescriptor(console, level);
        if (descriptor && descriptor.get === accessor.get) return;
        accessor.set(console[level]);
        try {
          install();
        } catch (_) {
          // Unconfigurable: any page assignment replaces the plain fallback,
          // so put a view back the same way.
          console[level] = viewAt(current);
        }
      } catch (_) {
        // Frozen: nothing can be written back.
      }
    });
  });

  function healConsole() {
    _consoleHeals.forEach(heal => heal());
  }

  function isError(value) {
    // instanceof is realm-bound, so an Error thrown from an iframe fails it.
    // The brand check catches those; Error subclasses keep the same tag.
    try {
      if (value instanceof Error) return true;
    } catch (_) {
      return false;
    }
    try {
      return Object.prototype.toString.call(value) === '[object Error]';
    } catch (_) {
      return false;
    }
  }

  function describeReason(reason) {
    try {
      // Errors carry nothing enumerable, so JSON.stringify would render even a
      // perfectly readable one as "{}".
      if (isError(reason)) {
        const name = reason.name;
        const message = reason.message;
        // Symbol.toStringTag is writable, so the brand check alone would
        // promote any object wearing the tag to "undefined: undefined". Take
        // the shortcut only when the fields exist — not when they are strings:
        // a subclass is free to put a number in `message`.
        if (name !== undefined || message !== undefined) {
          const label = name === undefined ? 'Error' : String(name);
          const text = message === undefined ? '' : String(message);
          return text ? label + ': ' + text : label;
        }
      }
      if (typeof reason === 'string') return reason;
      // JSON.stringify answers undefined for a symbol, function or undefined,
      // and "null" for NaN and Infinity. String() keeps all of those legible.
      if (reason === null || typeof reason !== 'object') return String(reason);
      try {
        const json = JSON.stringify(reason);
        return typeof json === 'string' ? json : String(reason);
      } catch (_) {
        return String(reason);
      }
    } catch (_) {
      return '[unprintable]';
    }
  }

  function firstAppFrame(stack, skipLocationFrames) {
    const lines = stack.split('\n');
    let skipped = 0;
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      // V8 opens with "Name: message" (possibly multi-line); JavaScriptCore
      // (WebKitGTK, WKWebView) starts at the throwing frame. V8 frames are
      // indented (`    at ...`); an unindented `at fake.js:12:5` is message
      // text. JSC is `@url:line:col` or `fn@url:line:col` (anonymous has no
      // name). Match V8 on the raw line so trim cannot invent indentation.
      const isV8Frame = /^\s+at\s+.*:\d+:\d+\)?$/.test(lines[i]);
      const isJscFrame = /^(?:[^:]*@.*):\d+:\d+\)?$/.test(line);
      if (!isV8Frame && !isJscFrame) continue;
      // The eval wrapper (WRAPPER_NAME in eval.rs). JavaScriptCore writes
      // eval'd frames with no location and ignores `//# sourceURL`, so
      // reaching the wrapper means the call came from a script pilot sent,
      // not from the app (#245). `__PILOT__evalScript` frames sit on the way
      // there and fall to the `__PILOT__` skip: JSC keeps one when a stage
      // calls the script inside `try`, and V8 tags the eval'd code itself
      // `eval at __PILOT__evalScript`.
      if (line.includes('__PILOT_EVAL__')) return 'tauri-pilot-eval';
      if (line.includes('__PILOT__')) continue;
      if (skipped < skipLocationFrames) {
        skipped++;
        continue;
      }
      // An anonymous JSC frame is `@url:line:col`. The empty function name
      // leaves a leading `@` that reads as noise in `logs` output, and an
      // eval'd frame carries no url at all — WebKitGTK writes `@undefined:1:91`
      // or `@:1:91` there. Both report `line:col`, like `eventSource` does.
      return line.charAt(0) === '@' ? line.slice(1).replace(/^(?:undefined)?:/, '') : line;
    }
    return null;
  }

  // WebKitGTK reports the *string* "undefined" as `filename` for code that
  // came from eval, so a truthiness check lets `undefined:1:91` through.
  function eventSource(event) {
    const name = event.filename;
    const file = (typeof name === 'string' && name && name !== 'undefined') ? name + ':' : '';
    const lineno = event.lineno || 0;
    const colno = event.colno || 0;
    if (!file && !lineno && !colno) return null;
    return file + lineno + ':' + colno;
  }

  function stackSource(error) {
    if (!error) return null;
    try {
      const stack = error.stack;
      if (typeof stack !== 'string') return null;
      return firstAppFrame(stack, 0);
    } catch (_) {
      return null;
    }
  }

  // Uncaught errors and unhandled rejections never pass through console.* --
  // the browser prints those itself. Without these listeners the documented
  // `logs --level error` workflow cannot see the failures it exists to find.
  if (typeof window.addEventListener === 'function') {
    window.addEventListener('error', event => {
      try {
        // Only script errors carry a message. A failed image or script
        // resource load (HTTP 404) raises a bare Event that does not bubble
        // to window, but guard anyway rather than logging an empty entry.
        if (!event || typeof event.message !== 'string') return;
        pushLog('error', [event.message], stackSource(event.error) || eventSource(event));
      } catch (_) {
        if (event && typeof event.message === 'string') {
          pushLog('error', [event.message], null);
        }
      }
    });

    window.addEventListener('unhandledrejection', event => {
      let message = 'Unhandled rejection: [unprintable]';
      let source = null;
      try {
        const reason = event && event.reason;
        message = 'Unhandled rejection: ' + describeReason(reason);
        source = stackSource(reason);
      } catch (_) {}
      pushLog('error', [message], source);
    });
  }

  function consoleLogs(options) {
    healConsole();
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
    healConsole();
    _logs.length = 0;
    return { cleared: true };
  }

  // Sizes are UTF-8 bytes, not String.length, which counts UTF-16 code units
  // and under-reports non-ASCII text (#253). An XHR text response is measured
  // after the browser decoded it, so a compressed body does not match the
  // bytes on the wire.
  const _utf8 = new TextEncoder();
  function utf8Size(text) {
    return _utf8.encode(text).length;
  }

  function bodySize(body) {
    if (!body) return 0;
    if (typeof body === "string") return utf8Size(body);
    if (body instanceof URLSearchParams) return body.toString().length;
    if (body instanceof Blob) return body.size;
    if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) return body.byteLength;
    return 0;
  }

  // The one unknown-size convention for `response_size`: a size nobody
  // measured is `null`, never a confident 0 (#232). `parseInt` would accept
  // "1380bytes", so the whole header has to be digits.
  function headerSize(raw) {
    if (typeof raw !== "string" || !/^\d+$/.test(raw)) return null;
    const n = Number(raw);
    return Number.isSafeInteger(n) ? n : null;
  }

  // XHR decodes text with the Content-Type charset, UTF-8 when it names none.
  // Re-encoding a windows-1252 or Shift_JIS body as UTF-8 does not give its
  // size, so only a UTF-8 body is measured. A charset forced with
  // overrideMimeType() does not show in the header and is missed.
  function isUtf8Text(xhr) {
    const m = /;\s*charset\s*=\s*"?([^";\s]+)/i.exec(xhr.getResponseHeader("Content-Type") || "");
    return !m || /^utf-?8$/i.test(m[1]);
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
      // tauri:// responses carry no Content-Length. Reading the real size
      // would mean cloning and buffering every body, so report null — an
      // unknown size, not a confident 0 (#232).
      const responseSize = headerSize(response.headers.get("Content-Length"));
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
        response_size: null,
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
        // A responseType of "json" or "document" hands back a plain object,
        // and non-UTF-8 text cannot be re-measured, so neither branch below
        // measures them — fall back to the header, and to null when it is
        // missing, like the fetch wrapper does (#232, #253).
        const cl = headerSize(this.getResponseHeader("Content-Length"));
        const r = this.response;
        const responseSize = (this.responseType === "" || this.responseType === "text")
          ? (typeof r === "string" && isUtf8Text(this) ? utf8Size(r) : cl)
          : (r instanceof ArrayBuffer ? r.byteLength : (r instanceof Blob ? r.size : cl));
        pushEntry(this.status, null, responseSize);
      };
      onError = () => { pushEntry(0, "Network error", null); };
      onTimeout = () => { pushEntry(0, "Timeout", null); };
      onAbort = () => { pushEntry(0, "Aborted", null); };
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
    // HTML-AAM: a select shown as a list box (multiple, or size > 1) is a
    // listbox, not a combobox (#307).
    if (el.tagName === "SELECT" && (el.multiple || el.size > 1)) return "listbox";
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

    const fromLabels = labelText(el);
    if (fromLabels) return fromLabels.slice(0, 50);

    // HTML-AAM: text inputs and textareas take `title` before `placeholder`.
    // Other elements keep their text ahead of `title` (checked last below).
    if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
      const title = el.getAttribute("title");
      if (title && title.trim()) return title.trim().slice(0, 50);
    }

    if (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT") {
      const placeholder = el.getAttribute("placeholder");
      if (placeholder) return placeholder.trim().slice(0, 50);
    }

    // A <select>'s text is every option's text (#277), and a textbox's text is
    // its value, which the user edits (#303): a <textarea>'s default value, a
    // contenteditable host's content, a textbox or searchbox widget's content.
    // None of these is a name. An <input> has no text.
    if (el.tagName !== "SELECT" && !isTextboxHost(el)) {
      const text = el.textContent || "";
      const trimmed = text.replace(/\s+/g, " ").trim();
      if (trimmed) return trimmed.slice(0, 50);
    }

    const title = el.getAttribute("title");
    if (title && title.trim()) return title.trim().slice(0, 50);
    return null;
  }

  // Whether the element's text content is an editable value, not a name.
  // ARIA defines `searchbox` as a kind of `textbox`.
  function isTextboxHost(el) {
    if (el.tagName === "TEXTAREA" || el.tagName === "INPUT" || carriesContentEditable(el)) return true;
    const role = String(el.getAttribute("role") || "").trim().toLowerCase();
    return role === "textbox" || role === "searchbox";
  }

  // Text of the <label>s associated with a form control: `el.labels` covers
  // both a wrapping label and `<label for=...>` (#277). Nested <select> and
  // <textarea> subtrees are skipped, the control's own and any other, so a
  // wrapping label does not leak their options or contents into the name.
  // A labelled button keeps its own text after the label's, as in accname.
  // Hidden content is not part of the name (accname step 2A), e.g. a
  // required-field asterisk: subtrees marked `aria-hidden="true"`, `hidden`
  // or `display: none` are skipped. `visibility: hidden` drops the node's own
  // text only, since a descendant can set `visibility: visible` again.
  function labelText(el) {
    const labels = el.labels;
    if (!labels || labels.length === 0) return "";
    const canStyle = typeof window.getComputedStyle === "function";
    const parts = [];
    function collect(node, visible) {
      if (node.tagName === "SELECT" || node.tagName === "TEXTAREA") return;
      if (node.nodeType === Node.TEXT_NODE) {
        if (visible) parts.push(node.nodeValue || "");
        return;
      }
      if (node.nodeType !== Node.ELEMENT_NODE) return;
      const ariaHidden = String(node.getAttribute("aria-hidden") || "").trim().toLowerCase();
      if (ariaHidden === "true" || node.hasAttribute("hidden")) return;
      let shown = visible;
      const style = canStyle ? window.getComputedStyle(node) : null;
      if (style) {
        if (style.display === "none") return;
        shown = style.visibility !== "hidden" && style.visibility !== "collapse";
      }
      for (const child of node.childNodes || []) collect(child, shown);
    }
    for (const label of labels) {
      collect(label, true);
      parts.push(" ");
    }
    return parts.join("").replace(/\s+/g, " ").trim();
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
        // form controls, but a number for `<li>`, `<progress>`, and
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
          // Text renderers mask a password value; --json keeps it raw (#279).
          if (inputType === "password") entry.sensitive = true;
        }
        if (node.disabled) entry.disabled = true;
        // fill and type refuse a readonly field, so the snapshot says so (#324).
        if (isReadOnly(node)) entry.readonly = true;
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

  // ─── Recorded locators (#276) ────────────────────────────────────────────
  // A ref names an element of the last snapshot only. While recording, the
  // plugin asks `locate` for a selector that finds the same element in any
  // document, plus a fingerprint to check it is still that element.

  // `CSS.escape`, with a fallback for the test DOM: a leading digit becomes
  // its code point escape, any other non-identifier character is escaped.
  function cssEscape(value) {
    if (typeof CSS !== "undefined" && CSS && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    const s = String(value);
    let out = "";
    for (let i = 0; i < s.length; i++) {
      const ch = s[i];
      const leading = i === 0 || (i === 1 && s[0] === "-");
      if (leading && ch >= "0" && ch <= "9") out += "\\3" + ch + " ";
      else if (/[a-zA-Z0-9_-]/.test(ch) || ch.charCodeAt(0) >= 0x80) out += ch;
      else out += "\\" + ch;
    }
    return out;
  }

  // A quoted CSS string: quotes and backslashes escaped, control characters
  // as hex escapes (a raw line break is a parse error).
  function cssString(value) {
    return '"' + String(value).replace(/["\\]|[\x00-\x1f\x7f]/g, function (ch) {
      return ch === '"' || ch === "\\" ? "\\" + ch : "\\" + ch.charCodeAt(0).toString(16) + " ";
    }) + '"';
  }

  // Every element `selector` matches, or null for a selector the page rejects.
  function queryAll(selector) {
    try {
      return Array.from(document.querySelectorAll(selector));
    } catch (_) {
      return null;
    }
  }

  function matchesOnly(selector, el) {
    const found = queryAll(selector);
    return found !== null && found.length === 1 && found[0] === el;
  }

  function hasUniqueId(el) {
    return Boolean(el.id) && matchesOnly("#" + cssEscape(el.id), el);
  }

  // `tag:nth-of-type(n)` steps up to the nearest ancestor with a unique id,
  // or to <body>. `:nth-of-type` is added only where a same-tag sibling exists.
  // `anchor` is that unique-id ancestor, or null when the path starts at the
  // document: the path can only ever match inside it.
  function cssPath(el) {
    const steps = [];
    let anchor = null;
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE) {
      if (node !== el && hasUniqueId(node)) {
        steps.unshift("#" + cssEscape(node.id));
        anchor = node;
        break;
      }
      const tag = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (!parent || node === document.body) {
        steps.unshift(tag);
        break;
      }
      const sameTag = Array.from(parent.children).filter(function (c) {
        return c.tagName === node.tagName;
      });
      steps.unshift(sameTag.length > 1 ? tag + ":nth-of-type(" + (sameTag.indexOf(node) + 1) + ")" : tag);
      node = parent;
    }
    return { selector: steps.join(" > "), anchor: anchor };
  }

  // First candidate that matches `el` and nothing else, or null. The CSS
  // path is positional, so it only identifies `el` when its fingerprint is
  // distinctive where the path can reach: with a twin there (identical
  // "Delete" buttons in a list), a shift would land on the twin and still
  // pass the check at replay.
  function stableSelector(el) {
    const tag = el.tagName.toLowerCase();
    const candidates = [];
    if (el.id) candidates.push("#" + cssEscape(el.id));
    const testId = el.getAttribute("data-testid");
    if (testId) candidates.push("[data-testid=" + cssString(testId) + "]");
    const name = el.getAttribute("name");
    if (name) {
      const byName = tag + "[name=" + cssString(name) + "]";
      candidates.push(byName);
      // Radios and checkboxes only: in a group sharing one name, the value is
      // what tells the members apart. On other controls the value attribute
      // is page data (SSR, re-renders), not identity.
      const type = (el.getAttribute("type") || "").toLowerCase();
      const checkable = tag === "input" && (type === "radio" || type === "checkbox");
      const value = el.getAttribute("value");
      if (checkable && value !== null) candidates.push(byName + "[value=" + cssString(value) + "]");
    }
    const found = candidates.find(function (c) { return matchesOnly(c, el); });
    if (found) return found;
    const path = cssPath(el);
    if (hasFingerprintTwin(el, path.anchor)) return null;
    return matchesOnly(path.selector, el) ? path.selector : null;
  }

  function fingerprint(el) {
    return { tag: el.tagName.toLowerCase(), role: getRole(el) || null, name: getName(el) || null };
  }

  function sameFingerprint(a, b) {
    return a.tag === b.tag && a.role === b.role && a.name === b.name;
  }

  // Whether another element inside `scope` (the whole document when null)
  // carries `el`'s fingerprint.
  function hasFingerprintTwin(el, scope) {
    const own = fingerprint(el);
    const sameTag = queryAll(own.tag) || [];
    return sameTag.some(function (other) {
      return other !== el && isInside(other, scope) && sameFingerprint(fingerprint(other), own);
    });
  }

  function isInside(node, scope) {
    if (!scope) return true;
    for (let n = node; n; n = n.parentElement) {
      if (n === scope) return true;
    }
    return false;
  }

  function describeFingerprint(fp) {
    let out = "<" + fp.tag;
    if (fp.role) out += " role=" + JSON.stringify(fp.role);
    if (fp.name) out += " name=" + JSON.stringify(fp.name);
    return out + ">";
  }

  // `{refs: {key: ref}}` → `{key: {selector?, expect}}`. A ref this page no
  // longer knows is left out, so the other refs of a drag keep their locator.
  function locate(params) {
    const out = {};
    const refs = (params && params.refs) || {};
    for (const key of Object.keys(refs)) {
      const el = resolve(refs[key]);
      if (!el) continue;
      const entry = {};
      const selector = stableSelector(el);
      if (selector) entry.selector = selector;
      entry.expect = fingerprint(el);
      out[key] = entry;
    }
    return out;
  }

  // A recorded step: its selector must match exactly one element (no
  // fallback to the ref, which may name another element by now), and that
  // element must still carry the recorded fingerprint.
  function resolveRecorded(params) {
    const want = params.expect;
    if (typeof want !== "object" || want === null || typeof want.tag !== "string") {
      throw new Error('Invalid recorded fingerprint: expected an object with a string "tag"');
    }
    let el, where;
    if (params.selector) {
      where = "Recorded selector " + params.selector;
      const found = queryAll(params.selector);
      if (found === null) throw new Error("Invalid recorded selector: " + params.selector);
      if (found.length === 0) throw new Error("No element matches recorded selector " + params.selector);
      if (found.length > 1) {
        throw new Error(where + " matches " + found.length + " elements, expected exactly 1");
      }
      el = found[0];
    } else if (params.ref) {
      where = "Ref " + params.ref;
      el = requireEl(params.ref);
    } else {
      throw new Error("Recorded step has no selector or ref");
    }
    const got = fingerprint(el);
    if (got.tag !== want.tag || got.role !== (want.role || null) || got.name !== (want.name || null)) {
      throw new Error(where + " found " + describeFingerprint(got) +
        ", recorded " + describeFingerprint(want));
    }
    return el;
  }

  function resolveTarget(params) {
    if (params.expect !== undefined) return resolveRecorded(params);
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
    requireEnabled(el, "click");
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

  // `:disabled` covers a control inside a disabled <fieldset> and an option
  // inside a disabled <optgroup>, which the `disabled` property misses.
  function matchesDisabled(node) {
    return typeof node.matches === "function" && node.matches(":disabled");
  }

  // Roles whose own `aria-disabled` counts, as in Playwright's actionability
  // check (WAI-ARIA lists the roles that support the attribute).
  const ARIA_DISABLED_ROLES = new Set([
    "application", "button", "checkbox", "columnheader", "combobox", "composite",
    "grid", "gridcell", "group", "input", "link", "listbox", "menu", "menubar",
    "menuitem", "menuitemcheckbox", "menuitemradio", "option", "radio",
    "radiogroup", "row", "rowheader", "scrollbar", "searchbox", "select",
    "separator", "slider", "spinbutton", "switch", "tab", "tablist", "textbox", "toolbar",
    "tree", "treegrid", "treeitem",
  ]);

  // Whether one of the target's role tokens is in `roles`. A `role` attribute
  // is a whitespace-separated fallback list, matched without case.
  function hasRoleIn(el, roles) {
    const tokens = String(getRole(el) || "").toLowerCase().split(/\s+/);
    return tokens.some((token) => roles.has(token));
  }

  // Like Playwright: a target whose role supports `aria-disabled` reads it on
  // itself, then on its ancestors, the nearest explicit value winning. A
  // control disabled this way blocks a user as much as a native `disabled`.
  function isAriaDisabled(el) {
    if (typeof el.getAttribute !== "function" || !hasRoleIn(el, ARIA_DISABLED_ROLES)) return false;
    for (let node = el; node && typeof node.getAttribute === "function"; node = node.parentElement) {
      const value = node.getAttribute("aria-disabled");
      if (value === null) continue;
      const lowered = String(value).toLowerCase();
      if (lowered === "true") return true;
      if (lowered === "false") return false;
    }
    return false;
  }

  // HTML ignores `readonly` on these input types, though the `readOnly`
  // property still reflects the attribute.
  const READONLY_IGNORED_TYPES = new Set([
    "button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit",
  ]);

  // Roles whose own `aria-readonly` counts, as in Playwright's editable check.
  const ARIA_READONLY_ROLES = new Set([
    "checkbox", "combobox", "grid", "gridcell", "listbox", "radiogroup", "searchbox",
    "slider", "spinbutton", "textbox",
  ]);

  // Whether the field is readonly. A native <input> or <textarea> follows
  // HTML: its `readonly`, on an input type that honours it. Any other element
  // whose role supports it reads its own `aria-readonly="true"`, as a
  // rich-text editor in read mode sets on its contenteditable host.
  function isReadOnly(el) {
    const tag = elementTag(el);
    if (tag === "textarea") return el.readOnly === true;
    if (tag === "input") {
      if (el.readOnly !== true) return false;
      const type = String(el.getAttribute("type") || "text").toLowerCase();
      return !READONLY_IGNORED_TYPES.has(type);
    }
    if (tag === "select" || typeof el.getAttribute !== "function") return false;
    const value = el.getAttribute("aria-readonly");
    return value !== null && String(value).toLowerCase() === "true" && hasRoleIn(el, ARIA_READONLY_ROLES);
  }

  const DISABLEABLE_TAGS = new Set(["button", "input", "select", "textarea"]);

  // Whether the target sits inside a disabled form control, such as an icon
  // <span> in a disabled <button>. A synthetic click on the child bubbles to
  // the control and runs its handler; a user's click there does nothing. A
  // disabled <fieldset> blocks only its controls, which `:disabled` covers.
  function insideDisabledControl(el) {
    for (let node = el.parentElement; node; node = node.parentElement) {
      if (DISABLEABLE_TAGS.has(elementTag(node)) && matchesDisabled(node)) return true;
    }
    return false;
  }

  // A user cannot act on a disabled control, so the action fails before
  // touching the element or firing any event (#324). Synthetic events bypass
  // the browser's own block: a disabled button's onclick runs on
  // `dispatchEvent`.
  function requireEnabled(el, action) {
    if (matchesDisabled(el) || insideDisabledControl(el) || isAriaDisabled(el)) {
      throw new Error(action + ": target is disabled");
    }
  }

  // `fill` and `type` also need a field a user could edit (#324).
  function requireWritable(el, action) {
    requireEnabled(el, action);
    if (isReadOnly(el)) throw new Error(action + ": target is readonly");
  }

  // Without an action the error stays neutral: `checked` backs both
  // `assert checked` and `assert unchecked`, so naming it misleads (#311).
  function requireCheckable(el, action) {
    const tag = elementTag(el);
    const type = el && el.type != null ? String(el.type).toLowerCase() : "";
    if (tag === "input" && (type === "checkbox" || type === "radio")) return;
    const reported = (tag === "input" ? "input type=" + type : tag || String(el)).slice(0, 64);
    const lead = action ? action + " requires" : "expected";
    throw new Error(lead + ' an <input type="checkbox"> or <input type="radio">, got: ' + reported);
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

  // A user cannot pick a disabled option, nor one in a disabled <optgroup>
  // (#324). `wanted[i]` is the value given for the option `matches[i]`.
  function rejectDisabledOptions(wanted, matches, command) {
    const locked = wanted.filter((_, i) => matchesDisabled(matches[i]));
    if (locked.length === 0) return;
    const quoted = locked.map((w) => JSON.stringify(w)).join(", ");
    throw new Error(
      command + (locked.length === 1 ? ": option " + quoted + " is disabled" : ": options " + quoted + " are disabled"),
    );
  }

  function resolveSelectOptions(el, wantedRaw, command) {
    // Resolve the target option before mutating anything. Setting
    // `HTMLSelectElement.value` to a string that matches no option `value`
    // silently yields `value=""` / `selectedIndex=-1` per the DOM spec, so
    // "set then trust" reports success on a no-op (#113). Match the option
    // first — by `value`, then by visible label — and error if none matches so
    // a reported `ok` always means the selection was set to exactly the
    // requested options, or cleared when the list is empty (#327).
    // `wantedRaw` is one value or a list (#306). `fill` delegates here too, so
    // error prefixes name the command the user actually ran.
    const wanted = (Array.isArray(wantedRaw) ? wantedRaw : [wantedRaw]).map(String);
    // An empty list clears a <select multiple> (#327): the loop below then
    // deselects every option. A single select cannot be left empty by a user.
    if (wanted.length === 0 && !el.multiple) {
      throw new Error(command + ": no value given; only a <select multiple> can be cleared");
    }
    if (wanted.length > 1 && !el.multiple) {
      throw new Error(
        command + ": " + wanted.length + " values given, but the <select> is not multiple",
      );
    }
    const options = Array.from(el.options || []);
    const matches = wanted.map(
      (w) =>
        options.find((o) => o.value === w) ||
        options.find((o) => (o.text || "").trim() === w.trim()),
    );
    const missing = wanted.filter((_, i) => !matches[i]);
    if (missing.length > 0) {
      throw new Error(command + ": no option matches " + missing.map((w) => JSON.stringify(w)).join(", "));
    }
    rejectDisabledOptions(wanted, matches, command);
    return matches;
  }

  function applySelectOption(el, wantedRaw, command) {
    const matches = resolveSelectOptions(el, wantedRaw, command);
    if (el.multiple) {
      // Assigning `.value` on a multi-select keeps only the first match, so
      // set each option's own flag: exactly the listed options end up chosen.
      const chosen = new Set(matches);
      for (const o of Array.from(el.options || [])) o.selected = chosen.has(o);
      return;
    }
    const matched = matches[0];
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
    requireWritable(el, "fill");
    // `select` owns the list form (#306); `fill` keeps its one-value contract
    // on every target, before any setter can stringify the list. Only a
    // `<select>` can take the list through `select`, so only it gets the hint.
    if (Array.isArray(params.value)) {
      if (elementTag(el) === "select") {
        throw new Error("fill takes one value; use select for several options");
      }
      throw new Error("fill takes one value, not a list");
    }
    const isSelect = elementTag(el) === "select";
    // Check the options before focusing, so a refused pick fires no event.
    if (isSelect) resolveSelectOptions(el, params.value, "fill");
    el.focus();
    let wroteViaExec = false;
    if (isSelect) {
      applySelectOption(el, params.value, "fill");
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
    requireWritable(el, "type");
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
    requireEnabled(el, "select");
    applySelectOption(el, params.value, "select");
    // A user's pick fires `input` then `change`, once for the whole selection.
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }

  function check(params) {
    const el = resolveTarget(params);
    requireCheckable(el, "check");
    requireEnabled(el, "check");
    const type = el && el.type != null ? String(el.type).toLowerCase() : "";
    // Radios have no click-to-uncheck; a selected one stays as it is.
    if (type === "radio" && el.checked) return { ok: true };
    // Assigning `.checked` also updates React's value tracker, so its
    // onChange never runs. A native click changes the state behind the
    // tracker and fires click, input and change like a user click (#212).
    const before = el.checked;
    el.click();
    if (el.checked === before) {
      throw new Error("check did not change the target; the page may have cancelled the click");
    }
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
  // reports its `value` attribute as written: `HTMLLIElement.value` reflects
  // it as a `long` and reads `0` when it is absent, empty, or not an integer,
  // even in an `<ol>` (#162). A textbox with no IDL `.value` (a
  // contenteditable host, a `role=textbox` or `role=searchbox` widget) reports
  // its text, whitespace collapsed, as its value (#326). A form control keeps
  // its IDL `.value` whatever its role or contenteditable state.
  function elementValue(el) {
    if (!el) return undefined;
    const tag = String(el.tagName || "").toLowerCase();
    if (tag === "select" && el.multiple) return selectedOptionValues(el).join(", ");
    if (tag === "li") return el.getAttribute("value") || undefined;
    if (el.value === undefined && isTextboxHost(el)) return editableText(el);
    return el.value;
  }

  // Text of an editable element, whitespace collapsed. `innerText` is
  // layout-aware: it puts a newline between block children (each paragraph
  // of ProseMirror / Tiptap / Lexical) and for a `<br>`, where `textContent`
  // glues the words together. `textContent` stays the fallback when
  // `innerText` is not a string.
  function editableText(el) {
    const raw = typeof el.innerText === "string" ? el.innerText : el.textContent;
    return String(raw || "").replace(/\s+/g, " ").trim();
  }

  // `fill` / `type` accept a child of a contenteditable host (inherited
  // editability), so `value` on that child reads its text too. The snapshot
  // keeps using `elementValue`, so child paragraphs never get a value there.
  function value(params) {
    const el = resolveTarget(params);
    if (el && el.value === undefined && isContentEditable(el)) return editableText(el);
    return elementValue(el) || "";
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
    // `missingOk` lets `assert hidden` pass once a selector matches nothing:
    // a removed node is not visible (#281). Refs keep throwing, since an
    // unknown ref usually means a stale snapshot, not a removed element.
    if (
      params.missingOk &&
      !params.ref &&
      params.selector &&
      !document.querySelector(params.selector)
    ) {
      return { visible: false };
    }
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
    // Without the guard, any other element reports `false` and
    // `assert unchecked` passes on a wrong target (#286).
    requireCheckable(el);
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

  // The `__PILOT__` prefix marks its frames as the bridge's, so an app
  // function that happens to be called evalScript keeps its log source.
  function __PILOT__evalScript(options) {
    var script = options && options.script;
    if (!script) throw new Error("No script provided");
    // Top-level `await` is detected before stage 1: `await (1 + 1)` also
    // compiles as a plain expression, a call to a function named `await`,
    // and then fails at run time (#302). Such a script skips stage 1 and
    // goes to the async stages.
    var topLevelAwait = hasTopLevelAwait(script);
    // Stage 1 — expression compile, for scripts without top-level `await`.
    // `{a:1}` keeps its object-literal semantics (not a labeled block) and
    // `class C {}` evaluates to the constructor. Keep compilation separate
    // from execution: a runtime SyntaxError from e.g. `JSON.parse('x')` must
    // propagate, not trigger a fallback — otherwise the script would run twice.
    if (!topLevelAwait) {
      var expr = null;
      try {
        expr = new Function("return (\n" + script + "\n)");
      } catch (e1) {
        if (!(e1 instanceof SyntaxError)) throw e1;
      }
      if (expr) return expr();
    }
    // The newlines around `script` in every wrapper isolate user tokens
    // from generated closing punctuation. Without them, a trailing
    // `// comment` on the last line of the user script swallows `))()` or
    // `})()` and the wrapper fails to compile.
    if (topLevelAwait) {
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

  // Top-level `await` detector (#79). The engine decides first: in a plain
  // function body `await expr` is a SyntaxError, while in an async function
  // body it compiles. No check runs the script, and every constructor parses
  // `src` as a whole body, so a script cannot close a probe early.
  //
  //   * sync fails, async compiles: the script has top-level `await`.
  //   * sync compiles, async fails: `await` is used as an identifier.
  //   * both compile: sloppy code also reads `await` as an identifier in
  //     `await (x)`, `await [x]`, `await +x`, `` await `x` `` and before a
  //     line break, so each `await` is probed on its own (#272).
  //   * both fail: a real syntax error. The text scan keeps the old routing,
  //     so a broken script with `await` still gets the auto-wrap hint.
  function hasTopLevelAwait(src) {
    // Every branch below needs an `await` token, and the detector runs on
    // each eval: skip its compile probes for a script without one.
    if (!/\bawait\b/.test(src)) return false;
    var syncOk = compiles(Function, src);
    var asyncOk = compiles(AsyncFunction, src);
    if (!syncOk && asyncOk) return true;
    if (syncOk && !asyncOk) return false;
    if (syncOk) {
      var probed = probeEachAwait(src);
      if (probed !== null) return probed;
      return /\bawait\b(?=\s*[(\[`+\-!~\w$'"])/.test(maskNestedFunctions(src));
    }
    return /\bawait\b/.test(maskNestedFunctions(src));
  }

  var AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  var AsyncGeneratorFunction = Object.getPrototypeOf(async function* () {}).constructor;

  function compiles(Ctor, body) {
    try {
      new Ctor(body);
      return true;
    } catch (e) {
      if (e instanceof SyntaxError) return false;
      throw e;
    }
  }

  // Asks the engine where each `await` token of a script that compiles in
  // both modes sits. Returns true or false, or null when it cannot tell.
  //
  //   * Code or text: the token is code when putting `#` in its place breaks
  //     the parse. In a string, comment, template text or regex literal `#`
  //     is just a character.
  //   * Top level or nested: in a strict async generator body, `yield` is
  //     reserved in every nested function, arrow, method and class body, so
  //     `(yield await 0)||` in place of the token compiles only at the top
  //     level of the script. It also compiles inside a nested async
  //     generator; that `await` is then taken as top-level, like before.
  //
  // Returns null when the script has sloppy-only syntax (`with`, legacy
  // octals, `yield` as a name) that the strict probe rejects, or when it
  // holds too many `await` tokens to probe one by one.
  function probeEachAwait(src) {
    if (!compiles(AsyncGeneratorFunction, '"use strict";\n' + src)) return null;
    var re = /(^|[^\w$])await(?![\w$])/g;
    var m;
    for (var n = 0; (m = re.exec(src)) !== null; n++) {
      if (n >= 64) return null;
      var at = m.index + m[1].length;
      var before = src.slice(0, at);
      var after = src.slice(at + 5);
      re.lastIndex = at + 5;
      if (compiles(AsyncFunction, before + "#" + after)) continue;
      if (compiles(AsyncGeneratorFunction, '"use strict";\n' + before + "(yield await 0)||" + after)) {
        return true;
      }
    }
    return false;
  }

  // Strips comments and single/double quoted strings, masks property
  // accesses (`obj.await`), then removes nested `function`/arrow bodies so
  // an `await` buried in a nested function is hidden.
  //
  // Only a fallback: probeEachAwait decides whenever the script compiles.
  // This scan runs for a script with a syntax error, or one the strict probe
  // rejects. Known misses there: template text, regex literals, method
  // shorthand (`async m() { ... }`) and functions whose parameter list holds
  // parentheses are left in place, so an `await` inside them still counts.
  //
  // For scripts larger than 100 KB the strip pass is skipped to bound
  // worst-case scan time; the raw source is returned instead.
  function maskNestedFunctions(src) {
    if (src.length > 100000) return src;
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
    // (`() => { ... }`) and concise (`() => expr`), then drop the braces of
    // the innermost remaining blocks (`if`, `try`, object literals) but keep
    // their content, so an `await` in a top-level block stays visible and
    // the function around a block can peel on the next pass (#272). Each
    // pass handles one nesting level; the cap bounds pathological input.
    // Concise arrow bodies stop at any of `;,){}\n` to avoid chewing
    // through the rest of the script, and must not start with `{` or a
    // space, or they would peel only the `() =>` of a block body.
    for (var k = 0; k < 64; k++) {
      var prev = stripped;
      stripped = stripped
        .replace(/\bfunction\s*\*?\s*[\w$]*\s*\([^()]*\)\s*\{[^{}]*\}/g, "fn()")
        .replace(/\([^()]*\)\s*=>\s*\{[^{}]*\}/g, "fn()")
        .replace(/\b[\w$]+\s*=>\s*\{[^{}]*\}/g, "fn()")
        .replace(/\([^()]*\)\s*=>\s*[^{};,)\s][^{};,)\n]*/g, "fn()")
        .replace(/\b[\w$]+\s*=>\s*[^{};,)\s][^{};,)\n]*/g, "fn()")
        .replace(/\{([^{}]*)\}/g, " $1 ");
      if (stripped === prev) break;
    }
    return stripped;
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

  // Text of the node's own text children, whitespace-collapsed, max 80 chars.
  function directText(node) {
    return Array.from(node.childNodes)
      .filter(function(n) { return n.nodeType === Node.TEXT_NODE; })
      .map(function(n) { return n.textContent || ''; })
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim()
      .substring(0, 80);
  }

  // True when `nodes` holds a text node with non-whitespace text. Formatting
  // whitespace around added or removed elements is not a text change.
  function hasNonBlankText(nodes) {
    for (var n = 0; n < nodes.length; n++) {
      if (nodes[n].nodeType === Node.TEXT_NODE && /\S/.test(nodes[n].textContent || '')) return true;
    }
    return false;
  }

  function summarizeNode(node) {
    var entry = { tag: node.tagName.toLowerCase() };
    if (node.id) entry.id = node.id;
    if (node.className && typeof node.className === 'string' && node.className.trim()) entry.class = node.className.trim();
    var text = directText(node);
    if (text) entry.text = text;
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

      function hasChanges() {
        return changes.added.length > 0 || changes.removed.length > 0 ||
          changes.modified.length > 0;
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
        // `textContent = ...` swaps text nodes through a childList mutation;
        // report it as a text change on the parent, once per batch (#304).
        var textTargets = [];
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
            if (
              textTargets.indexOf(mutation.target) === -1 &&
              (hasNonBlankText(mutation.addedNodes) || hasNonBlankText(mutation.removedNodes))
            ) {
              textTargets.push(mutation.target);
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
            var data = mutation.target.textContent || '';
            // Comment data and whitespace-to-whitespace edits are not text
            // changes.
            if (
              parent && mutation.target.nodeType === Node.TEXT_NODE &&
              (/\S/.test(data) || /\S/.test(mutation.oldValue || ''))
            ) {
              pushCapped(changes.modified, {
                tag: parent.tagName.toLowerCase(),
                text: data.replace(/\s+/g, ' ').trim().substring(0, 80),
              });
            }
          }
        }
        for (var t = 0; t < textTargets.length; t++) {
          var textTarget = textTargets[t];
          pushCapped(changes.modified, {
            tag: textTarget.tagName.toLowerCase(),
            text: directText(textTarget),
          });
        }
        // With requireMutation, mutations that add no entry (blank text,
        // comments, detached characterData) must not end the wait: the
        // summary would be empty (#304).
        if (requireMutation && !hasChanges()) return;
        resetStableTimer();
      });

      observer.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        characterData: true,
        characterDataOldValue: true,
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
    "padding-bottom", "padding-left", "overflow-x", "overflow-y",
    // CSS scrollbar painting. `::-webkit-scrollbar` (and other scrollbar
    // pseudo-elements) cannot be copied this way — html-to-image clones
    // computed styles on the element, not on pseudo-elements (#166).
    "scrollbar-color", "scrollbar-width", "scrollbar-gutter",
    "visibility", "opacity", "vertical-align", "content-visibility", "clip",
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

  // Removing a missing key succeeds, as `removeItem` does; `deleted` reports
  // whether the key existed beforehand (#284).
  function storageDelete(params) {
    if (typeof params.key !== "string") {
      throw new Error("storageDelete requires a string key");
    }
    var storage = params.session ? sessionStorage : localStorage;
    var existed = storage.getItem(params.key) !== null;
    storage.removeItem(params.key);
    return { deleted: existed };
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
    eval: __PILOT__evalScript,
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
    storageDelete: storageDelete,
    storageClear: storageClear,
    formDump: formDump,
    locate: locate,
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
