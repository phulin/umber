# tex-content Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the shared fixed-size content identity and compact immutable byte owner used across live state and detached output.

## File Map

- `AGENTS.md`: Crate-local identity guidance.
- `Cargo.toml`: Dependency-free crate manifest.
- `src/lib.rs`: Compact immutable byte ownership and the current versioned,
  domain-separated content identity implementation.

## Boundaries

- Keep this crate dependency-free and below both `tex-state` and `tex-out`.
- `SharedBytes` may adopt fresh vectors or existing shared slices without a
  payload copy, but exposes only immutable byte views and cheap owner clones.
- Never change an existing domain/version preimage. Introduce a new version explicitly.
- Artifact reads require the same current domain-separated identity as writes;
  historical undomained and version-1 hashes are unsupported.
