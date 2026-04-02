#!/usr/bin/env bash
# Integrate tauri-pilot plugin into Prism for development testing.
# Run from the tauri-pilot project root.
set -euo pipefail

PRISM_DIR="${1:-../prism}"

if [ ! -f "$PRISM_DIR/src-tauri/Cargo.toml" ]; then
  echo "Error: Prism not found at $PRISM_DIR"
  echo "Usage: $0 [prism-directory]"
  exit 1
fi

echo "Adding tauri-plugin-pilot dependency to Prism..."
cd "$PRISM_DIR/src-tauri"
cargo add tauri-plugin-pilot --path "../../tauri-pilot/crates/tauri-plugin-pilot"

echo "Done! Now add this to $PRISM_DIR/src-tauri/src/lib.rs after .plugin(tauri_plugin_opener::init()):"
echo ""
echo '    #[cfg(debug_assertions)]'
echo '    let builder = builder.plugin(tauri_plugin_pilot::init());'
echo ""
echo "Then run: cd $PRISM_DIR && cargo tauri dev"
echo "Test with: tauri-pilot ping"
