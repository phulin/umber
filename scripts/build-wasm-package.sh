#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
crate="${repo_root}/crates/umber-wasm"
output="${1:-${repo_root}/target/umber-wasm-package}"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

wasm-pack build "$crate" --target web --release --out-dir "$temporary/generated"

mkdir -p "$temporary/package/assets" "$temporary/package/examples"
cp "$crate/package.json" "$crate/THIRD_PARTY_NOTICES.md" "$temporary/package/"
cp "$temporary/generated/umber_wasm.js" \
  "$temporary/generated/umber_wasm.d.ts" \
  "$temporary/generated/umber_wasm_bg.wasm" \
  "$temporary/package/"
for module in "$crate"/js/*.js; do
  [[ "$module" == *.test.js ]] || cp "$module" "$temporary/package/"
done
cp "$crate"/js/*.d.ts "$temporary/package/"
cp "$crate"/assets/plain.fmt \
  "$crate"/assets/plain-format.json \
  "$crate"/assets/plain-source.lock \
  "$temporary/package/assets/"
cp "$crate"/examples/index.html "$crate"/examples/main.js "$temporary/package/examples/"

rm -rf "$output"
mkdir -p "$(dirname "$output")"
mv "$temporary/package" "$output"

printf 'Umber WASM package: %s\n' "$output"
