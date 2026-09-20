// Uncaught errors and unhandled rejections must reach the log buffer (#188).
//
// README step 6 of the AI-agent workflow is `tauri-pilot logs --level error`
// to "check for JS errors", but the bridge only wrapped console.*. A page that
// throws never calls console.error itself — the browser prints that — so the
// one command documented for finding JS errors could not see them.
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// real file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.errors.test.mjs

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";
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

// Object.assign would flow through a previous bridge's console setter rather
// than restore a native console, so each load would sit on top of the last
// one. The accessor is configurable, so redefining it gives a clean start.
function resetConsole() {
  for (const level of Object.keys(REAL_CONSOLE)) {
    Object.defineProperty(console, level, {
      value: REAL_CONSOLE[level],
      writable: true,
      configurable: true,
      enumerable: true,
    });
  }
}

const SAVED_GLOBALS = {
  window: globalThis.window,
  location: globalThis.location,
  document: globalThis.document,
  XMLHttpRequest: globalThis.XMLHttpRequest,
};

// These tests mutate shared process globals. Node's per-file isolation hides
// that today, but an in-process runner would leak a stale window and a console
// still wearing a bridge view.
after(() => {
  resetConsole();
  for (const [key, value] of Object.entries(SAVED_GLOBALS)) {
    if (value === undefined) delete globalThis[key];
    else globalThis[key] = value;
  }
});

// The bridge wraps fetch/XHR on load, so the mock has to carry both even
// though these tests only care about the error listeners.
function baseGlobals() {
  resetConsole();
  globalThis.location = { href: "https://app.example/" };
  globalThis.XMLHttpRequest = function () {};
  globalThis.XMLHttpRequest.prototype.open = function () {};
  globalThis.XMLHttpRequest.prototype.send = function () {};
  globalThis.XMLHttpRequest.prototype.addEventListener = function () {};
  globalThis.XMLHttpRequest.prototype.removeEventListener = function () {};
  globalThis.document = { querySelector() { return null; } };
  return {
    fetch() { return Promise.resolve({ status: 200, headers: { get() { return "0"; } } }); },
    location: globalThis.location,
  };
}

function loadBridge() {
  const listeners = Object.create(null);
  globalThis.window = Object.assign(baseGlobals(), {
    addEventListener(type, handler) {
      (listeners[type] || (listeners[type] = [])).push(handler);
    },
    dispatch(type, event) {
      for (const handler of listeners[type] || []) handler(event);
    },
  });

  (0, eval)(BRIDGE_SRC);
  return { pilot: globalThis.window.__PILOT__, win: globalThis.window };
}

test("an uncaught error lands in the log buffer at level error", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("error", {
    message: "Uncaught TypeError: x is not a function",
    filename: "https://app.example/main.js",
    lineno: 42,
    colno: 7,
  });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.equal(errors[0].args[0], "Uncaught TypeError: x is not a function");
  assert.match(errors[0].source, /main\.js:42:7/);
});

test("an unhandled rejection lands in the log buffer at level error", () => {
  const { pilot, win } = loadBridge();
  const reason = new Error("boom");
  reason.stack = "Error: boom\n    at https://app.example/main.js:9:1";
  win.dispatch("unhandledrejection", { reason });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.match(errors[0].args[0], /Unhandled rejection: .*boom/);
  assert.match(errors[0].source, /main\.js:9:1/, "the first stack frame must land in source");
});

test("a JavaScriptCore stack names the throwing frame, not the caller", () => {
  // JSC (WebKitGTK, WKWebView) has no "Name: message" header: line 0 is
  // already the throw site. Skipping a fixed index records the caller, or
  // null on a single-frame stack.
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack =
    "handler@https://app.example/widget.js:12:5\nglobal code@https://app.example/main.js:40:1";
  win.dispatch("error", {
    message: "Uncaught Error: boom",
    filename: "https://app.example/main.js",
    lineno: 40,
    colno: 1,
    error,
  });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.source, /widget\.js:12:5/);
  assert.doesNotMatch(entry.source, /main\.js/, "the throwing frame beats the caller and the event location");
});

