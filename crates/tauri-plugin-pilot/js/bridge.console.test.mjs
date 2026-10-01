// Pilot's console hook must survive later wrappers (#190).
//
// The bridge installs capture with a plain `console[level] = wrapper` at
// document-start. Any page code that later assigns console.log without
// chaining to what was there silently drops pilot out of the chain, and
// `tauri-pilot logs` reports "No logs captured" forever after. Extension-heavy
// apps do this routinely.
//
// bridge.js is an IIFE that attaches its API to `window.__PILOT__`. We load the
// real file into a minimal global mock so these tests exercise the shipping
// code, not a re-implementation.
//
// Run: node --test crates/tauri-plugin-pilot/js/bridge.console.test.mjs

import { test, after } from "node:test";
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

// The bridge wraps fetch/XHR on load, so the mock has to carry both even
// though these tests only care about console.
// Object.assign would flow through the previous bridge's setter (pushing onto
// its chain) while its getter kept answering with that bridge's view, so every
// test after the first booted on top of the last one's wrapper. The accessor is
// configurable, so redefining it gives each test a genuinely native console.
// `native` stands in for chosen levels, so a test can see what reaches the
// real console.
function resetConsole(native = {}) {
  for (const level of Object.keys(REAL_CONSOLE)) {
    Object.defineProperty(console, level, {
      value: native[level] || REAL_CONSOLE[level],
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

function installGlobals(native) {
  resetConsole(native);
  globalThis.location = { href: "https://app.example/" };
  globalThis.XMLHttpRequest = function () {};
  globalThis.XMLHttpRequest.prototype.open = function () {};
  globalThis.XMLHttpRequest.prototype.send = function () {};
  globalThis.XMLHttpRequest.prototype.addEventListener = function () {};
  globalThis.XMLHttpRequest.prototype.removeEventListener = function () {};
  globalThis.document = { querySelector() { return null; } };
  globalThis.window = {
    fetch() { return Promise.resolve({ status: 200, headers: { get() { return "0"; } } }); },
    location: globalThis.location,
    addEventListener() {},
  };
}

function loadBridge(native) {
  installGlobals(native);
  (0, eval)(BRIDGE_SRC);
  return globalThis.window.__PILOT__;
}

function messages(pilot) {
  return pilot.consoleLogs({ level: "log" }).map((e) => e.args[0]);
}

test("capture survives a wrapper that does not chain", () => {
  const pilot = loadBridge();
  const seen = [];
  // The shape that breaks it today: no reference to the previous console.log.
  console.log = (...args) => { seen.push(args[0]); };

  console.log("after");

  assert.deepEqual(messages(pilot), ["after"]);
  assert.deepEqual(seen, ["after"], "the replacement must still receive the call");
});

test("capture survives a wrapper that does chain", () => {
  const pilot = loadBridge();
  const seen = [];
  const previous = console.log;
  console.log = (...args) => { seen.push(args[0]); previous.apply(console, args); };

  console.log("chained");

  assert.deepEqual(messages(pilot), ["chained"], "must be recorded exactly once");
  assert.deepEqual(seen, ["chained"]);
});

test("a wrapper that calls back through console does not recurse forever", () => {
  const pilot = loadBridge();
  let calls = 0;
  // `previous` is pilot's wrapper, so a naive re-entry would loop.
  const previous = console.log;
  console.log = function (...args) {
    calls += 1;
    previous.apply(console, args);
  };

  console.log("reentrant");

  assert.equal(calls, 1);
  assert.deepEqual(messages(pilot), ["reentrant"], "must not be recorded twice");
});

test("two stacked wrappers both run and capture still works", () => {
  const pilot = loadBridge();
  const order = [];
  const first = console.log;
  console.log = (...args) => { order.push("first"); first.apply(console, args); };
  const second = console.log;
  console.log = (...args) => { order.push("second"); second.apply(console, args); };

  console.log("stacked");

  assert.deepEqual(order, ["second", "first"]);
  assert.deepEqual(messages(pilot), ["stacked"]);
});

test("restoring a saved console.log puts the real console back", () => {
  const printed = [];
  const pilot = loadBridge({ log: (...args) => { printed.push(args[0]); } });
  const saved = console.log;
  console.log = () => {};
  console.log = saved;

  console.log("restored");

  assert.deepEqual(messages(pilot), ["restored"]);
  assert.deepEqual(printed, ["restored"], "the muting replacement must be gone");
  assert.equal(console.log, saved, "the restored reference reads back");
});

test("a wrapper installed and removed repeatedly leaves nothing running", () => {
  const printed = [];
  const pilot = loadBridge({ log: (...args) => { printed.push(args[0]); } });
  let runs = 0;
  // An HMR dispose hook or Sentry's consoleSandbox: wrap, then put the saved
  // reference back. Ignoring the restore left every removed wrapper in the
  // path of every later call.
  for (let i = 0; i < 3; i++) {
    const original = console.log;
    console.log = (...args) => { runs += 1; original.apply(console, args); };
    console.log = original;
  }

  console.log("unwrapped");

  assert.equal(runs, 0);
  assert.deepEqual(printed, ["unwrapped"]);
  assert.deepEqual(messages(pilot), ["unwrapped"]);
});

test("re-assigning the installed function does not stack it again", () => {
  loadBridge();
  const mine = () => {};
  // An idempotency guard never matches: the getter hands back Pilot's entry
  // point, not `mine`, so the page assigns again on every check.
  console.log = mine;
  const installed = console.log;
  if (console.log !== mine) console.log = mine;

  assert.equal(console.log, installed);
});

test("a reference saved before a later replacement is still captured", () => {
  const pilot = loadBridge();
  // loglevel, debug and pino's browser build keep `const log = console.log`
  // from init, so that reference ends up below whatever the page installs
  // later.
  const early = console.log;
  console.log = () => {};

  early("from a logger");

  assert.deepEqual(messages(pilot), ["from a logger"]);
});

test("assigning a non-function falls back to the original console", () => {
  const pilot = loadBridge();
  console.log = "not a function";

  assert.doesNotThrow(() => console.log("still fine"));
  assert.deepEqual(messages(pilot), ["still fine"]);
});

test("each level keeps its own downstream", () => {
  const pilot = loadBridge();
  const warns = [];
  console.warn = (...args) => { warns.push(args[0]); };

  console.log("to log");
  console.warn("to warn");

  assert.deepEqual(warns, ["to warn"], "replacing warn must not divert log");
  assert.deepEqual(messages(pilot), ["to log"]);
  assert.deepEqual(pilot.consoleLogs({ level: "warn" }).map((e) => e.args[0]), ["to warn"]);
});

test("a replacement that defers to its saved console.log terminates", async () => {
  const pilot = loadBridge();
  // The saved reference is called from a timer, long after the original call
  // returned. If it re-entered at the top of the chain, it would go around
  // again -- recording the same line over and over until something stops it.
  const saved = console.log;
  let invocations = 0;
  console.log = (...args) => {
    invocations += 1;
    if (invocations < 50) setTimeout(() => saved(...args), 0);
  };

  console.log("deferred");
  await new Promise((resolve) => setTimeout(resolve, 50));

  assert.equal(invocations, 1, "the replacement must not be re-entered");
  // Recorded twice, on purpose. Called from a timer, the saved reference looks
  // exactly like a logger's early-bound one, and those must record: dropping
  // them would lose every line such a logger writes.
  assert.deepEqual(messages(pilot), ["deferred", "deferred"]);
});

test("assigning another level's console does not record the call twice", () => {
  const pilot = loadBridge();
  const warned = [];
  console.warn = (...args) => { warned.push(args[0]); };
  // Page code redirecting one level at another. console.warn reads back as
  // Pilot's own view, so stacking it would file one call under both levels.
  console.log = console.warn;

  console.log("redirected");

  assert.deepEqual(messages(pilot), ["redirected"], "recorded once, as log");
  assert.deepEqual(
    pilot.consoleLogs({ level: "warn" }),
    [],
    "and not a second time as warn"
  );
  assert.deepEqual(
    warned,
    ["redirected"],
    "the redirect still runs whatever the page installed on warn"
  );
});

test("a level that forwards to another is recorded once, under its own level", () => {
  const pilot = loadBridge();
  // A page function, not Pilot's view, so the setter cannot resolve it. The
  // warn call happens inside the error call that was already recorded.
  console.error = (...args) => console.warn(...args);

  console.error("forwarded");

  assert.deepEqual(
    pilot.consoleLogs().map((e) => [e.level, e.args[0]]),
    [["error", "forwarded"]]
  );
});

test("assigning an older saved view of another level calls what that view called", () => {
  const pilot = loadBridge();
  const order = [];
  const firstWarn = console.warn;
  console.warn = (...args) => { order.push("wrapWarn"); firstWarn.apply(console, args); };
  const staleWarn = console.warn;
  console.warn = () => { order.push("laterWarn"); };
  // staleWarn was saved while wrapWarn was installed, and calling it directly
  // reaches wrapWarn. Taking warn's current function instead would swap in
  // one the page never pointed log at.
  console.log = staleWarn;

  console.log("stale");

  assert.deepEqual(order, ["wrapWarn"]);
  assert.deepEqual(messages(pilot), ["stale"]);
});

test("console.log keeps a stable identity", () => {
  loadBridge();
  assert.equal(console.log, console.log, "page code may compare the reference");

  const before = console.log;
  console.log = (...args) => before(...args);
  assert.notEqual(console.log, before, "a new replacement is a new entry point");
  assert.equal(console.log, console.log);
});

test("two levels aliased to each other do not recurse", () => {
  const pilot = loadBridge();
  // Each alias used to push a thunk that read the other chain's tail when it
  // ran, so once both pointed at each other a single call bounced between
  // them until the stack gave out -- taking down the page, not just the log.
  console.log = console.warn;
  console.warn = console.log;

  assert.doesNotThrow(() => console.log("boom"));
  assert.deepEqual(messages(pilot), ["boom"], "still recorded exactly once");

  // And the same the other way round.
  const pilot2 = loadBridge();
  console.warn = console.log;
  console.log = console.warn;
  assert.doesNotThrow(() => console.warn("other way"));
  assert.equal(pilot2.consoleLogs({ level: "warn" }).length, 1);
});

test("a frozen console does not stop the bridge from loading", () => {
  installGlobals();
  // Freezing the real console cannot be undone, so stand a frozen one in its
  // place for the load. defineProperty throws on it, and so does the plain
  // assignment behind it, because bridge.js is strict. That aborted the IIFE
  // before window.__PILOT__ was installed: losing console capture is bad,
  // losing the bridge takes every other pilot command with it.
  const realConsole = globalThis.console;
  globalThis.console = Object.freeze({ ...REAL_CONSOLE });
  let errors;
  try {
    assert.doesNotThrow(() => (0, eval)(BRIDGE_SRC));
    errors = globalThis.window.__PILOT__.consoleLogs({ level: "error" });
  } finally {
    globalThis.console = realConsole;
  }

  // An empty buffer would read as a quiet page, the very silence #190 is
  // about, so the loss has to show up under `logs --level error`.
  assert.deepEqual(
    errors.map((e) => e.args[0]),
    ["log", "warn", "error", "info"].map(
      (level) => `tauri-pilot: console.${level} capture unavailable (console is frozen)`
    )
  );
});

test("an unconfigurable but writable console is still captured, even after a page assignment", () => {
  installGlobals();
  // defineProperty throws on these, so the bridge falls back to a plain
  // assignment: the only capture left on such a console. A page assignment
  // replaces that outright, so capture has to come back when the buffer is
  // read, as it does after a redefine.
  const stub = {};
  for (const level of Object.keys(REAL_CONSOLE)) {
    Object.defineProperty(stub, level, {
      value: REAL_CONSOLE[level],
      writable: true,
      configurable: false,
      enumerable: true,
    });
  }
  const realConsole = globalThis.console;
  globalThis.console = stub;
  const seen = [];
  let logged;
  try {
    (0, eval)(BRIDGE_SRC);
    const pilot = globalThis.window.__PILOT__;
    console.log("plain fallback");
    console.log = (...args) => { seen.push(args[0]); };
    pilot.consoleLogs();
    console.log("recovered");
    logged = messages(pilot);
  } finally {
    globalThis.console = realConsole;
  }

  assert.deepEqual(logged, ["plain fallback", "recovered"]);
  assert.deepEqual(seen, ["recovered"], "the page's replacement still runs");
});

test("capture recovers once the accessor has been redefined away", () => {
  const pilot = loadBridge();
  const seen = [];
  // React's dev build swaps console.* for plain data properties while it
  // builds a component stack, then puts back what it read. The accessor is
  // gone afterwards, so the next plain assignment displaces capture as in #190.
  const saved = console.log;
  const plain = (value) => ({ value, writable: true, configurable: true, enumerable: true });
  Object.defineProperty(console, "log", plain(() => {}));
  Object.defineProperty(console, "log", plain(saved));
  console.log = (...args) => { seen.push(args[0]); };

  // Nothing fires on a redefine, so reading the buffer is where it heals.
  pilot.consoleLogs();
  console.log("recovered");

  assert.deepEqual(messages(pilot), ["recovered"]);
  assert.deepEqual(seen, ["recovered"], "the page's replacement still runs");
});

test("an entry names the page function that logged it, not a wrapper", () => {
  const pilot = loadBridge();
  function pageCallerPlain() { console.log("plain"); }
  pageCallerPlain();
  const previous = console.log;
  console.log = (...args) => previous.apply(console, args);
  function pageCallerChained() { console.log("chained"); }
  pageCallerChained();

  const [plain, chained] = pilot.consoleLogs({ level: "log" });
  assert.match(plain.source, /pageCallerPlain/);
  assert.match(chained.source, /pageCallerChained/);
});

test("a replacement that fakes the Pilot marker is still chained", () => {
  const pilot = loadBridge();
  const seen = [];
  // The marker used to be a writable property on the view, so page code could
  // wear it: claiming this level got the assignment ignored, and claiming
  // another routed the call into that level's chain.
  const impostor = (...args) => { seen.push(args[0]); };
  impostor.__PILOT_CONSOLE__ = "log";
  console.log = impostor;

  console.log("mine");

  assert.deepEqual(seen, ["mine"], "the page's function must not be dropped");
  assert.deepEqual(messages(pilot), ["mine"]);
});

test("a log from a pilot eval names the eval, not the wrapper position", () => {
  // JavaScriptCore writes eval and `new Function` frames with no location and
  // ignores `//# sourceURL`, so the first frame with one was the eval wrapper
  // and `logs` showed `tauri://localhost:1:143` (#245). Stacks captured on
  // pilot-test-app under WebKitGTK 2.52: a stage that calls the script in
  // tail position loses the `__PILOT__evalScript` frame, one that calls it
  // inside `try` keeps it.
  const head = ["extractSource@user-script:6:74:30", "view@user-script:6:162:60"];
  for (const frames of [
    ["eval code@", "eval@[native code]", "__PILOT_EVAL__@tauri://localhost:1:143"],
    ["@", "anonymous@", "__PILOT__evalScript@user-script:6:1491:27", "__PILOT_EVAL__@tauri://localhost:1:165"],
  ]) {
    const pilot = loadBridge();
    const saved = Error.prepareStackTrace;
    Error.prepareStackTrace = () => [...head, ...frames, "global code@tauri://localhost:1:435"].join("\n");
    try {
      console.log("from eval");
    } finally {
      Error.prepareStackTrace = saved;
    }

    const [entry] = pilot.consoleLogs({ level: "log" });
    assert.equal(entry.source, "tauri-pilot-eval", frames.join(" | "));
  }
});

test("a log from a pilot eval under V8 names the eval too", () => {
  // V8 gives eval'd code a location (`eval at __PILOT__evalScript ...`), so without a
  // check the entry pointed into the bridge instead of naming the eval.
  const pilot = loadBridge();
  function __PILOT_EVAL__() {
    return pilot.eval({ script: 'console.log("from eval"); 1' });
  }
  __PILOT_EVAL__();

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.equal(entry.source, "tauri-pilot-eval");
});

// Replays `frames` below the bridge's own two frames, the way
// `extractSource` sees a console call, and returns the entry's source.
function sourceOfReplayedLog(frames) {
  const pilot = loadBridge();
  const head = ["extractSource@user-script:6:74:30", "view@user-script:6:162:60"];
  const saved = Error.prepareStackTrace;
  Error.prepareStackTrace = () => [...head, ...frames].join("\n");
  try {
    console.log("replayed");
  } finally {
    Error.prepareStackTrace = saved;
  }
  const [entry] = pilot.consoleLogs({ level: "log" });
  return entry.source;
}

test("an app frame called from a pilot eval keeps the log", () => {
  // Every bridge command (`click`, `fill`, ...) runs inside the eval wrapper,
  // so an app handler it triggers must not be filed as eval output.
  assert.equal(
    sourceOfReplayedLog([
      "onSave@tauri://localhost/assets/index.js:12:5",
      "eval code@",
      "__PILOT_EVAL__@tauri://localhost:1:143",
    ]),
    "onSave@tauri://localhost/assets/index.js:12:5",
  );

  const pilot = loadBridge();
  globalThis.appHandler = function appHandler() {
    console.log("from app");
  };
  try {
    (function __PILOT_EVAL__() {
      return pilot.eval({ script: "appHandler(); 1" });
    })();
  } finally {
    delete globalThis.appHandler;
  }
  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.match(entry.source, /appHandler/);
});

test("an app function or file named evalScript keeps the log", () => {
  // The bridge used to skip any frame matching `evalScript`, so the log
  // reported the caller instead of the line that called console.log.
  const frame = "evalScript@tauri://localhost/assets/evalScript.js:3:9";
  assert.equal(sourceOfReplayedLog([frame, "global code@tauri://localhost/assets/main.js:1:1"]), frame);

  const pilot = loadBridge();
  function evalScript() {
    console.log("from app");
  }
  evalScript();
  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.match(entry.source, /evalScript/);
});

test("a log after an await in a pilot eval under V8 names the eval", async () => {
  // V8 follows the `await` back to the wrapper with an `async __PILOT_EVAL__`
  // frame. WebKit keeps no such frame, so there the entry has no source.
  const pilot = loadBridge();
  await (async function __PILOT_EVAL__() {
    return await pilot.eval({ script: 'await Promise.resolve(); console.log("after await"); 1' });
  })();

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.equal(entry.source, "tauri-pilot-eval");
});

// An `await` inside a nested async function is not top-level (#272). The
// detector used to miss function bodies holding any `{`, so these scripts
// went to the async-statement wrapper, which has no completion value: the
// result was `null` and a rejection from the IIFE went unhandled.
test("eval returns the result of an async IIFE whose body has a nested block", async () => {
  const pilot = loadBridge();
  const cases = [
    ['const x = 1; (async () => { if (x) { await 0; } return "done"; })()', "done"],
    ['const x = 1; (async () => { try { await 0; } catch (e) {} return "done"; })()', "done"],
    ["const x = 1; (async () => { const o = { a: 1 }; await 0; return o.a; })()", 1],
    // `await (expr)` compiles in both modes, so the text scan decides.
    ['const x = 1; (async () => { if (x) { await (Promise.resolve(2)); } return "done"; })()', "done"],
    // Methods and parameter defaults holding parentheses kept `await (`
    // visible to the old text scan.
    ["const o = { async load() { if (1) { return (await (Promise.resolve({ v: 9 }))).v; } } }; o.load()", 9],
    ["class C { async m() { return await (Promise.resolve(3)); } } new C().m()", 3],
    ["async function load(u = String('/a')) { if (u) { return (await (Promise.resolve({ v: 9 }))).v; } } load()", 9],
    ["const x = 1; (async (n = Number('1')) => { const r = await (Promise.resolve(n)); return r; })()", 1],
    // More than six blocks deep: every level takes one pass of the masker.
    [
      "const x = 1; (async () => { if (x) { if (x) { if (x) { if (x) { if (x) { if (x) { if (x) { await (0); } } } } } } } return 8; })()",
      8,
    ],
    // Controls that already worked.
    ['const x = 1; (async () => { await 0; return "done"; })()', "done"],
  ];
  for (const [script, expected] of cases) {
    assert.equal(await pilot.eval({ script }), expected, script);
  }
});

test("eval rejects when an async IIFE with a nested block throws", async () => {
  const pilot = loadBridge();
  for (const script of [
    'const x = 1; (async () => { if (x) { await 0; throw new Error("boom"); } })()',
    "const o = { async f() { await (0); throw new Error('boom'); } }; o.f()",
    // Control that already worked.
    '(async () => { if (1) { await 0; throw new Error("boom"); } })()',
  ]) {
    await assert.rejects(Promise.resolve().then(() => pilot.eval({ script })), /boom/, script);
  }
});

test("eval keeps the completion value when await only appears in a literal or a class method", async () => {
  // Former false positives of the regex detector: they were wrapped and
  // returned `null`.
  const pilot = loadBridge();
  const cases = [
    ["const s = `await`; s", "await"],
    ["const r = /await/; r.source", "await"],
    ["class C { async m() { await 0; return 1; } } new C().m()", 1],
    ["const s = `await (x)`; s", "await (x)"],
    ["const r = /await (x)/; r.source", "await (x)"],
    ['const s = "await (x)"; s // await (y)', "await (x)"],
  ];
  for (const [script, expected] of cases) {
    assert.equal(await pilot.eval({ script }), expected, script);
  }
});

test("eval still wraps top-level await, including the await (expr) form", async () => {
  const pilot = loadBridge();
  const cases = [
    ['await Promise.resolve("hi")', "hi"],
    ["const v = await Promise.resolve(2); return v * 3", 6],
    ["const x = 1; if (x) { await 0; } return 7", 7],
    ["const p = Promise.resolve(5); return await (p)", 5],
    ["const p = Promise.resolve(4); return await [p][0]", 4],
    // Sloppy code also reads these as the identifier `await`: ASI before a
    // line break, a binary operator, a tagged template.
    ["const data = await\n  Promise.resolve({ v: 9 });\nreturn data.v", 9],
    ["const n = 2; const v = await +n; return v", 2],
    ["const v = await `x`; return v", "x"],
    // `with` is sloppy-only, so the per-await probe gives up and the text
    // scan decides.
    ["var o = { k: 1 }; with (o) { k; }\nconst data = await\n  Promise.resolve({ v: 9 });\nreturn data.v", 9],
  ];
  for (const [script, expected] of cases) {
    assert.equal(await pilot.eval({ script }), expected, script);
  }
});

test("eval does not let a script close the await probe's wrapper", () => {
  // Pasted into an arrow body, this script closes it and compiles, so it was
  // taken for top-level await and returned nothing instead of failing.
  const pilot = loadBridge();
  assert.throws(() => pilot.eval({ script: "}); 1; (() => {" }), SyntaxError);
});

test("eval keeps the auto-wrap hint for a broken script with top-level await", () => {
  const pilot = loadBridge();
  assert.throws(() => pilot.eval({ script: "const v = await 1; }" }), /could not be auto-wrapped/);
});

test("eval gives a plain syntax error when a broken script's await is deeply nested", () => {
  // The await sits in a function more than six blocks deep: once peeled, it is
  // not top-level, so the script must not get the auto-wrap hint.
  const pilot = loadBridge();
  const script =
    "(async () => { if (1) { if (1) { if (1) { if (1) { if (1) { if (1) { if (1) { await 0; } } } } } } } })(); }";
  assert.throws(
    () => pilot.eval({ script }),
    (e) => e instanceof SyntaxError && !/could not be auto-wrapped/.test(e.message),
  );
});

// Console arguments are snapshotted when they are logged (#274).
//
// serializeArg kept the live reference and only checked that JSON.stringify
// did not throw, so an object was rendered with its state at read time, and an
// Error, Map, Set or DOM node came out as {} because none of them has
// enumerable own properties.

// Stand-ins for the DOM: as in a browser, nodeType is an accessor on
// Node.prototype, which is what tells a node from an object with the same
// fields. Installed per test because loadBridge resets the globals.
class FakeNode {
  get nodeType() { return this._type; }
}
class FakeElement extends FakeNode {
  constructor(localName, id = "", className = "") {
    super();
    this._type = 1;
    this.nodeName = localName.toUpperCase();
    this.localName = localName;
    this.id = id;
    this.className = className;
  }
}

function loadBridgeWithDom() {
  const pilot = loadBridge();
  globalThis.Node = FakeNode;
  return pilot;
}

after(() => { delete globalThis.Node; });

test("an object is logged with its state at call time", () => {
  const pilot = loadBridge();
  const o = { a: 1 };
  console.log("state", o);
  o.a = 2;

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args, ["state", { a: 1 }]);
});

test("errors, collections, nodes and undefined say what they are", () => {
  const pilot = loadBridgeWithDom();
  console.log(
    "err",
    new TypeError("bad input"),
    new Map([["k", 1]]),
    new Set([1]),
    new FakeElement("body"),
    undefined,
    [new Error("in array")],
  );

  const [entry] = pilot.consoleLogs({ level: "log" });
  const expected = [
    "err",
    "TypeError: bad input",
    { __type: "Map", size: 1, entries: [["k", 1]] },
    { __type: "Set", size: 1, values: [1] },
    "<body>",
    "undefined",
    ["Error: in array"],
  ];
  assert.deepEqual(entry.args, expected);
  // What the CLI receives is the JSON form of the entry, so nothing may be
  // lost or changed on the way through.
  assert.deepEqual(JSON.parse(JSON.stringify(entry.args)), expected);
});

test("an element names its tag, id and classes", () => {
  const pilot = loadBridgeWithDom();
  console.log(new FakeElement("button", "save", "primary  wide"), { target: new FakeElement("div") });

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args, ["<button#save.primary.wide>", { target: "<div>" }]);
});

test("an object with node-like fields that is not a node is logged as data", () => {
  const pilot = loadBridgeWithDom();
  class AstNode {
    constructor() { this.nodeType = 3; this.nodeName = "Ident"; this.value = 42; }
  }
  console.log({ nodeType: 1, nodeName: "row" }, new AstNode());

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args, [
    { nodeType: 1, nodeName: "row" },
    { nodeType: 3, nodeName: "Ident", value: 42 },
  ]);
});

test("nested values follow the same rules as top-level ones", () => {
  const pilot = loadBridge();
  const inner = { n: 1 };
  console.log({
    err: new RangeError("too far"),
    map: new Map([["inner", inner]]),
    missing: undefined,
    when: new Date(0),
  });
  inner.n = 2;

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args[0], {
    err: "RangeError: too far",
    map: { __type: "Map", size: 1, entries: [["inner", { n: 1 }]] },
    missing: "undefined",
    when: "1970-01-01T00:00:00.000Z",
  });
});

