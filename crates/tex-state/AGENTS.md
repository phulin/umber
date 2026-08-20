# tex-state Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the live TeX state layer and the primary boundary between engine logic, durable snapshots, and host effects.

## Crate Role

`tex-state` owns `Universe`, the aggregate facade for live engine stores, and `World`, the controlled interface for files, streams, clocks, randomness, shell escape policy, and effect records. It stores meanings, registers, code tables, token lists, glue specs, nodes, boxes, fonts, hyphenation data, input summaries, grouping/journaling state, epochs, and snapshot/replay support.

All production mutation of live TeX state should pass through `Universe` or similarly aggregate facades. This crate also owns the barriered APIs that keep rollback, grouping, effect commit, and replay behavior coherent.

## File Map

- `AGENTS.md`: Local guidance for agents working in the `tex-state` crate.
- `Cargo.toml`: Crate manifest, dependencies, features, library target, and integration test wiring.
- `src/cell.rs`: Packed environment cell identifiers and bank tags shared by journals, raw storage, dependency tracking, and semantic hashing; assignment scope is stripped through the canonical `CellId` helper.
- `src/cell/tests.rs`: Unit tests for cell id packing, bank decoding, and global-bit handling.
- `src/code_tables.rs`: Storage-independent code-table value vocabulary; the
  live cat/lc/uc/sf/math/delcode banks and INITEX virtual defaults are owned by
  `DenseState`.
- `src/command_context.rs`: Already-admitted session/generation borrow for
  direct meaning, register, code-table, definition, token-list, and glue reads.
- `src/dependency.rs`: Region-scoped dependency keys with scope-free `CellId` environment identity, typed recorder lifecycle and first-reason poison barrier, detached observations, changed-at validation, conservative page/PDF family clocks, registered World-backed mutation keys, semantic backdating, and opaque cross-Universe memo validation stamps.
- `src/dependency/tests.rs`: Dependency mutation matrix, generic tracked-region lifecycle and journal-write records, deterministic ordering, rollback failure closure, and handle-independent observation tests.
- `src/diagnostic.rs`: tex.web §245's shared `begin_diagnostic`/`end_diagnostic` print channel, which every `\tracing*` parameter's text is routed through.
- `src/diagnostic/tests.rs`: Destination-selection, `print_nl` line-break, and scalar-formatting tests for the diagnostic channel.
- `src/definition_arena.rs`: Append-only generation-branded immutable macro
  definitions whose parameter and replacement words live in the same arena.
- `src/durable_arena.rs`: Separate generation-branded append-only token-list,
  glue, and provenance arenas with direct row resolution.
- `src/env.rs`: Generation-branded eqtb-equivalent current state, exact TeX
  local/global save semantics, group boundaries, and journal-cursor restore.
- `src/engine_state.rs`: Read-only execution mode and state projection consumed by expansion-time enquiries.
- `src/expansion_diagnostic.rs`: Detached recoverable expansion diagnostic
  values shared by command expansion and execution-side presentation.
- `src/expansion_recovery.rs`: Detached main-control recovery vocabulary that
  keeps execution independent of the command expansion error tree.
- `src/env/banks.rs`: Direct contiguous banks, page/index dense banks,
  dense-prefix/paged-overflow register banks, and typed parameter ids.
- `src/env/banks/tests.rs`: Direct-index, virtual-default, and paged-overflow
  bank tests.
- `src/env/group.rs`: Storage-independent group kinds, display frames, and
  mismatch values carried by the ordered journal.
- `src/env/tests.rs`: Meaning admission, dense/paged registers, INITEX code
  defaults, exact nested local/global restoration, and cursor rollback tests.
- `src/epoch.rs`: Monotonic epoch stamps used to coalesce journal entries within a state epoch.
- `src/epoch/tests.rs`: Unit tests for epoch ordering, raw values, and overflow behavior.
- `src/effect_journal.rs` and `src/effect_journal/tests.rs`: Validated detached effect-ledger ownership, aligned publication metadata, copy-only deferred-write token coordinates, prefix splicing, and terminal materialization.
- `src/etex_tracing.rs` and `src/etex_tracing/tests.rs`: e-TeX 2.6's `\tracinggroups` group-enter/leave transcript trace, printed through the shared `\tracing*` diagnostic channel; `\tracingassigns`'s value rendering lives in `tex-exec` instead, against the primitives declared here, and `\tracingifs` renders directly in `tex-command` through the same channel.
- `src/file_framing.rs` and `src/file_framing/tests.rs`: tex.web §54's `open_parens` and the §537/§362/§1335 prints that maintain it, held as print-adjacent `World` state so the command core can close a file's paren at §362's own point, ahead of the `check_outer_validity` diagnostic that follows it.
- `src/font.rs`: Stateful loaded-font store, font handles, null font, missing-character records, and rollback marks.
- `src/format_container.rs`: Portable schema-11 format-image header, section directory, compatibility fingerprints, checksum, and structural validation.
- `src/format_container/tests.rs`: Focused frozen-container header, directory, checksum-coverage, fingerprint, and geometry tests.
- `src/frozen_lookup.rs`: Versioned portable literal bucket/index codecs used to encode and validate cold format structures; decoded token lookup tables own no runtime liveness.
- `src/frozen_lookup/tests.rs`: Deterministic generation, lookup equivalence, and malformed literal-table validation tests.
- `src/glue.rs`: Storage-independent immutable TeX glue values.
- `src/hot_core/journal.rs`: Inline-small first-write inverse records, strictly
  nested marks, exact rollback, and parent-epoch transfer over typed mutable
  targets.
