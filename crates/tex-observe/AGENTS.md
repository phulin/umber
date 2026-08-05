# tex-observe Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
thin, host-neutral projection from `tex-command` observation records and
`tex-state` source provenance into `tex-oracle` values. `tex-oracle` owns every
persistent evidence transport.

## Boundaries

- Depend only on lower command/state/oracle libraries, never `umber` or host
  comparison tools.
- Keep engine IDs and raw `CommandObservation` values out of finalized detached
  evidence.
- Preserve command-v1 and geometry-v2 normalization byte-for-byte.

## File Map

- `src/lib.rs`: source/session projection into oracle-owned bundles; source
  identity is resolved directly from engine records rather than a shadow
  input stack.
- `src/translation.rs`: exact observation-to-oracle event mappings.
- `src/tests.rs`: focused internal translation tests.
- `tests/live_session.rs`: public live-session and extraction-equivalence tests.
