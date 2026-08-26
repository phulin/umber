# umber-hash Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
portable persisted aHash64 algorithm and its domain registry.

## Boundaries

- Keep this crate dependency-free and compatible with `wasm32-unknown-unknown`.
- Never change seeds, framing, byte order, or an existing domain/version.
- Add a new algorithm version or domain for incompatible identity changes.
- This is a deterministic corruption/identity check, not cryptographic authentication.