test("an anonymous JavaScriptCore frame drops its leading @ in source", () => {
  // JSC writes an anonymous frame as `@url:line:col`. The `@` separates the
  // function name from the location, so an empty name leaves it dangling and
  // `logs` printed `(@tauri://localhost:1:143)` (#232).
  const { pilot, win } = loadBridge();
  const error = new Error("rej");
  error.stack = "@tauri://localhost:1:143";
  win.dispatch("unhandledrejection", { reason: error });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, "tauri://localhost:1:143");
});

test("a named JavaScriptCore frame keeps its function name", () => {
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack = "handler@https://app.example/widget.js:12:5";
  win.dispatch("unhandledrejection", { reason: error });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, "handler@https://app.example/widget.js:12:5");
});

test("an uncaught error from eval'd code reports line:col without \"undefined\"", () => {
  // WebKitGTK sets `filename` to the *string* "undefined" for code that came
  // from eval; the truthiness guard let it through as `undefined:1:91` (#232).
  const { pilot, win } = loadBridge();
  win.dispatch("error", {
    message: "Error: boom",
    filename: "undefined",
    lineno: 1,
    colno: 91,
  });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, "1:91");
});

test("an uncaught error with no location at all has a null source", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("error", { message: "Error: boom" });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, null);
});

test("an uncaught error with a location but no filename reports line:col", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("error", { message: "Error: boom", lineno: 12, colno: 3 });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, "12:3");
});

test("a \"undefined\" filename with no line or column has a null source", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("error", { message: "Error: boom", filename: "undefined" });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.source, null);
});

test("an anonymous JavaScriptCore frame from eval'd code drops the missing file", () => {
  // WebKitGTK writes the url of an eval'd frame as `undefined`, or leaves it
  // out entirely, so the stack path needs the same filter as the event path
  // or `logs` shows `undefined:1:91` / `:1:91` (#232).
  for (const [stack, expected] of [
    ["@undefined:1:91", "1:91"],
    ["@:1:91", "1:91"],
  ]) {
    const { pilot, win } = loadBridge();
    const error = new Error("rej");
    error.stack = stack;
    win.dispatch("unhandledrejection", { reason: error });

    const [entry] = pilot.consoleLogs({ level: "error" });
    assert.equal(entry.source, expected, stack);
  }
});

test("an anonymous JavaScriptCore frame is still the throw site", () => {
  // JSC writes `@url:line:col` for anonymous callbacks (no function name).
  // `[^:]+@` required a name, so this line was skipped and source became the
  // enclosing named frame — or null on a rejection with only anonymous frames.
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack =
    "@https://app.example/widget.js:12:5\nglobal code@https://app.example/main.js:40:1";
  win.dispatch("unhandledrejection", { reason: error });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.source, /widget\.js:12:5/);
  assert.doesNotMatch(entry.source, /main\.js/, "the anonymous throw site beats the named caller");
});

test("a V8 message that ends with :line:col is not treated as a frame", () => {
  // The location regex alone matches "Error: failed at url:12:5". That is the
  // V8 header, not a frame; source must be the real `at` line below it.
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack =
    "Error: failed at https://app.example/widget.js:12:5\n    at handler (https://app.example/main.js:40:1)";
  win.dispatch("error", {
    message: "Uncaught Error: boom",
    filename: "https://app.example/widget.js",
    lineno: 12,
    colno: 5,
    error,
  });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.source, /main\.js:40:1/);
  assert.doesNotMatch(entry.source, /widget\.js/, "the V8 header is not a frame");
});

