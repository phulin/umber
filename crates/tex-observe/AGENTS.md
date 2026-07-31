# tex-observe Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
detached, host-neutral translation from `tex-command` observation records and
`tex-state` source provenance into `tex-oracle` values.

## Boundaries

- Depend only on lower command/state/oracle libraries, never `umber` or host
  comparison tools.
- Keep engine IDs and raw `CommandObservation` values out of finalized detached
  evidence.
- Preserve command-v1 and geometry-v2 normalization byte-for-byte.

## File Map

- `src/lib.rs`: source-stack/session translation and public detached APIs.
- `src/translation.rs`: exact observation-to-oracle event mappings.
- `src/tests.rs`: focused internal translation tests.
- `tests/live_session.rs`: public live-session and extraction-equivalence tests.
