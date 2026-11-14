#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/msrv.sh          print the MSRV
#        ./scripts/msrv.sh --check  fail when a copy of it has drifted
# Anything else prints the usage and exits 2. Tests: ./scripts/msrv.test.sh
#
# The MSRV is `rust-version` in the root Cargo.toml and nowhere else. CI reads
# it through this script, so bumping it is a one-line change there plus the
# docs that quote it for readers. `--check` keeps those two promises: the docs
# quote the same version, and no workflow hard-codes a toolchain number.

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

# Docs that tell a reader which Rust they need. Each must quote the MSRV.
DOCS=(
  README.md
  CONTRIBUTING.md
  docs/src/content/docs/contributing.md
  docs/src/content/docs/getting-started.md
)

MSRV="$(sed -n 's/^rust-version = "\([^"]*\)".*/\1/p' Cargo.toml | tr -d '\r')"
if ! [[ "$MSRV" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
  echo "Error: expected exactly one 'rust-version = \"x.y.z\"' in Cargo.toml, got '$MSRV'" >&2
  exit 1
fi

# A misspelled `--check` must not print the MSRV and exit 0: its caller is a
# required CI step, which would then stay green while checking nothing.
if [ "$#" -eq 0 ]; then
  echo "$MSRV"
  exit 0
fi
if [ "$#" -ne 1 ] || [ "$1" != "--check" ]; then
  echo "Usage: $0 [--check]" >&2
  exit 2
fi

STATUS=0

# Matches the badge (`rust-1.95.0+-orange`), prose (`Rust 1.95.0+`) and the
# code standards line (`rust-version 1.95.0`).
QUOTED='(rust-|Rust |rust-version )[0-9]+\.[0-9]+(\.[0-9]+)?'
for doc in "${DOCS[@]}"; do
  if [ ! -f "$doc" ]; then
    echo "Error: $doc is missing; update DOCS in scripts/msrv.sh" >&2
    STATUS=1
    continue
  fi
  quotes="$(grep -noE "$QUOTED" "$doc" || true)"
  if [ -z "$quotes" ]; then
    echo "Error: $doc no longer quotes a Rust version; expected $MSRV" >&2
    STATUS=1
    continue
  fi
  while IFS= read -r quote; do
    version="$(echo "$quote" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?$')"
    if [ "$version" != "$MSRV" ]; then
      echo "Error: $doc:${quote%%:*} quotes Rust $version, Cargo.toml says $MSRV" >&2
      STATUS=1
    fi
  done <<< "$quotes"
done

# The workflows read the MSRV from this script, so the number itself has no
# reason to appear in one. Searching for it does not depend on how a pin is
# spelled: a matrix list, a value on the line after its key, a comment next to
# a commit SHA.
if copies="$(grep -rnF -- "$MSRV" .github/workflows)"; then
  echo "Error: the MSRV ($MSRV) is written out in a workflow; read it from scripts/msrv.sh:" >&2
  echo "$copies" >&2
  STATUS=1
fi

# Any other numbered toolchain is a pin that silently ages: workflows use
# `stable`. A number has no fixed text to search for, so these are the
# spellings known to select a toolchain. `scripts/msrv.test.sh` lists them.
V='[0-9]+\.[0-9]+'
PINNED="(toolchain:.*|rust-toolchain@.*|(^|[[:space:]])rust(-version)?:.*|(cargo|rustc|rustdoc) \\+|Install Rust |rustup (run|install|update|default|override set|toolchain (install|add)) |--(default-)?toolchain[= ]|[A-Z_]*(MSRV|TOOLCHAIN|RUST_VERSION)[A-Z_]*[:=][[:space:]]*[\"']?)$V"
if pinned="$(grep -rnE "$PINNED" .github/workflows)"; then
  echo "Error: hard-coded Rust version in a workflow; use stable or scripts/msrv.sh:" >&2
  echo "$pinned" >&2
  STATUS=1
fi

if [ "$STATUS" -eq 0 ]; then
  echo "MSRV $MSRV: Cargo.toml, ${#DOCS[@]} docs and the workflows agree"
fi
exit "$STATUS"
