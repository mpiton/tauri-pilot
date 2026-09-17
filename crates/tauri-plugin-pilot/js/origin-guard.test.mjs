// Field comparison used by wrap_script (#173).
//
// The wrapper injects scheme, hostname, and Url::port (empty when the port
// is the scheme default, matching location.port) or the hostless href. Same
// origin must match; a different origin must not. An empty location.port
// must not inherit a non-default checked port (https://host:8443 vs
// https://host/).
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
  return (
    location.protocol !== checked.protocol
    || location.hostname !== checked.hostname
    || location.port !== checked.port
  );
}

const cases = [
  {
    name: "https://host/",
    href: "https://host/",
    checked: { protocol: "https:", hostname: "host", port: "" },
    same: ["https://host/login", "https://host:443/"],
    other: ["http://host/", "https://other/", "tauri://localhost/", "https://host:8443/"],
  },
  {
    name: "http://host/",
    href: "http://host/",
    checked: { protocol: "http:", hostname: "host", port: "" },
    same: ["http://host/x", "http://host:80/"],
    other: ["https://host/", "http://other/", "http://host:8080/"],
  },
  {
    name: "https://host:8443/",
    href: "https://host:8443/",
    checked: { protocol: "https:", hostname: "host", port: "8443" },
    same: ["https://host:8443/login"],
    other: ["https://host/", "https://host:443/"],
  },
  {
    name: "tauri://localhost/",
    href: "tauri://localhost/",
    checked: { protocol: "tauri:", hostname: "localhost", port: "" },
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
