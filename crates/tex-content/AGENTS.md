# tex-content Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the shared fixed-size content identity and compact immutable byte owner used across live state and detached output.

## File Map

- `AGENTS.md`: Crate-local identity and compatibility guidance.
- `Cargo.toml`: Dependency-free crate manifest.
- `src/lib.rs`: Compact immutable byte ownership, versioned domain-separated
  content identity implementation, and explicit legacy compatibility policy.

## Boundaries

- Keep this crate dependency-free and below both `tex-state` and `tex-out`.
- `SharedBytes` may adopt fresh vectors or existing shared slices without a
  payload copy, but exposes only immutable byte views and cheap owner clones.
- Never change an existing domain/version preimage. Introduce a new version explicitly.
- Legacy hashing exists only for compatible reads of previously committed objects; new domain-aware writes must not use it.
