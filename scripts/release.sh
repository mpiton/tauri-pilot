#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh 0.2.0
# Bumps version in all Cargo.toml files, updates CHANGELOG.md,
# commits and tags the release; push separately to trigger the release workflow.

VERSION="${1:-}"

# Validate argument
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>"
  echo "Example: $0 0.2.0"
  exit 1
fi

# Validate semver format
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
  echo "Error: '$VERSION' is not a valid semver version"
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

# Preflight: ensure clean worktree
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: working tree is not clean. Commit or stash changes before releasing."
  git status --short
  exit 1
fi

# Preflight: ensure tag does not already exist
if git rev-parse -q --verify "refs/tags/v$VERSION" > /dev/null 2>&1; then
  echo "Error: tag v$VERSION already exists"
  exit 1
fi

echo "Releasing v$VERSION..."

# Update [package].version in all crate Cargo.toml files
for toml in crates/tauri-plugin-pilot/Cargo.toml crates/tauri-pilot-cli/Cargo.toml; do
  perl -i -pe "s/^version = \".*\"/version = \"$VERSION\"/" "$toml"
  echo "  Updated $toml"
done

# Update CHANGELOG.md
TODAY=$(date +%Y-%m-%d)
# Replace "## [Unreleased]" with "## [Unreleased]\n\n## [$VERSION] - $TODAY"
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
  echo "Error: CHANGELOG.md is missing '## [Unreleased]' header"
  exit 1
fi
if grep -q "## \[$VERSION\]" CHANGELOG.md; then
  echo "Error: CHANGELOG.md already has an entry for $VERSION"
  exit 1
fi
perl -i -pe "s/^## \\[Unreleased\\]/## [Unreleased]\n\n## [$VERSION] - $TODAY/" CHANGELOG.md

# Update comparison links at bottom of CHANGELOG
# Add new unreleased comparison link and version link
if grep -q "\[Unreleased\]:.*compare" CHANGELOG.md; then
  perl -i -pe "s|\[Unreleased\]:.*|[Unreleased]: https://github.com/mpiton/tauri-pilot/compare/v$VERSION...HEAD|" CHANGELOG.md
else
  echo "" >> CHANGELOG.md
  echo "[Unreleased]: https://github.com/mpiton/tauri-pilot/compare/v$VERSION...HEAD" >> CHANGELOG.md
fi

# Add version link if not present — insert immediately after the [Unreleased]
# definition so every version link stays grouped together instead of being
# appended to the end of the file on each release.
# Prefer compare/vPREV...vNEW format for consistency with prior entries; fall back
# to releases/tag/ only for the first-ever release (no previous tag exists yet).
if ! grep -q "\[$VERSION\]:" CHANGELOG.md; then
  PREVIOUS_TAG=$(git tag -l 'v*' --sort=-v:refname | grep -vx "v$VERSION" | head -n1 || true)
  if [ -n "$PREVIOUS_TAG" ]; then
    NEW_LINK="[$VERSION]: https://github.com/mpiton/tauri-pilot/compare/$PREVIOUS_TAG...v$VERSION"
  else
    NEW_LINK="[$VERSION]: https://github.com/mpiton/tauri-pilot/releases/tag/v$VERSION"
  fi
  perl -i -pe "s|(\[Unreleased\]:.*)|\$1\n$NEW_LINK|" CHANGELOG.md
fi

echo "  Updated CHANGELOG.md"

# Verify compilation and quality
echo "Running cargo check..."
cargo check --workspace

echo "Running cargo clippy..."
cargo clippy --workspace -- -D warnings

echo "Running cargo test..."
cargo test --workspace

echo "Running cargo fmt..."
cargo fmt --all

# Git operations — stage all changes (release files + any fmt-touched files)
git add -A
git commit -m "chore: release v$VERSION"
git tag -a "v$VERSION" -m "Release v$VERSION"

echo ""
echo "Release v$VERSION prepared. Run the following to publish:"
echo ""
echo "  git push && git push --tags"
echo ""