- `src/hot_core/mod.rs`: Private HotCore storage module boundary; command
  semantics remain outside this substrate.
- `src/hot_core/stack.rs`: Copy-only compact
  stacks with 32-bit marks, inline common storage, spill reuse, accounting, and
  bounded-cycle controls.
- `src/hot_core/state.rs`: Fixed-length
  inline-small dense mutable banks, typed namespace/generation coordinates,
  first-write journal integration, stale rejection, nested rollback, and
  plateau controls.
- `src/hyphenation.rs`: Hyphenation pattern trie and exception table implementing Liang-style position lookup.
- `src/hyphenation/tests.rs`: Unit tests for hyphenation patterns, exceptions, bounds, and overlapping matches.
- `src/identity.rs`: Shared generation-tagged runtime identity allocator for rollback-truncated stores.
- `src/generation.rs`: Fresh invariant generation brands, sealed arena
  construction, coarse generation ownership, and whole-generation retirement.
- `src/ids.rs`: Opaque snapshot and font handles retained outside the deleted runtime-value ownership substrate.
- `src/input.rs`: Snapshot-ready lexer/input stack summaries with copy-only token-list ids, macro replay sites and argument slots, source ids, and generic checkpoint future-state comparison.
- `src/input/tests.rs`: Value and snapshot-isolation tests for frozen input
  summaries and source payloads.
- `src/interner.rs`: Bounded append-only session epoch for control-sequence
  names and retained spellings, with explicit foreign-session admission and
  whole-epoch retirement.
- `src/interner/tests.rs`: Session budget, namespace, stability, admission,
  and retirement tests.
- `src/journal.rs`: Generation-branded ordered mutation/group history carrying
  exact prior packed values, TeX save levels, and owner-checked cursors.
- `src/journal/tests.rs`: Exact position/truncation and foreign-owner cursor
  rejection tests.
- `src/lib.rs`: Public module declarations and re-exports forming the `tex-state` API surface.
- `src/macro_definition.rs`: Storage-independent allocation-free macro parameter programs retained for the replacement definition arena.
- `src/math.rs`: Immutable math-list model for noads, fields, fractions, styles, choices, and math font families.
- `src/meaning.rs`: Static packed TeX meanings plus generation-typed macro
  meaning words; raw integers never materialize runtime definition ids.
- `src/meaning/tests.rs`: Static codec, primitive, and typed macro-meaning tests.
- `src/memo.rs`: Opaque schema-versioned detached memo envelopes, handle-free transition/effect/result DTOs, and aggregate token/glue/macro/node/font import APIs.
- `src/memo/tests.rs`: Cold/fork/rollback Cross-Universe memo import, provenance stripping, corruption, bounds, kind, and semantic round-trip tests.
- `profiling-allocator/`: isolated profiling-only `GlobalAlloc` forwarding
  shim used by executable profiling builds to attribute allocation calls and
  requested bytes to nested hot-core owner scopes.
- `src/node.rs`: Immutable TeX node, box, strongly rooted character/ligature provenance and glue, kern, penalty, rule, strongly token-rooted whatsit/mark/PDF payloads, math-list, discretionary, and list-field model.
- `src/node_sequence.rs`: Paired semantic and TeX-physical projections over
  the sole mutable `NodeListBuilder`, barrier-frozen immutable sidecars,
  TeX-cell lineage metadata, and semantic-only equality.
- `src/node_arena.rs`: Compact-node module boundary and deliberately narrow re-exports.
- `src/node_arena/builder.rs`: Sole mutable native-node builder shared by mode
  construction and packed episodes; freeze derives direct-child reachability
  before publishing one immutable graph.