test("an unindented V8 message continuation starting with at is not a frame", () => {
  // V8 frames are indented ("    at ..."). A message line that happens to
  // read `at fake.js:12:5` with no leading whitespace is not a frame; after
  // trim it currently matches the V8 `at` pattern and steals source.
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack =
    "Error: Invalid config:\nat fake.js:12:5\n    at handler (https://app.example/main.js:40:1)";
  win.dispatch("error", {
    message: "Uncaught Error: boom",
    filename: "https://app.example/fake.js",
    lineno: 12,
    colno: 5,
    error,
  });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.source, /main\.js:40:1/);
  assert.doesNotMatch(entry.source, /fake\.js/, "unindented at-line is message text, not a frame");
});

test("a V8 stack with a multi-line message still names the throw site", () => {
  // V8 puts "Name: message" on line 0, but the message itself can contain
  // newlines. Skipping one line then records "expected foo" as source.
  const { pilot, win } = loadBridge();
  const reason = new Error("Invalid config:\n  expected foo");
  win.dispatch("unhandledrejection", { reason });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.args[0], /Unhandled rejection: Error: Invalid config/);
  assert.match(entry.source, /bridge\.errors\.test\.mjs:\d+:\d+\)?$/);
  assert.doesNotMatch(entry.source, /expected foo/, "message lines are not frames");
});

test("a rejection with a non-Error reason is still recorded", () => {
  const { pilot, win } = loadBridge();
  win.dispatch("unhandledrejection", { reason: "plain string" });

  const errors = pilot.consoleLogs({ level: "error" });
  assert.equal(errors.length, 1);
  assert.match(errors[0].args[0], /Unhandled rejection: plain string/);
});

test("a failed resource load is not reported as a JS error", () => {
  const { pilot, win } = loadBridge();
  // Resource errors (img/script 404) fire the same event type on window but
  // carry no `message`; they are not JS errors and would be noise in `logs`.
  win.dispatch("error", { target: { tagName: "IMG" } });

  assert.deepEqual(pilot.consoleLogs({ level: "error" }), []);
});

test("uncaught errors share the id sequence with console entries", () => {
  const { pilot, win } = loadBridge();
  console.log("first");
  win.dispatch("error", { message: "Uncaught Error: second", filename: "a.js", lineno: 1, colno: 1 });

  const all = pilot.consoleLogs({});
  assert.equal(all.length, 2);
  assert.ok(all[1].id > all[0].id, "ids must stay monotonic across sources");
  // sinceId polling must not skip the uncaught error
  assert.equal(pilot.consoleLogs({ sinceId: all[0].id }).length, 1);
});

test("the bridge still loads when window has no addEventListener", () => {
  globalThis.window = baseGlobals();

  (0, eval)(BRIDGE_SRC);
  assert.ok(globalThis.window.__PILOT__, "bridge must not throw without addEventListener");
});

test("console entries still record the calling site", () => {
  // pushLog() is shared with the error listeners, but extractSource() skips a
  // fixed frame count, so the console wrapper must keep calling it directly.
  const { pilot } = loadBridge();
  console.log("hello");

  const [entry] = pilot.consoleLogs({});
  assert.match(entry.source, /bridge\.errors\.test\.mjs:\d+:\d+\)?$/);
  assert.doesNotMatch(entry.source, /eval at/, "bridge frames name the eval site; source must be the caller's own frame");
});

test("an error event with an Error prefers the stack over filename:lineno", () => {
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  error.stack = "Error: boom\n    at handler (https://app.example/widget.js:12:5)";
  win.dispatch("error", {
    message: "Uncaught Error: boom",
    filename: "https://app.example/main.js",
    lineno: 42,
    colno: 7,
    error,
  });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.source, /widget\.js:12:5/);
  assert.doesNotMatch(entry.source, /main\.js/, "the stack frame beats the event location");
});

test("a rejection with a cross-realm Error keeps its name and message", () => {
  const { pilot, win } = loadBridge();
  // An Error from an iframe fails `instanceof Error`, and an Error has no
  // enumerable own properties, so JSON.stringify would render it as "{}".
  const foreign = runInNewContext('new TypeError("from another realm")');
  assert.equal(foreign instanceof Error, false, "precondition: really cross-realm");

  win.dispatch("unhandledrejection", { reason: foreign });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.args[0], "Unhandled rejection: TypeError: from another realm");
});

