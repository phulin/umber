# umber-vfs Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
host-neutral virtual filesystem primitives shared by native and WebAssembly
document pipelines.

## Crate Role

`umber-vfs` currently owns canonical `/job` and `/texlive` virtual paths. Its
future transaction, immutable-file, resource-registration, and accounting
boundaries are specified in `docs/umber_vfs.md`.

## Boundaries

- Do not access host filesystems, environment variables, networks, processes,
  locale rules, or platform path canonicalization.
- Keep TeX input search, default extensions, and resource acquisition policy
  in their domain-specific callers.
- Preserve exact path bytes after syntax-only canonicalization.

## File Map

- `Cargo.toml`: package metadata, test dependency, and workspace lints.
- `src/lib.rs`: canonical virtual path and typed error public API.
- `src/tests.rs`: focused canonicalization parity and property tests.

## Validation

Run `cargo test --tests -p umber-vfs` after changes. The crate must also compile
for `wasm32-unknown-unknown`.
