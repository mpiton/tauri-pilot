#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/msrv.sh          print the MSRV
#        ./scripts/msrv.sh --check  fail when a copy of it has drifted
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

if [ "${1:-}" != "--check" ]; then
  echo "$MSRV"
  exit 0
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

# A numbered toolchain in a workflow is a second copy of the MSRV, or a pin
# that silently ages. Workflows use `stable`, or read the MSRV from here.
PINNED="(toolchain:[[:space:]]*[\"']?|rust-toolchain@|(cargo|rustc|rustdoc) \\+|Install Rust |rustup (run|toolchain install|default|override set) |RUSTUP_TOOLCHAIN[:=][[:space:]]*[\"']?)[0-9]+\\.[0-9]+"
if pinned="$(grep -rnE "$PINNED" .github/workflows)"; then
  echo "Error: hard-coded Rust version in a workflow; use stable or scripts/msrv.sh:" >&2
  echo "$pinned" >&2
  STATUS=1
fi

if [ "$STATUS" -eq 0 ]; then
  echo "MSRV $MSRV: Cargo.toml, ${#DOCS[@]} docs and the workflows agree"
fi
exit "$STATUS"