- `src/node_arena/copy.rs`: Test-only compact-copy and child-patch machinery retained for node-storage measurement coverage; production freeze does not copy immutable child payloads.
- `src/node_arena/mutation.rs`: Test-only shape-preserving compact-row replacement support for compact-copy measurement coverage.
- `src/node_arena/schema.rs`: Exhaustive allocation-free logical node descriptors, typed handle policies, origins, and ordered child traversal.
- `src/node_arena/semantic.rs`: Versioned, allocation-independent semantic identity for immutable node-list aggregates.
- `src/node_arena/storage.rs`: Canonical node words, aligned provenance plus copy-only token/glue coordinate sidecars, and immutable payload encoding.
- `src/node_arena/tables.rs`: Typed structure-of-arrays sidecar tables for boxes, unsets, insertions, and noads.
- `src/node_arena/view.rs`: Zero-allocation node references, list spans, raw tag predicates, character runs, and iterators.
- `src/page.rs`: Snapshot-owned page-builder state with copy-only last-glue and scalar/class mark coordinates, page dimensions/integers, contribution/current-page queues, and fire-up records.
- `src/pdf.rs`: Checkpointed pdfTeX document mode with copy-only token coordinates in catalog/page/form collections, deterministic object allocation, snapshots, suffix transfer, and committed-page ledger.
- `src/pdf/action.rs`: Typed, checkpointed PDF action model carrying copy-only token coordinates shared by catalog, link, and outline scanners.
- `src/pdf/annotation.rs`: Checkpointed general-annotation reservations with copy-only token coordinates, running dimension specs, and logical/open-link records.
- `src/pdf/outline.rs`: Immediately allocated, checkpointed PDF outline entries owning their attributes, title, action, and action/item/title identities.
- `src/pdf/object.rs`: Copy-on-write raw PDF object reservations, coordinate-valued initialization payloads, and last-object state.
- `src/pdf/document.rs`: Copy-on-write coordinate-valued raw document dictionary and trailer fragments in source order.
- `src/page/sequence.rs`: Canonical persistent binary-forest sequence for growing current-page nodes.
- `src/page/state_hash.rs`: Page semantic cursors, bounded derived projection caches, and component framing.
- `src/page/tests.rs`: Page snapshot value isolation, mark-value, and semantic
  hash rollback tests.
- `src/print.rs`: tex.web §54's print `selector`, §§57--65's print primitives, §73's `print_err`, and §82's `error` report channel.
- `src/print/error_context.rs`: tex.web §§310--318's `show_context` two-line pseudoprint, bounded eager before/after projections captured at the live input seam, §314's token-list labels, and §310's `\errorcontextlines` elision, shared by every input-stack owner.
- `src/print/tests.rs`: Unit tests for context widths, selector routing, help routing, and error-report completion.
- `src/provenance.rs`: Storage-independent provenance demand, budget, source,
  invocation, insertion, synthesis, related-location, and origin-record values;
  live storage and ownership are deliberately absent pending the final arenas.
- `src/pure_memo.rs`: Optional entry/byte-bounded pure-query caches for pretolerance, page-breaking, and shipout results, bounded eviction telemetry, explicit cache release, and stable output-provenance recipes.
- `src/resource.rs`: Generic host-resource availability, absence, and stable
  suspension identities plus the state-owned immutable input-content resolver
  contract shared across engine layers.
- `src/read_observation.rs`: State-owned read-recorder contract, detached
  transactional batches, and deterministic dependency-set recorder.
- `src/pure_memo/tests.rs`: Collision, eviction, retention-release, and disabled-cache tests.
- `src/scaled.rs`: Compatibility re-export for shared TeX scaled-point arithmetic.
- `src/source_map.rs`: Rollback-coupled logical source regions, validated positions/spans, and immutable World/generated backing identities.
- `src/source_map/tests.rs`: Source-region anchors, validation, overflow, rollback/reuse, and O(1)-mark tests.
- `src/source_fragments.rs`: Session-scoped immutable source fragments, editor
  piece tables, demand-selected generation backing, rebound root registration,
  and layout-aware stable-recipe resolution.
- `src/source_fragments/layout_index.rs`: Fragment-and-offset index for logarithmic current/deleted piece resolution across repeated views.
- `src/source_fragments/tests.rs`: Fragment range, deletion, fork-liveness, anchor, allocator, snapshot, and line-index cache tests.
- `src/state_hash.rs`: Deterministic semantic state hasher used by snapshots and replay convergence checks.
- `src/stores.rs`: Coarse generation state owner, immutable/mutable admitted
  episode views, typed arena publication, and whole-generation retirement.
- `src/stores/tests.rs`: Direct arena resolution, generation-id bank
  installation, and coarse retirement tests.
- `src/tests.rs`: Crate-level integration-style unit tests for `Universe`, snapshots, world effects, and module test wiring.
- `src/tests/live_boundary.rs`: Unit tests proving live-state capability boundaries and restricted context APIs.
- `src/tests/replay.rs`: Unit tests for snapshot/replay behavior and semantic state convergence.
- `src/tests/replay_common.rs`: Shared helpers for replay tests, including model cells and expected hash state.
- `src/token.rs`: Token and catcode value definitions, constructors,
  classification helpers, and inline-small rooted traced-token buffers with
  sparse provenance ownership and spillover storage. Generated runs sharing
  one origin pack every word against its id and move that structural root into
  the sparse owner set once; they never clone it per token before deduplication.
