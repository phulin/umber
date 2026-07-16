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
  in `src/manifest.rs`.

## Validation

Run `cargo test -p umber-distribution --tests`, the authored JavaScript tests,
and a `wasm32-unknown-unknown` check after contract changes.
