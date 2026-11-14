#!/usr/bin/env bash
set -uo pipefail

# Usage: ./scripts/msrv.test.sh
#
# Runs `scripts/msrv.sh` against a throwaway tree: this repository's workflows,
# plus a `Cargo.toml` and docs written here with a made-up MSRV, so the expected
# values below do not come from the files the script reads in the real tree.
# Each case then adds one line to a workflow, or edits one doc, and states the
# exit code the guard must return.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MSRV="1.70.0"   # the fixture's MSRV
OTHER="1.80.1"  # a toolchain number that is not the MSRV

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

FAILED=0
CASES=0

# Rebuilds the fixture tree from scratch, so no case sees the previous one.
fixture() {
  rm -rf "$WORK/tree"
  mkdir -p "$WORK/tree/scripts" "$WORK/tree/.github" "$WORK/tree/docs/src/content/docs"
  cp "$SCRIPT_DIR/msrv.sh" "$WORK/tree/scripts/msrv.sh"
  cp -R "$ROOT_DIR/.github/workflows" "$WORK/tree/.github/workflows"
  printf '[workspace.package]\nrust-version = "%s"\n' "$MSRV" > "$WORK/tree/Cargo.toml"
  printf '![Rust](https://img.shields.io/badge/rust-%s+-orange)\n' "$MSRV" > "$WORK/tree/README.md"
  printf -- '- Rust %s+ with edition 2024\n' "$MSRV" > "$WORK/tree/CONTRIBUTING.md"
  printf -- '- Rust %s+ with edition 2024\n' "$MSRV" > "$WORK/tree/docs/src/content/docs/contributing.md"
  printf 'You need Rust %s+.\n' "$MSRV" > "$WORK/tree/docs/src/content/docs/getting-started.md"
}

# expect <exit code> <case name> [msrv.sh arguments...]
expect() {
  local want="$1" name="$2" got
  shift 2
  CASES=$((CASES + 1))
  "$WORK/tree/scripts/msrv.sh" "$@" > "$WORK/stdout" 2> "$WORK/stderr"
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name: exit $got, expected $want" >&2
    sed 's/^/    /' "$WORK/stdout" "$WORK/stderr" >&2
    FAILED=$((FAILED + 1))
  fi
}

# workflow_line <exit code> <line>: the guard's verdict on a workflow that
# gains this one line.
workflow_line() {
  fixture
  printf '%s\n' "$2" > "$WORK/tree/.github/workflows/fixture.yml"
  expect "$1" "workflow line: $2" --check
}

# --- Arguments -------------------------------------------------------------

fixture
expect 0 "no argument prints the MSRV"
CASES=$((CASES + 1))
if [ "$(cat "$WORK/stdout")" != "$MSRV" ]; then
  echo "FAIL: no argument printed '$(cat "$WORK/stdout")', expected '$MSRV'" >&2
  FAILED=$((FAILED + 1))
fi
expect 0 "--check passes on a tree in sync" --check
expect 2 "a misspelled flag is refused" --chek
CASES=$((CASES + 1))
if [ -s "$WORK/stdout" ] || ! grep -q 'Usage' "$WORK/stderr"; then
  echo "FAIL: a misspelled flag must print the usage on stderr and nothing on stdout" >&2
  FAILED=$((FAILED + 1))
fi
expect 2 "an unknown flag is refused" --help
expect 2 "a second argument is refused" --check extra

# --- Docs ------------------------------------------------------------------

fixture
printf -- '- Rust %s+ with edition 2024\n' "$OTHER" > "$WORK/tree/CONTRIBUTING.md"
expect 1 "a doc quoting another version" --check

fixture
printf 'You need Rust.\n' > "$WORK/tree/docs/src/content/docs/getting-started.md"
expect 1 "a doc that no longer quotes a version" --check

fixture
rm "$WORK/tree/README.md"
expect 1 "a listed doc that is missing" --check

fixture
printf '[workspace.package]\nrust-version = "%s"\n' "$OTHER" > "$WORK/tree/Cargo.toml"
expect 1 "Cargo.toml moving without the docs" --check

# --- Workflows: a numbered toolchain, the MSRV or another one -------------------

for v in "$MSRV" "$OTHER"; do
  workflow_line 1 "          toolchain: $v"
  workflow_line 1 "          toolchain: \"$v\""
  workflow_line 1 "        toolchain: [stable, \"$v\"]"
  workflow_line 1 "        rust: [\"$v\"]"
  workflow_line 1 "      - uses: dtolnay/rust-toolchain@$v"
  workflow_line 1 "      - uses: dtolnay/rust-toolchain@0123456789abcdef0123456789abcdef01234567 # $v"
  workflow_line 1 "        run: cargo +$v check --workspace"
  workflow_line 1 "        run: rustc +$v --version"
  workflow_line 1 "        run: rustdoc +$v --version"
  workflow_line 1 "        run: rustup run $v cargo check"
  workflow_line 1 "        run: rustup install $v"
  workflow_line 1 "        run: rustup update $v"
  workflow_line 1 "        run: rustup default $v"
  workflow_line 1 "        run: rustup override set $v"
  workflow_line 1 "        run: rustup toolchain add $v"
  workflow_line 1 "        run: rustup toolchain install $v --profile minimal"
  workflow_line 1 "        run: rustup component add clippy --toolchain $v"
  workflow_line 1 "        run: sh rustup-init.sh -y --default-toolchain $v"
  workflow_line 1 "        run: sh rustup-init.sh -y --default-toolchain=$v"
  workflow_line 1 "      RUSTUP_TOOLCHAIN: $v"
  workflow_line 1 "        run: RUSTUP_TOOLCHAIN=$v cargo check"
  workflow_line 1 "      MSRV: \"$v\""
  workflow_line 1 "      RUST_VERSION: $v"
  workflow_line 1 "      - name: Install Rust $v"
done
workflow_line 1 "          toolchain: '1.80'"

# Spellings no pattern lists: only the search for the MSRV itself sees them.
workflow_line 1 "      - name: Build on $MSRV"
fixture
printf '          toolchain:\n            %s\n' "$MSRV" > "$WORK/tree/.github/workflows/fixture.yml"
expect 1 "workflow: toolchain value on the line after its key" --check

# --- Workflows: lines the guard must leave alone -------------------------------

workflow_line 0 "          toolchain: stable"
workflow_line 0 "          toolchain: \${{ env.MSRV }}"
workflow_line 0 "      - uses: dtolnay/rust-toolchain@stable"
workflow_line 0 "      - uses: dtolnay/rust-toolchain@master"
workflow_line 0 "      NIGHTLY: nightly-2026-11-13"
workflow_line 0 "      MSRV: \${{ needs.rust-version.outputs.msrv }}"
workflow_line 0 "        run: cargo +\${{ env.MSRV }} check --workspace --all-targets"
workflow_line 0 "        run: rustup target add aarch64-linux-android"
workflow_line 0 "          node-version: 26"
workflow_line 0 "      - uses: actions/setup-node@v7"

if [ "$FAILED" -ne 0 ]; then
  echo "$FAILED of $CASES checks failed" >&2
  exit 1
fi
echo "$CASES checks passed"