- `src/token/tests.rs`: Unit tests for token constructors, catcodes, parameter tokens, and display/debug behavior.
- `src/token_show.rs`: tex.web §§49/262--294's printable token spellings -- `show_token_list`, `print_cs`, and `\string` rendering over the interner, catcodes, and `\escapechar`.
- `src/universe.rs`: Public session/generation aggregate, typed mutation and
  allocation facade, owner-checked journal cursors, admitted command views,
  and whole-session retirement.
- `src/universe/tests.rs`: Session/generation admission, rollback-independent
  interning, foreign-session rejection, and retirement tests.
- `src/world.rs`: External-effect boundary for files, atomic downstream
  file-set publication, streams, clocks, randomness, shell policy, printing,
  coordinate-valued deferred-write effects, artifact-owned rendered-source
  roots/recipes, weak snapshot-root mounts, and field/key-specific
  allocation-independent dependency projections.
- `src/world/tests.rs`: Unit tests for world snapshots, file records, streams,
  printing, randomness, shell escape, deferred-write values, and effect replay.
- `tests/it.rs`: Integration test harness that includes capability-boundary and live-boundary test modules.
- `tests/structural_node_lifecycle.rs`: Focused success, committed-failure, rollback, retry, rejection, checkpoint, and generation-fork controls for structural node-list ownership.
- `tests/it/capability_boundaries.rs`: Compile-fail integration tests asserting restricted input and transaction capabilities fail to compile.
- `tests/it/handle_serialization.rs`: Downstream compile-fail probe proving serde and private constructors cannot mint live handles or handle-bearing nodes.
- `tests/it/live_boundary.rs`: Downstream compile-fail assertion ensuring private stores and raw environment mutation stay inaccessible.
- `tests/ui/input_open_context_forbidden.rs`: Compile-fail fixture that attempts forbidden reads, world access, and mutations from `InputOpenContext`.
- `tests/ui/arena_transaction_exclusive.rs`: Compile-fail fixture proving suffix-owning transactions exclusively borrow the aggregate timeline.
- `tests/ui/*-boundary-forbidden.rs`: Independent compile-fail fixtures attempting to bypass private live-state stores, omit paired editor-layout validation, or bypass the `Universe` facade.
- `tests/ui/handle_serialization_forbidden.rs`: Compile-fail fixture attempting to serialize, deserialize, or construct live handles downstream.

## Runtime-storage Test Contract

The runtime-lifetime migration preserves observable value, rollback, format,
effect, source, and diagnostic behavior rather than the outgoing ownership
substrate. Standalone tests for opaque-id layouts, generation and region
coordinates, root-set admission, pointer or reference counts, physical store
growth, and handle lookup internals have been removed. Mixed subsystem tests
must compare semantic values, portable format sections, semantic hashes,
rollback results, and diagnostics instead of allocation identities or storage
topology. Test-only growth, root-traversal, cache-shape, and ownership-census
facades with no semantic consumers do not belong in this crate.

The deletion boundary includes fabricated raw ids; stale-slot and generation
number comparisons whose only subject is the old coordinate layout; exact
owner, root-set, and admission projections; fixed handle, mark, checkpoint, or
aggregate sizes; `Arc`/`Weak` counts and pointer identity; exact retained-byte
growth tied to old owners; and weak-candidate lookup or reclamation work. Do
not recreate those assertions while replacing the storage substrate. The
cross-crate retained-owner and deletion ledger is
[`../../docs/runtime_storage_contract_tests.md`](../../docs/runtime_storage_contract_tests.md).

## Boundaries

- Do not expose raw substores, raw checkpoint/restore hooks, raw word decoders, or opaque handle constructors outside crate-private or test-only APIs.
- Do not let downstream crates mutate state directly; keep the live-store boundary production-like, including under `shadow`.
- Do not place expansion or execution policy here when it belongs in `tex-command` or `tex-exec`; state should provide the substrate and invariants.
- Keep all host I/O and effectful facts behind `World`; engine crates should not reach for `std::fs`, clocks, random sources, or shell execution directly.
- Validate symbol-keyed or handle-keyed writes against the owning interner/store liveness before accepting them.

## Validation

Run `cargo test --tests -p tex-state` for state changes. For boundary-sensitive changes, include the live-boundary, replay, shadow, and compile-fail coverage that exercises the affected facade.
State performance benchmarks live in the standalone `benchmarks/tex-state` crate and are run explicitly with `cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets`.
