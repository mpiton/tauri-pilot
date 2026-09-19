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
