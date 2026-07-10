# Docs Guidance

Read the repository-level `AGENTS.md` before editing here. Documentation should describe the current fixture workflow: `cargo test --workspace --tests` is the correctness gate against committed fixtures, and `scripts/regen-fixtures.sh` is the only supported live-reference regeneration entry point.

When documenting tests or parity workflow, point fixture changes to `scripts/regen-fixtures.sh` modes rather than cargo-test environment variables or retired scripts.

`provenance_performance.md` records durable benchmark and memory observations for packed token provenance; update it when provenance hot-path behavior or benchmark workloads change.

`source_spans_and_provenance.md` specifies the compact source-map, source-span, and derived-provenance design plus its phased migration plan.

`math_layout_arena.md` specifies the contiguous, span-backed Appendix G math
conversion result and its pure lowering boundary.

`node_word_arena.md` is the authoritative compact node-word arena document: it
combines measurements with the representation, sidecar ownership, migration,
validation, and conditional adoption design. Do not create a separate
`node_word_layout.md` whose encoding or rollback rules could drift.