test("a rejection reason JSON cannot represent is still legible", () => {
  // JSON.stringify answers undefined for these, and "null" for NaN/Infinity,
  // which would record the useless "Unhandled rejection: undefined".
  const cases = [
    [undefined, "undefined"],
    [function named() {}, "function named"],
    [Symbol("tag"), "Symbol(tag)"],
    [NaN, "NaN"],
    [Infinity, "Infinity"],
    [null, "null"],
  ];

  for (const [reason, expected] of cases) {
    const { pilot, win } = loadBridge();
    win.dispatch("unhandledrejection", { reason });

    const [entry] = pilot.consoleLogs({ level: "error" });
    assert.ok(
      entry.args[0].startsWith("Unhandled rejection: " + expected),
      `reason ${String(expected)} recorded as ${entry.args[0]}`
    );
  }
});

test("a rejection with a circular object is still recorded", () => {
  const { pilot, win } = loadBridge();
  const circular = {};
  circular.self = circular;

  win.dispatch("unhandledrejection", { reason: circular });
  assert.equal(
    pilot.consoleLogs({ level: "error" })[0].args[0],
    "Unhandled rejection: [object Object]"
  );
});

test("a rejection whose reason getter throws is still recorded as unprintable", () => {
  // describeReason and stackSource swallow their own throws. The listener's
  // outer catch is only reached when reading event.reason itself throws.
  const { pilot, win } = loadBridge();
  const event = {};
  Object.defineProperty(event, "reason", {
    get() {
      throw new Error("reason getter");
    },
  });

  win.dispatch("unhandledrejection", event);
  assert.equal(
    pilot.consoleLogs({ level: "error" })[0].args[0],
    "Unhandled rejection: [unprintable]"
  );
});

test("a rejection whose stack getter throws keeps its message", () => {
  const { pilot, win } = loadBridge();
  const error = new Error("boom");
  Object.defineProperty(error, "stack", {
    get() {
      throw new Error("stack getter");
    },
  });

  assert.doesNotThrow(() => win.dispatch("unhandledrejection", { reason: error }));

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.equal(entry.args[0], "Unhandled rejection: Error: boom");
  assert.equal(entry.source, null, "no stack is better than no entry");
});

test("an object that only spoofs the Error tag keeps its own fields", () => {
  const { pilot, win } = loadBridge();
  // Symbol.toStringTag is writable, so the brand check is not proof of an
  // Error. Formatting this one as `name: message` would record
  // "undefined: undefined" and drop everything it actually carried.
  const reason = { [Symbol.toStringTag]: "Error", code: 42, detail: "real info" };

  win.dispatch("unhandledrejection", { reason });

  const [entry] = pilot.consoleLogs({ level: "error" });
  assert.match(entry.args[0], /"code":42/);
  assert.match(entry.args[0], /"detail":"real info"/);
});

test("an Error whose name or message is not a string keeps them", () => {
  // An Error subclass may put anything in these. Requiring a string dropped
  // the field silently, which for a non-string `message` threw away the only
  // description of the failure the reason carried.
  const cases = [
    [{ name: "HttpError", message: 404 }, "HttpError: 404"],
    [{ name: 7, message: "boom" }, "7: boom"],
    [{ message: { code: "E_NET" } }, "Error: [object Object]"],
  ];

  for (const [fields, expected] of cases) {
    const { pilot, win } = loadBridge();
    // No own `name` in the last case, so it reads "Error" off the prototype.
    const reason = Object.assign(new Error(), fields);

    win.dispatch("unhandledrejection", { reason });

    const [entry] = pilot.consoleLogs({ level: "error" });
    assert.equal(entry.args[0], "Unhandled rejection: " + expected);
  }
});
