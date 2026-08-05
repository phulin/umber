# tex-oracle Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
portable semantic-event contract shared by canonical TeX82, e-TeX, pdfTeX, and
Umber observers.

## Boundaries

- Keep the schema independent of every engine's semantic storage.
- Events contain owned canonical values, never allocation, pointer, pool,
  selector, transcript, input-stack, or host-path identities.
- Never change an existing schema preimage. Introduce a new schema version.
- Instrumentation transports are separate from ordinary engine output.

## File Map

- `src/schema.rs`: versioned engine, token, event, and manifest values.
- `src/fixture.rs`: committed fixture-contract validation and hermetic loading.
- `src/fixture_audit.rs`: bidirectional semantic-matrix, citation, source, and
  ordinary-output coverage audit.
- `src/encoding.rs`: canonical JSON encoding and domain-separated identities.
- `src/event.rs`: exhaustive borrowed/mutable event views and schema-owned
  normalization, location, identity-key, classification, and rendering walks.
- `src/normalize.rs`: deterministic stream normalization.
- `src/transport.rs`: enabled and compile-away disabled observer boundaries.
- `src/bundle.rs`: canonical detached semantic/geometry bundle, byte codec,
  pinned-header JSONL binding, validation, and resource limits.
- `src/tests.rs`: focused synthetic contract tests.
- `src/bin/tex-oracle-validate.rs`: canonical JSON Lines validator used by
  live reference regeneration tooling.
