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
- `src/dependency.rs`: Region-scoped dependency keys, detached observations, changed-at validation, semantic backdating, and opaque cross-Universe memo validation stamps.
- `src/dependency/tests.rs`: Dependency mutation matrix, deterministic nested propagation, and handle-independent observation tests.
- `src/env.rs`: Barriered mutable environment storage for meanings, registers, parameters, font values, grouping, and journals.
- `src/env/banks.rs`: Dense fixed-size bank codecs, parameter ids, and typed bank access helpers.
- `src/env/box_bank.rs`: Dense-and-paged box slots combining semantic values with journal-owned assignment and coalescing state.
- `src/env/group.rs`: Group stack, aftergroup/afterassignment handling, group mismatch types, and environment snapshot logic.
- `src/env/overflow.rs`: Sparse e-TeX overflow register banks for high register numbers.
- `src/env/paragraph.rs`: Lazy count/int paragraph fingerprints and journal-derived root survivor redo extraction.
- `src/env/raw.rs`: Restore-only raw environment writes, semantic word iteration, shadow verification, and raw word helpers.
- `src/env/tests.rs`: Unit tests for environment write barriers, grouping, globals, aftergroup, font banks, and raw restore behavior.
- `src/epoch.rs`: Monotonic epoch stamps used to coalesce journal entries within a state epoch.
- `src/epoch/tests.rs`: Unit tests for epoch ordering, raw values, and overflow behavior.
- `src/font.rs`: Stateful loaded-font store, font handles, null font, missing-character records, and rollback marks.
- `src/format_container.rs`: Portable schema-10 format-image header, section directory, compatibility fingerprints, checksum, and structural validation.
- `src/format_container/tests.rs`: Focused frozen-container header, directory, checksum-coverage, fingerprint, and geometry tests.
- `src/frozen_lookup.rs`: Versioned portable literal bucket/index codec and immutable runtime lookup for format-backed store prefixes.
- `src/frozen_lookup/tests.rs`: Deterministic generation, lookup equivalence, and malformed literal-table validation tests.
- `src/glue.rs`: Immutable hash-consed glue-spec storage and rollback watermarks.
- `src/glue/tests.rs`: Unit tests for glue interning, canonical zero glue, rollback, and hash-index rebuilds.
- `src/hyphenation.rs`: Hyphenation pattern trie and exception table implementing Liang-style position lookup.
- `src/hyphenation/tests.rs`: Unit tests for hyphenation patterns, exceptions, bounds, and overlapping matches.
- `src/identity.rs`: Shared generation-tagged runtime identity allocator for rollback-truncated stores.
- `src/identity/tests.rs`: Property and boundary tests for rollback, fork, exhaustion, and foreign-handle rejection.
- `src/ids.rs`: Opaque ids for token lists, origin lists, macros, glue, fonts, snapshots, survivor roots, and node-list spans.
- `src/ids/tests.rs`: Unit tests for opaque id raw values and node/origin-list span metadata.
- `src/input.rs`: Snapshot-ready lexer/input stack summaries, macro argument slots, source ids, and replay frame metadata.
- `src/input/tests.rs`: Structural-sharing tests for frozen input-summary roots and source payloads.
- `src/interner.rs`: Control-sequence name interner with dense symbols, lookup, hashing, and rollback marks.
- `src/interner/tests.rs`: Unit tests for symbol interning, resolution, rollback, and content hashing.
- `src/journal.rs`: Append-only journal records, markers, undo entries, and rollback/group replay support.
- `src/journal/tests.rs`: Unit tests for journal positions, markers, entry traversal, and truncation.
- `src/lib.rs`: Public module declarations and re-exports forming the `tex-state` API surface.
- `src/macro_store.rs`: Immutable macro-definition store and macro meaning metadata.
- `src/math.rs`: Immutable math-list model for noads, fields, fractions, styles, choices, and math font families.
- `src/meaning.rs`: TeX meaning representation, primitive enums, flags, and packed raw meaning encode/decode logic.
- `src/meaning/tests.rs`: Unit tests for meaning round trips, flag packing, and primitive encoding.
- `src/memo.rs`: Opaque schema-versioned detached memo envelopes, handle-free transition/effect/result DTOs, and aggregate token/glue/macro/node/font import APIs.
- `src/memo/tests.rs`: Cold/fork/rollback Cross-Universe memo import, provenance stripping, corruption, bounds, kind, and semantic round-trip tests.
- `src/measurement.rs`: `profiling-stats` process-local allocation-owner counters used by dedicated profiling builds.
- `src/node.rs`: Immutable TeX node, box, glue, kern, penalty, rule, whatsit, math-list, discretionary, and list-field model.
- `src/node_arena.rs`: Compact-node module boundary and deliberately narrow re-exports.
- `src/node_arena/arena.rs`: Epoch arena facade and reusable owned node-list builder.
- `src/node_arena/copy.rs`: Private compact-to-compact span copying and typed child-patch descriptions.
- `src/node_arena/measurement.rs`: `profiling-stats` compact-column and peak-storage accounting.
- `src/node_arena/measurement/tests.rs`: Coherence, divergent-maximum, nested-payload, and concurrent peak-measurement tests.
- `src/node_arena/mutation.rs`: Private shape-preserving compact-row replacement operations.
- `src/node_arena/semantic.rs`: Versioned, allocation-independent semantic identity for immutable node-list aggregates.
- `src/node_arena/storage.rs`: Canonical node words, sidecar coordination, encoding, aggregate watermarks, and rollback.
- `src/node_arena/tables.rs`: Typed structure-of-arrays sidecar tables for boxes, unsets, insertions, and noads.
- `src/node_arena/view.rs`: Zero-allocation node references, list spans, mount-local output-provenance overlays, raw tag predicates, character runs, and iterators.
- `src/node_arena/tests.rs`: Unit tests for node-list allocation, lookup, rollback, and arena liveness.
- `src/page.rs`: Snapshot-owned page-builder state, page dimensions/integers, contribution/current-page queues, and fire-up records.
- `src/pdf.rs`: Checkpointed pdfTeX document mode, deterministic object allocation, and committed-page ledger.
- `src/pdf/action.rs`: Typed, checkpointed PDF action model shared by catalog, link, and outline scanners.
- `src/pdf/annotation.rs`: Checkpointed general-annotation reservations, running dimension specs, and logical-link records.
- `src/pdf/outline.rs`: Immediately allocated, checkpointed PDF outline entries and their action/item/title identities.
- `src/pdf/object.rs`: Copy-on-write raw PDF object reservations, initialization payloads, and last-object state.
- `src/pdf/document.rs`: Copy-on-write raw document dictionary and trailer fragments in source order.
- `src/page/sequence.rs`: Canonical persistent binary-forest sequence for growing current-page nodes.
- `src/page/state_hash.rs`: Page semantic cursors, bounded derived projection caches, and component framing.
- `src/page/tests.rs`: Page snapshot-root sharing and copy-on-write isolation tests.
- `src/provenance.rs`: Diagnostic origin-record and origin-list arenas with rollback watermarks.
- `src/provenance/tests.rs`: Unit tests for provenance allocation, readback, and rollback marks.
- `src/pure_memo.rs`: Optional bounded session-local pure-query experiments plus the ordered accepted paragraph history, stable-start cursor, validation telemetry, compact output-provenance recipes, and retained result metadata anchored in accepted-generation node roots.
- `src/pure_memo/tests.rs`: Collision, eviction, retention-release, and disabled-cache tests.
- `src/scaled.rs`: Compatibility re-export for shared TeX scaled-point arithmetic.
- `src/source_map.rs`: Rollback-coupled logical source regions, validated positions/spans, and immutable World/generated backing identities.
- `src/source_map/tests.rs`: Source-region anchors, validation, overflow, rollback/reuse, and O(1)-mark tests.
- `src/source_fragments.rs`: Session-scoped immutable source fragments, editor piece tables, and layout-aware coordinate resolution.
- `src/source_fragments/layout_index.rs`: Fragment-and-offset index for logarithmic current/deleted piece resolution across repeated views.
- `src/source_fragments/tests.rs`: Fragment range, deletion, fork-liveness, anchor, allocator, snapshot, and line-index cache tests.
- `src/state_hash.rs`: Deterministic semantic state hasher used by snapshots and replay convergence checks.
- `src/stores.rs`: Internal aggregate store tuple that coordinates interner, env, token, provenance, glue, node, font, accepted-generation paragraph roots/auxiliaries, survivor pins, input, and rollback/shipout scope state.
- `src/stores/handles.rs`: Store-boundary liveness checks for symbols, token lists, origins, glue, fonts, macros, and node handles.
- `src/stores/exact_identity.rs`: Persistent deterministic Merkle treap for canonical environment-cell identities retained by checkpoints.
- `src/stores/exact_collection.rs`: Persistent deterministic Merkle collection for allocation-order-independent immutable-store roots.
- `src/stores/node_semantic.rs`: Canonical node encoding and bottom-up semantic-identity composition at aggregate freeze.
- `src/stores/format.rs`: Deterministic versioned format-image DTO capture/validation and fresh-store reconstruction.
- `src/stores/format/frozen_core.rs`: Fixed-width schema-10 names, token-list, macro, and glue section codecs plus direct validated dense-store restoration.
- `src/stores/format/frozen_non_node.rs`: Schema-10 font, code-table, and hyphenation section codecs plus direct validated store restoration.
- `src/stores/format/frozen_node.rs`: Schema-10 fixed-record reachable node-graph codec, semantic-identity validation, and frozen arena installation metadata.
- `src/stores/format/frozen_env.rs`: Schema-10 fixed-record environment-cell codec and validated immutable-base installation input.
- `src/stores/format/node.rs`: Handle-free serialized node/math DTO graph and validated conversion to and from live nodes.
- `src/stores/format/tests.rs`: Malformed format DTO validation tests that reject references before live-store publication.
- `src/stores/format/font_validation.rs`: Pre-publication validation of detached font metrics, identifiers, and serialized Env font banks, plus test-only corruption fixtures.
- `src/stores/state_hash.rs`: Store snapshot cursor and semantic hashing implementation for changed cells and store-owned slices.
- `src/stores/tests.rs`: Unit tests for aggregate store rollback, builders, handle validation, parameters, boxes, and state hashes.
- `src/survivor.rs`: Survivor arena for node lists that escape epoch rollback boundaries, including root-safe buffer recycling and profiling-only promotion measurements.
- `src/tests.rs`: Crate-level integration-style unit tests for `Universe`, snapshots, world effects, and module test wiring.
- `src/tests/handle_matrix.rs`: Table-driven aggregate rollback, fork, and cross-Universe liveness coverage for every production opaque handle class.
- `src/tests/live_boundary.rs`: Unit tests proving live-state capability boundaries and restricted context APIs.
- `src/tests/replay.rs`: Unit tests for snapshot/replay behavior and semantic state convergence.
- `src/tests/replay_common.rs`: Shared helpers for replay tests, including model cells and expected hash state.
- `src/token.rs`: Token and catcode value definitions, constructors, and classification helpers.
- `src/token/tests.rs`: Unit tests for token constructors, catcodes, parameter tokens, and display/debug behavior.
- `src/token_store.rs`: Immutable hash-consed token-list storage, builders, lookup, and rollback marks.
- `src/token_store/tests.rs`: Unit tests for token-list interning, builder reuse, lookup, and rollback.
- `src/universe.rs`: Top-level TeX state timeline, snapshots, effect commits, and capability-specific context facades.
- `src/universe/tests.rs`: Unit tests for `Universe` mutation, snapshots, contexts, effects, and boundary behavior.
- `src/world.rs`: External-effect boundary for files, atomic downstream file-set publication, streams, clocks, randomness, shell policy, printing, and effect records.
- `src/world/tests.rs`: Unit tests for world snapshots, file records, streams, printing, randomness, shell escape, and effect replay.
- `tests/it.rs`: Integration test harness that includes capability-boundary and live-boundary test modules.
- `tests/it/capability_boundaries.rs`: Compile-fail integration tests asserting restricted expansion and input capabilities fail to compile.
- `tests/it/handle_serialization.rs`: Downstream compile-fail probe proving serde and private constructors cannot mint live handles or handle-bearing nodes.
- `tests/it/live_boundary.rs`: Downstream compile-fail assertion ensuring private stores and raw environment mutation stay inaccessible.
- `tests/ui/expansion_context_forbidden.rs`: Compile-fail fixture that attempts forbidden privileged calls from `ExpansionContext`.
- `tests/ui/expansion_state_input_forbidden.rs`: Compile-fail fixture that attempts input opening through generic `ExpansionState`.
- `tests/ui/input_open_context_forbidden.rs`: Compile-fail fixture that attempts forbidden reads, world access, and mutations from `InputOpenContext`.
- `tests/ui/arena_transaction_exclusive.rs`: Compile-fail fixture proving suffix-owning transactions exclusively borrow the aggregate timeline.
- `tests/ui/*-boundary-forbidden.rs`: Independent compile-fail fixtures attempting to bypass private live-state stores, omit paired editor-layout validation, or bypass the `Universe` facade.
- `tests/ui/handle_serialization_forbidden.rs`: Compile-fail fixture attempting to serialize, deserialize, or construct live handles downstream.

## Boundaries

- Do not expose raw substores, raw checkpoint/restore hooks, raw word decoders, or opaque handle constructors outside crate-private or test-only APIs.
- Do not let downstream crates mutate state directly; keep the live-store boundary production-like, including under `shadow`.
- Do not place expansion or execution policy here when it belongs in `tex-expand` or `tex-exec`; state should provide the substrate and invariants.
- Keep all host I/O and effectful facts behind `World`; engine crates should not reach for `std::fs`, clocks, random sources, or shell execution directly.
- Validate symbol-keyed or handle-keyed writes against the owning interner/store liveness before accepting them.

## Validation

Run `cargo test --tests -p tex-state` for state changes. For boundary-sensitive changes, include the live-boundary, replay, shadow, and compile-fail coverage that exercises the affected facade.
State performance benchmarks live in the standalone `benchmarks/tex-state` crate and are run explicitly with `cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets`.