test("a cyclic object is logged instead of falling back to [object Object]", () => {
  const pilot = loadBridge();
  const o = { name: "loop" };
  o.self = o;
  console.log(o);

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args[0], { name: "loop", self: "[Circular]" });
});

test("deep nesting and large containers are capped", () => {
  const pilot = loadBridge();
  let deep = { leaf: true };
  for (let i = 0; i < 20; i++) deep = { child: deep };
  const big = Array.from({ length: 1000 }, (_, i) => i);
  console.log(deep, big);

  const [entry] = pilot.consoleLogs({ level: "log" });
  const text = JSON.stringify(entry.args[0]);
  assert.doesNotMatch(text, /leaf/, "nesting past the depth cap is cut");
  assert.match(text, /\[Object\]/);
  assert.equal(entry.args[1].length, 101, "100 items plus one marker");
  assert.equal(entry.args[1][99], 99);
  assert.equal(entry.args[1][100], "... 900 more items");
});

test("a wide graph is capped in total, not only per level", () => {
  // 100 rows of 100 objects: every container is under the per-level cap, but
  // the whole graph holds 10,101 objects.
  const pilot = loadBridge();
  const row = () => Object.fromEntries(Array.from({ length: 100 }, (_, i) => ["k" + i, { v: i }]));
  const wide = Object.fromEntries(Array.from({ length: 100 }, (_, i) => ["r" + i, row()]));
  console.log(wide);

  const [entry] = pilot.consoleLogs({ level: "log" });
  const text = JSON.stringify(entry.args[0]);
  assert.ok(text.length < 100_000, `snapshot is ${text.length} chars`);
  assert.match(text, /\[Object\]/, "objects past the total cap are cut");
});

