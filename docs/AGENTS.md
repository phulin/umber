# Docs Guidance

Read the repository-level `AGENTS.md` before editing here. Documentation should describe the current fixture workflow: `cargo test --workspace --tests` is the correctness gate against committed fixtures, and `scripts/regen-fixtures.sh` is the only supported live-reference regeneration entry point.

When documenting tests or parity workflow, point fixture changes to `scripts/regen-fixtures.sh` modes rather than cargo-test environment variables or retired scripts.

`provenance_performance.md` records durable benchmark and memory observations for packed token provenance; update it when provenance hot-path behavior or benchmark workloads change.

`snapshot_performance.md` defines the focused snapshot latency and retained-allocation gate, including its asymptotic budgets and measurement semantics.

`testing_policy.md` is forward-looking guidance for test design and placement.
`testing_infrastructure.md` inventories the current test commands, budgets,
fixtures, corpora, and harnesses; update it when those implementation facts
change.

`retained_group_roots.md` specifies the proposed persistent/COW environment
history needed for durable paragraph checkpoints inside ordinary open groups,
including store ownership, reclamation, hashing, rollout, and validation.

`source_spans_and_provenance.md` specifies the compact source-map, source-span, and derived-provenance design plus its phased migration plan.

`math_layout_arena.md` specifies the contiguous, span-backed Appendix G math
conversion result and its pure lowering boundary.

`node_word_arena.md` is the authoritative compact node-word arena document: it
combines measurements with the representation, sidecar ownership, migration,
validation, and conditional adoption design. Do not create a separate
`node_word_layout.md` whose encoding or rollback rules could drift.

`wasm_mvp.md` specifies the proposed browser WebAssembly package, including
the restart-on-fetch driver protocol, hosted TeX Live manifest, cache and
resource limits, binding API, and MVP verification boundary.

`etex_primitives.md` is the extension-only e-TeX V2 primitive checklist and
maps each family to its short-reference-manual contract and conformance gate.
