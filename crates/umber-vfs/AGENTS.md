# umber-vfs Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
host-neutral virtual filesystem primitives shared by native and WebAssembly
document pipelines.

## Crate Role

`umber-vfs` owns canonical `/job` and `/texlive` virtual paths plus immutable
file identity and deterministic four-layer storage. Its future transaction,
resource-registration, and accounting boundaries are specified in
`docs/umber_vfs.md`.

## Boundaries

- Do not access host filesystems, environment variables, networks, processes,
  locale rules, or platform path canonicalization.
- Keep TeX input search, default extensions, and resource acquisition policy
  in their domain-specific callers.
- Preserve exact path bytes after syntax-only canonicalization.

## File Map

- `Cargo.toml`: package metadata, test dependency, and workspace lints.
- `src/lib.rs`: canonical virtual path API and crate exports.
- `src/file.rs`: immutable shared file values, provenance, and identities.
- `src/limits.rs`: checked file-count and byte limits shared by all VFS clients.
- `src/resource.rs`: typed resource requests, deterministic batches, and atomic provisioning.
- `src/resource/tests.rs`: request, registration, conflict, limit, and retry tests.
- `src/snapshot.rs`: immutable generation snapshots, exact reads, invalidation, and bounded enumeration.
- `src/snapshot/tests.rs`: snapshot stability, precedence, retention, staleness, and ordering tests.
- `src/storage.rs`: deterministic ownership layers and conflict handling.
- `src/tests.rs`: focused identity, storage, canonicalization, and property tests.

## Validation

Run `cargo test --tests -p umber-vfs` after changes. The crate must also compile
for `wasm32-unknown-unknown`.
