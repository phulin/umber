# Umber Distribution Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
strict, host-neutral contract for immutable distribution manifests.

## Boundaries

- Keep the crate dependency-free, I/O-free, and compatible with
  `wasm32-unknown-unknown`.
- Validate all untrusted manifest structure in the parser. Consumers must not
  derive object URLs or interpret unchecked manifest fields themselves.
- Keep request-key encoding and deterministic job/miss selection here so native
  hosts and the authored JavaScript can share fixtures without sharing I/O.
- `src/json.rs` is only the private strict JSON substrate; schema policy belongs
  in `src/manifest.rs` and the focused HTML record module below.
- `src/html.rs` owns the schema-9/schema-2 HTML font and exact legacy-mapping
  records; `src/ahash64.rs` is the dependency-free canonical shard-index and
  packed-table hash.
- `src/catalog.rs` owns canonical publication partitioning and complete
  root/shard graph assembly. Publisher and host adapters must not reconstruct
  these invariants.
- `src/packed.rs` owns canonical packed-shard encoding plus the safe validated
  byte view. Native and WebAssembly runtime selection must use this view and
  must not restore a JSON shard, per-record materialization, or `BTreeMap` hot
  path. `ManifestShard` remains a publisher/test construction model only.

## Validation

Run `cargo test -p umber-distribution --tests`, the authored JavaScript tests,
and a `wasm32-unknown-unknown` check after contract changes.
