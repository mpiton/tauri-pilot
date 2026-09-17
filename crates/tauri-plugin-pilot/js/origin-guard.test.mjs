// Field comparison used by wrap_script (#173).
//
// The wrapper injects scheme, hostname, and Url::port_or_known_default (or
// the hostless href) from the checked URL. This mocks `location` the way a
// page would see those URLs and checks the comparison against origin_key
// grouping: same origin must match, a different origin must not.
//
// Run: node --test crates/tauri-plugin-pilot/js/origin-guard.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";

function mockLocation(href) {
  const url = new URL(href);
  return {
    protocol: url.protocol,
    hostname: url.hostname,
    port: url.port,
    href: url.href,
  };
}

function mismatch(location, checked) {
  if (checked.hostless) {
    return location.href.split("#")[0] !== checked.hostless;
  }
  if (checked.port === null) {
    return (
      location.protocol !== checked.protocol
      || location.hostname !== checked.hostname
      || location.port !== ""
    );
  }
  return (
    location.protocol !== checked.protocol
    || location.hostname !== checked.hostname
    || (location.port || checked.port) !== checked.port
  );
}

const cases = [
  {
    name: "https://host/",
    href: "https://host/",
    checked: { protocol: "https:", hostname: "host", port: "443" },
    same: ["https://host/login", "https://host:443/"],
    other: ["http://host/", "https://other/", "tauri://localhost/"],
  },
  {
    name: "http://host/",
    href: "http://host/",
    checked: { protocol: "http:", hostname: "host", port: "80" },
    same: ["http://host/x", "http://host:80/"],
    other: ["https://host/", "http://other/"],
  },
  {
    name: "tauri://localhost/",
    href: "tauri://localhost/",
    checked: { protocol: "tauri:", hostname: "localhost", port: null },
    same: ["tauri://localhost/settings"],
    other: ["https://host/", "tauri://other/"],
  },
  {
    name: "file:///tmp/a.html#frag",
    href: "file:///tmp/a.html#frag",
    checked: { hostless: "file:///tmp/a.html" },
    same: ["file:///tmp/a.html", "file:///tmp/a.html#other"],
    other: ["file:///tmp/b.html", "https://host/"],
  },
];

for (const c of cases) {
  test(`${c.name} matches origin_key grouping`, () => {
    assert.equal(mismatch(mockLocation(c.href), c.checked), false);
    for (const href of c.same) {
      assert.equal(mismatch(mockLocation(href), c.checked), false, href);
    }
    for (const href of c.other) {
      assert.equal(mismatch(mockLocation(href), c.checked), true, href);
    }
  });
}
