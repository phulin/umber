# Docs Guidance

Read the repository-level `AGENTS.md` before editing here. Documentation should describe the current fixture workflow: `cargo test --workspace --tests` is the correctness gate against committed fixtures, and `scripts/regen-fixtures.sh` is the only supported live-reference regeneration entry point.

When documenting tests or parity workflow, point fixture changes to `scripts/regen-fixtures.sh` modes rather than cargo-test environment variables or retired scripts.

`provenance_performance.md` records durable benchmark and memory observations for packed token provenance; update it when provenance hot-path behavior or benchmark workloads change.

`snapshot_performance.md` defines the focused snapshot latency and retained-allocation gate, including its asymptotic budgets and measurement semantics.

`profiling.md` documents the persistent in-process Gentle profiler, its
Samply wrapper, prerequisites, and measured boundary.

`testing_policy.md` is forward-looking guidance for test design and placement.
`testing_infrastructure.md` inventories the current test commands, budgets,
fixtures, corpora, and harnesses; update it when those implementation facts
change.

`incremental_v1.md` fixes the named-boundary schedule, editor-session
retention, edit mapping, pruning, and schedule-relative convergence contract
for the first incremental engine.

`persistent_compile_sessions.md` defines the unified native/WASM compile
session lifecycle that composes typed resource retries with revision-checked
root-buffer patches and retained incremental execution.

`incremental_memoization.md` supersedes folded-hash convergence as the general
incremental strategy and specifies stable input identity, constrained read
validation, semantic mutation/effect replay, layered memoization boundaries,
hierarchical trace reuse, retention, measurement, and rollout.

`retained_group_roots.md` specifies the proposed persistent/COW environment
history needed for durable paragraph checkpoints inside ordinary open groups,
including store ownership, reclamation, hashing, rollout, and validation.

`source_spans_and_provenance.md` specifies the compact source-map, source-span, and derived-provenance design plus its phased migration plan.

`node_word_arena.md` is the authoritative compact node-word arena document: it
combines measurements with the representation, sidecar ownership, migration,
validation, and conditional adoption design. Do not create a separate
`node_word_layout.md` whose encoding or rollback rules could drift.

`wasm_resource_acquisition.md` specifies the implemented typed, batched
resource state machine and the remaining OpenType rollout, including
required-versus-hint semantics, client-owned distribution, font reuse,
caching, and native parity.

`web_font_bundles.md` specifies the OpenType-first native/WASM font-resource
model: OTF/TTF native containers, WOFF2 browser containers, canonical program
identity, batched acquisition, client-owned distribution, retained HTML asset
reuse, and the migration rollout.

`etex_primitives.md` is the extension-only e-TeX V2 primitive checklist and
maps each family to its short-reference-manual contract and conformance gate.

`latex_dvi.md` defines the supported LaTeX-DVI engine identity, extension
inventory, pinned distribution boundary, parity contract, and explicit
non-goals such as pdfLaTeX identity and PDF output.
