# tex-state Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the live TeX state layer and the primary boundary between engine logic, durable snapshots, and host effects.

## Crate Role

`tex-state` owns `Universe`, the aggregate facade for live engine stores, and `World`, the controlled interface for files, streams, clocks, randomness, shell escape policy, and effect records. It stores meanings, registers, code tables, token lists, glue specs, nodes, boxes, fonts, hyphenation data, input summaries, grouping/journaling state, epochs, and snapshot/replay support.

All production mutation of live TeX state should pass through `Universe` or similarly aggregate facades. This crate also owns the barriered APIs that keep rollback, grouping, effect commit, and replay behavior coherent.

## File Map

- `AGENTS.md`: Local guidance for agents working in the `tex-state` crate.
- `Cargo.toml`: Crate manifest, dependencies, features, library target, and integration test wiring.
- `src/cell.rs`: Packed environment cell identifiers and bank tags used by journals and raw storage.
- `src/cell/tests.rs`: Unit tests for cell id packing, bank decoding, and global-bit handling.
- `src/code_tables.rs`: Sparse persistent-radix TeX catcode, lc/uc/sf/math/delcode tables with virtual canonical defaults, generation stamps, groups, and snapshots.
- `src/code_tables/global.rs`: Persistent global-assignment delta history used to rebase saved group roots without depth-sensitive writes.
- `src/code_tables/tests.rs`: Unit tests for code-table defaults, writes, sparse pages, generations, and snapshots.
- `src/env.rs`: Barriered mutable environment storage for meanings, registers, parameters, font values, grouping, and journals.
- `src/env/banks.rs`: Dense fixed-size bank codecs, parameter ids, and typed bank access helpers.
- `src/env/group.rs`: Group stack, aftergroup/afterassignment handling, group mismatch types, and environment snapshot logic.
- `src/env/overflow.rs`: Sparse e-TeX overflow register banks for high register numbers.
- `src/env/raw.rs`: Restore-only raw environment writes, semantic word iteration, shadow verification, and raw word helpers.
- `src/env/tests.rs`: Unit tests for environment write barriers, grouping, globals, aftergroup, font banks, and raw restore behavior.
- `src/epoch.rs`: Monotonic epoch stamps used to coalesce journal entries within a state epoch.
- `src/epoch/tests.rs`: Unit tests for epoch ordering, raw values, and overflow behavior.
- `src/font.rs`: Stateful loaded-font store, font handles, null font, missing-character records, and rollback marks.
- `src/glue.rs`: Immutable hash-consed glue-spec storage and rollback watermarks.
- `src/glue/tests.rs`: Unit tests for glue interning, canonical zero glue, rollback, and hash-index rebuilds.
- `src/hyphenation.rs`: Hyphenation pattern trie and exception table implementing Liang-style position lookup.
- `src/hyphenation/tests.rs`: Unit tests for hyphenation patterns, exceptions, bounds, and overlapping matches.
- `src/identity.rs`: Shared generation-tagged runtime identity allocator for rollback-truncated stores.
- `src/identity/tests.rs`: Property and boundary tests for rollback, fork, exhaustion, and foreign-handle rejection.
- `src/ids.rs`: Opaque ids for token lists, origin lists, macros, glue, fonts, snapshots, survivor roots, and node-list spans.
- `src/ids/tests.rs`: Unit tests for opaque id raw values and node/origin-list span metadata.
- `src/input.rs`: Snapshot-ready lexer/input stack summaries, macro argument slots, source ids, and replay frame metadata.
- `src/interner.rs`: Control-sequence name interner with dense symbols, lookup, hashing, and rollback marks.
- `src/interner/tests.rs`: Unit tests for symbol interning, resolution, rollback, and content hashing.
- `src/journal.rs`: Append-only journal records, markers, undo entries, and rollback/group replay support.
- `src/journal/tests.rs`: Unit tests for journal positions, markers, entry traversal, and truncation.
- `src/lib.rs`: Public module declarations and re-exports forming the `tex-state` API surface.
- `src/macro_store.rs`: Immutable macro-definition store and macro meaning metadata.
- `src/math.rs`: Immutable math-list model for noads, fields, fractions, styles, choices, and math font families.
- `src/meaning.rs`: TeX meaning representation, primitive enums, flags, and packed raw meaning encode/decode logic.
- `src/meaning/tests.rs`: Unit tests for meaning round trips, flag packing, and primitive encoding.
- `src/measurement.rs`: Feature-gated process-local allocation-owner counters used by profiling builds.
- `src/node.rs`: Immutable TeX node, box, glue, kern, penalty, rule, whatsit, math-list, discretionary, and list-field model.
- `src/node_arena.rs`: Compact-node module boundary and deliberately narrow re-exports.
- `src/node_arena/arena.rs`: Epoch arena facade and reusable owned node-list builder.
- `src/node_arena/copy.rs`: Private compact-to-compact span copying and typed child-patch descriptions.
- `src/node_arena/measurement.rs`: Feature-gated compact-column and peak-storage accounting.
- `src/node_arena/mutation.rs`: Private shape-preserving compact-row replacement operations.
- `src/node_arena/storage.rs`: Canonical node words, sidecar coordination, encoding, aggregate watermarks, and rollback.
- `src/node_arena/tables.rs`: Typed structure-of-arrays sidecar tables for boxes, unsets, insertions, and noads.
- `src/node_arena/view.rs`: Zero-allocation node references, list spans, character runs, and iterators.
- `src/node_arena/tests.rs`: Unit tests for node-list allocation, lookup, rollback, and arena liveness.
- `src/page.rs`: Snapshot-owned page-builder state, page dimensions/integers, contribution/current-page queues, and fire-up records.
- `src/provenance.rs`: Diagnostic origin-record and origin-list arenas with rollback watermarks.
- `src/provenance/tests.rs`: Unit tests for provenance allocation, readback, and rollback marks.
- `src/scaled.rs`: Compatibility re-export for shared TeX scaled-point arithmetic.
- `src/source_map.rs`: Rollback-coupled logical source regions, validated positions/spans, and immutable World/generated backing identities.
- `src/source_map/tests.rs`: Source-region anchors, validation, overflow, rollback/reuse, and O(1)-mark tests.
- `src/state_hash.rs`: Deterministic semantic state hasher used by snapshots and replay convergence checks.
- `src/stores.rs`: Internal aggregate store tuple that coordinates interner, env, token, provenance, glue, node, font, survivor, input, and rollback state.
- `src/stores/handles.rs`: Store-boundary liveness checks for symbols, token lists, origins, glue, fonts, macros, and node handles.
- `src/stores/node_clone.rs`: Iterative child-first cloning of mixed node graphs into epoch storage.
- `src/stores/format.rs`: Deterministic versioned format-image DTO capture/validation and fresh-store reconstruction.
- `src/stores/state_hash.rs`: Store snapshot cursor and semantic hashing implementation for changed cells and store-owned slices.
- `src/stores/tests.rs`: Unit tests for aggregate store rollback, builders, handle validation, parameters, boxes, and state hashes.
- `src/survivor.rs`: Survivor arena for node lists that escape epoch rollback boundaries, including root-safe buffer recycling and feature-gated promotion measurements.
- `src/tests.rs`: Crate-level integration-style unit tests for `Universe`, snapshots, world effects, and module test wiring.
- `src/tests/live_boundary.rs`: Unit tests proving live-state capability boundaries and restricted context APIs.
- `src/tests/replay.rs`: Unit tests for snapshot/replay behavior and semantic state convergence.
- `src/tests/replay_common.rs`: Shared helpers for replay tests, including model cells and expected hash state.
- `src/token.rs`: Token and catcode value definitions, constructors, and classification helpers.
- `src/token/tests.rs`: Unit tests for token constructors, catcodes, parameter tokens, and display/debug behavior.
- `src/token_store.rs`: Immutable hash-consed token-list storage, builders, lookup, and rollback marks.
- `src/token_store/tests.rs`: Unit tests for token-list interning, builder reuse, lookup, and rollback.
- `src/universe.rs`: Top-level TeX state timeline, snapshots, effect commits, and capability-specific context facades.
- `src/universe/tests.rs`: Unit tests for `Universe` mutation, snapshots, contexts, effects, and boundary behavior.
- `src/world.rs`: External-effect boundary for files, streams, clocks, randomness, shell policy, printing, and effect records.
- `src/world/tests.rs`: Unit tests for world snapshots, file records, streams, printing, randomness, shell escape, and effect replay.
- `tests/it.rs`: Integration test harness that includes capability-boundary and live-boundary test modules.
- `tests/it/capability_boundaries.rs`: Compile-fail integration tests asserting restricted expansion and input capabilities fail to compile.
- `tests/it/live_boundary.rs`: Downstream probe tests ensuring private stores and raw environment mutation stay inaccessible.
- `tests/ui/expansion_context_forbidden.rs`: Compile-fail fixture that attempts forbidden privileged calls from `ExpansionContext`.
- `tests/ui/expansion_state_input_forbidden.rs`: Compile-fail fixture that attempts input opening through generic `ExpansionState`.
- `tests/ui/input_open_context_forbidden.rs`: Compile-fail fixture that attempts forbidden reads, world access, and mutations from `InputOpenContext`.

## Boundaries

- Do not expose raw substores, raw checkpoint/restore hooks, raw word decoders, or opaque handle constructors outside crate-private or test-only APIs.
- Do not let downstream crates mutate state directly; keep the live-store boundary production-like, including under `shadow`.
- Do not place expansion or execution policy here when it belongs in `tex-expand` or `tex-exec`; state should provide the substrate and invariants.
- Keep all host I/O and effectful facts behind `World`; engine crates should not reach for `std::fs`, clocks, random sources, or shell execution directly.
- Validate symbol-keyed or handle-keyed writes against the owning interner/store liveness before accepting them.

## Validation

Run `cargo test --tests -p tex-state` for state changes. For boundary-sensitive changes, include the live-boundary, replay, shadow, and compile-fail coverage that exercises the affected facade.
State performance benchmarks live in the standalone `benchmarks/tex-state` crate and are run explicitly with `cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets`.