test("an object that throws while being read does not break the log", () => {
  const pilot = loadBridge();
  const hostile = {};
  Object.defineProperty(hostile, "boom", { enumerable: true, get() { throw new Error("no"); } });
  console.log("before", hostile);

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args, ["before", { boom: "[unreadable]" }]);
});

test("a throwing nodeType or toJSON still leaves the object's keys", () => {
  const pilot = loadBridgeWithDom();
  const trap = { a: 1 };
  Object.defineProperty(trap, "nodeType", { get() { throw new Error("no"); } });
  const badJson = { b: 2, toJSON() { throw new Error("no"); } };
  console.log(trap, badJson);

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args[0], { a: 1 });
  assert.equal(entry.args[1].b, 2);
});

test("a large Map keeps its first entries and says how many it dropped", () => {
  const pilot = loadBridge();
  console.log(new Map(Array.from({ length: 1000 }, (_, i) => [i, i])));

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.equal(entry.args[0].size, 1000);
  assert.equal(entry.args[0].entries.length, 100);
  assert.deepEqual(entry.args[0].entries[99], [99, 99]);
  assert.equal(entry.args[0].truncated, 900);
});

test("long strings inside a logged object are cut", () => {
  const pilot = loadBridge();
  const long = "x".repeat(50_000);
  console.log(long, { body: long });

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.equal(entry.args[0], long, "a top-level string argument is kept whole");
  assert.equal(entry.args[1].body, "x".repeat(10_000) + "... (40000 more chars)");
});

test("an array hole is logged as null, not as undefined", () => {
  const pilot = loadBridge();
  // eslint-disable-next-line no-sparse-arrays
  console.log([1, , undefined]);

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(entry.args[0], [1, null, "undefined"]);
});

test("NaN and Infinity keep their names instead of becoming null", () => {
  const pilot = loadBridge();
  console.log(NaN, { inf: -Infinity }, 1.5);

  const [entry] = pilot.consoleLogs({ level: "log" });
  assert.deepEqual(JSON.parse(JSON.stringify(entry.args)), ["NaN", { inf: "-Infinity" }, 1.5]);
});
