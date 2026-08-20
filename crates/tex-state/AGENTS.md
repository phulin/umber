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
- `src/code_tables.rs`: Sparse persistent-radix TeX catcode, lc/uc/sf/math/delcode tables whose virtual defaults are INITEX's initial values (tex.web §232 and §240, never a format's), plus generation stamps, groups, and snapshots.
- `src/code_tables/global.rs`: Persistent global-assignment delta history used to rebase saved group roots without depth-sensitive writes.
- `src/code_tables/tests.rs`: Unit tests for code-table defaults, writes, sparse pages, generations, and snapshots.
- `src/command_context.rs`: Interpretation-neutral aggregate access boundary
  reserved for the command processor. Exposes `begin_diagnostic`
  (e-TeX `\tracingifs`), `printer` (e-TeX `\tracingnesting`'s
  not-`stat`-gated `file_warning`), and `group_frames_from` (the same
  per-group display `\showgroups` uses) so `tex-command` can render those
  without a queued cross-crate diagnostic.
- `src/dependency.rs`: Region-scoped dependency keys with scope-free `CellId` environment identity, typed recorder lifecycle and first-reason poison barrier, detached observations, changed-at validation, conservative page/PDF family clocks, registered World-backed mutation keys, semantic backdating, and opaque cross-Universe memo validation stamps.
- `src/dependency/tests.rs`: Dependency mutation matrix, generic tracked-region lifecycle and journal-write records, deterministic ordering, rollback failure closure, and handle-independent observation tests.
- `src/diagnostic.rs`: tex.web §245's shared `begin_diagnostic`/`end_diagnostic` print channel, which every `\tracing*` parameter's text is routed through.
- `src/diagnostic/tests.rs`: Destination-selection, `print_nl` line-break, and scalar-formatting tests for the diagnostic channel.
- `src/env.rs`: Barriered mutable environment storage for meanings, registers, parameters, font values, grouping, journals, and tracked-region journal lineage; typed writes produce canonical semantic mutation receipts, while token-, macro-, and glue-valued cells carry copy-only runtime identities beside compact words.
- `src/engine_state.rs`: Read-only execution mode and state projection consumed by expansion-time enquiries.
- `src/universe.rs`: aggregate state facade, durable snapshots, tracked
  regions, and the fixed-size, non-restoring `DirectOperationMark` that owns
  only an environment-journal cursor and private-revision allocation suffix.
- `src/expansion_diagnostic.rs`: Detached recoverable expansion diagnostic
  values shared by command expansion and execution-side presentation.
- `src/expansion_recovery.rs`: Detached main-control recovery vocabulary that
  keeps execution independent of the command expansion error tree.
- `src/env/banks.rs`: Dense fixed-size bank codecs, parameter ids, and typed
  bank access helpers. Primitive spellings are owned by `tex-command`'s
  catalogue, not repeated here.
- `src/env/box_bank.rs`: Dense-and-paged box slots combining raw semantic projections, direct `NodeListRef` ownership, and journal-owned assignment/coalescing state.
- `src/env/group.rs`: Group stack, aftergroup/afterassignment handling, group mismatch types, final-value restoration receipts, and environment snapshot logic.
- `src/env/overflow.rs`: Sparse e-TeX overflow register banks for high register numbers.
- `src/env/raw.rs`: Restore-only raw environment writes, semantic word iteration, shadow verification, and raw word helpers.
- `src/env/tests.rs`: Unit tests for environment write barriers, grouping, globals, aftergroup, font banks, and raw restore behavior.
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
- `src/glue.rs`: Copy-only glue ids and immutable glue values; payload reads are admitted by the aggregate runtime-value registry.
- `src/hot_core/arena.rs`: Generic append-only typed region arena with
  compact namespace/generation coordinates, accepted sealed chunk bases,
  candidate-local overlays, suffix rollback, and reusable chunk slots.
- `src/hot_core/arena/layout.rs`: Fixed-width typed coordinates, spans,
  reservations, rollback marks, validation errors, and logical/retained
  accounting values for the region arena.
- `src/hot_core/arena/value_region.rs`: Heterogeneous token, macro, glue, and
  provenance columns sharing one rollback-owned region lifecycle and explicit
  canonical sealed-region root sets.
- `src/hot_core/arena/value_region/store.rs`: Concrete runtime token-list,
  macro, glue, and provenance row facade with copy-only coordinates, atomic
  composite publication, counted region roots, and admitted borrowed views.
- `src/hot_core/arena/value_region/store/storage.rs`: Fallible whole-bundle
  reservation and one-time admitted slice resolution for concrete regions.
- `src/hot_core/arena/value_region/store/registry.rs`: Persistent live
  token/macro/glue/origin-list candidate, append-only identity tables, dense
  coordinate maps, fixed rollback marks, incremental region publication, and
  cold forks that share sealed regions while copying only the private active
  suffix.
- `src/hot_core/arena/value_region/store/registry/tests.rs`: Allocation/read,
  stale/foreign identity, all-live growth, and bounded-retry registry controls.
- `src/hot_core/arena/value_region/store/tests.rs`: Typed-coordinate,
  composite co-location, counted-root, oversized-list, and rollback controls
  for the concrete runtime value store.
- `src/hot_core/arena/value_region/tests.rs`: Accept/reject, nested owner,
  resource retry, old-mark, exact all-live, and 10,000-cycle plateau controls
  for runtime value regions.
- `src/hot_core/arena/tests.rs`: Coordinate validation, accepted-base sharing,
  candidate isolation, rollback, plateau, and exact-growth controls for the
  HotCore arena substrate.
- `src/hot_core/journal.rs`: Inline-small first-write inverse records, strictly
  nested marks, exact rollback, and parent-epoch transfer over typed mutable
  targets.
- `src/hot_core/layout.rs` and `src/hot_core/layout/tests.rs`: Canonical
  32-bit token words, compact source coordinates, chunk-owned token spans,
  fixed 40-byte input frames, exact TeX input-kind values, and focused layout,
  generation-rejection, and warmed traversal controls.
- `src/packed_input.rs`: Narrow borrow-safe runtime seam through which the
  command input machine uses the canonical 40-byte frame layout without
  exposing arena ownership, reservations, or runtime coordinates.
- `src/hot_core/mod.rs`: Private HotCore storage module boundary; command
  semantics remain outside this substrate.
- `src/hot_core/snapshot.rs` and `src/hot_core/snapshot/tests.rs`: Storage-only
  HotCore aggregate, 152-byte runtime snapshots, atomic restore preflight,
  accepted-base lifecycle, exact-growth controls, and warmed aggregate plateau
  coverage.
- `src/hot_core/stack.rs` and `src/hot_core/stack/tests.rs`: Copy-only compact
  stacks with 32-bit marks, inline common storage, spill reuse, accounting, and
  bounded-cycle controls.
- `src/hot_core/state.rs` and `src/hot_core/state/tests.rs`: Fixed-length
  inline-small dense mutable banks, typed namespace/generation coordinates,
  first-write journal integration, stale rejection, nested rollback, and
  plateau controls.
- `src/hot_core_benchmark.rs`: Testing-feature scalar facade for the standalone
  HotCore snapshot latency and allocation gates; live runtime coordinates stay
  crate-private.
- `src/hyphenation.rs`: Hyphenation pattern trie and exception table implementing Liang-style position lookup.
- `src/hyphenation/tests.rs`: Unit tests for hyphenation patterns, exceptions, bounds, and overlapping matches.
- `src/identity.rs`: Shared generation-tagged runtime identity allocator for rollback-truncated stores.
- `src/identity/tests.rs`: Property and boundary tests for rollback, fork, exhaustion, and foreign-handle rejection.
- `src/ids.rs`: Opaque ids for token lists, live origin-list projections, macros, glue, fonts, snapshots, and borrow-scoped compact node-payload coordinates.
- `src/ids/tests.rs`: Unit tests for opaque id raw values and node/origin-list span metadata.
- `src/input.rs`: Snapshot-ready lexer/input stack summaries with copy-only token-list ids, macro replay sites and argument slots, source ids, and generic checkpoint future-state comparison.
- `src/input/tests.rs`: Structural-sharing tests for frozen input-summary roots and source payloads.
- `src/interner.rs`: Control-sequence name interner with dense symbols, lookup, hashing, and rollback marks.
- `src/interner/tests.rs`: Unit tests for symbol interning, resolution, rollback, and content hashing.
- `src/journal.rs`: Append-only journal records, markers, undo entries, copy-only token/macro/glue old/new sidecars, and rollback/group replay support.
- `src/journal/tests.rs`: Unit tests for journal positions, markers, entry traversal, and truncation.
- `src/lib.rs`: Public module declarations and re-exports forming the `tex-state` API surface.
- `src/macro_store.rs`: Copy-only macro-definition ids, allocation-free parameter programs, semantic meanings, detached provenance DTOs, and borrowed registry views for replay.
- `src/math.rs`: Immutable math-list model for noads, fields, fractions, styles, choices, and math font families.
- `src/meaning.rs`: TeX meaning representation, primitive enums, flags, and packed raw meaning encode/decode logic.
- `src/meaning/tests.rs`: Unit tests for meaning round trips, flag packing, and primitive encoding.
- `src/memo.rs`: Opaque schema-versioned detached memo envelopes, handle-free transition/effect/result DTOs, and aggregate token/glue/macro/node/font import APIs.
- `src/memo/tests.rs`: Cold/fork/rollback Cross-Universe memo import, provenance stripping, corruption, bounds, kind, and semantic round-trip tests.
- `src/measurement.rs` and `src/measurement/hot_core.rs`: `profiling-stats`
  process-local allocation-owner, loaded-format restoration-work, TeX82
  diagnostic-projection reuse/loss, and current main-control structural
  counters used by dedicated profiling builds.
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
- `src/node_arena/measurement.rs`: `profiling-stats` compact-column and peak-storage accounting.
- `src/node_arena/measurement/tests.rs`: Coherence, divergent-maximum, nested-payload, and concurrent peak-measurement tests.
- `src/node_arena/mutation.rs`: Test-only shape-preserving compact-row replacement support for compact-copy measurement coverage.
- `src/node_arena/owned.rs`: Direct `NodeListRef` ownership, consuming builder freeze, private borrow-scoped span resolution, exact weak candidate reuse, canonical empty ownership, and retained-byte accounting.
- `src/node_arena/owned/tests.rs`: Collision, canonical-empty, transactional freeze, child resolution, clone/final-drop, weak-metadata plateau, all-live, and allocation-independent semantic controls for direct node-list ownership.
- `src/node_arena/schema.rs`: Exhaustive allocation-free logical node descriptors, typed handle policies, origins, and ordered child traversal.
- `src/node_arena/semantic.rs`: Versioned, allocation-independent semantic identity for immutable node-list aggregates.
- `src/node_arena/storage.rs`: Canonical node words, aligned provenance plus copy-only token/glue coordinate sidecars, and immutable payload encoding.
- `src/node_arena/tables.rs`: Typed structure-of-arrays sidecar tables for boxes, unsets, insertions, and noads.
- `src/node_arena/view.rs`: Zero-allocation node references, list spans, raw tag predicates, character runs, and iterators.
- `src/page.rs`: Snapshot-owned page-builder state with copy-only last-glue and scalar/class mark coordinates, page dimensions/integers, contribution/current-page queues, and fire-up records.
- `src/patch_domain.rs` and `src/patch_domain/tests.rs`: Private-revision aggregate allocation ownership, exact single-operation marks, explicit root-set transfer, and focused lifecycle controls; it contains no per-value liveness marker.
- `src/pdf.rs`: Checkpointed pdfTeX document mode with copy-only token coordinates in catalog/page/form collections, deterministic object allocation, snapshots, suffix transfer, and committed-page ledger.
- `src/pdf/action.rs`: Typed, checkpointed PDF action model carrying copy-only token coordinates shared by catalog, link, and outline scanners.
- `src/pdf/annotation.rs`: Checkpointed general-annotation reservations with copy-only token coordinates, running dimension specs, and logical/open-link records.
- `src/pdf/outline.rs`: Immediately allocated, checkpointed PDF outline entries owning their attributes, title, action, and action/item/title identities.
- `src/pdf/object.rs`: Copy-on-write raw PDF object reservations, coordinate-valued initialization payloads, and last-object state.
- `src/pdf/document.rs`: Copy-on-write coordinate-valued raw document dictionary and trailer fragments in source order.
- `src/page/sequence.rs`: Canonical persistent binary-forest sequence for growing current-page nodes.
- `src/page/state_hash.rs`: Page semantic cursors, bounded derived projection caches, and component framing.
- `src/page/tests.rs`: Page snapshot-root sharing and copy-on-write isolation tests.
- `src/print.rs`: tex.web §54's print `selector`, §§57--65's print primitives, §73's `print_err`, and §82's `error` report channel.
- `src/print/error_context.rs`: tex.web §§310--318's `show_context` two-line pseudoprint, bounded eager before/after projections captured at the live input seam, §314's token-list labels, and §310's `\errorcontextlines` elision, shared by every input-stack owner.
- `src/print/tests.rs`: Unit tests for context widths, selector routing, help routing, and error-report completion.
- `src/provenance.rs`: Structural origin-record authority, copy-only
  `OriginListRef` facade and borrowed aggregate views, packed
  macro-invocation records with cold root materialization, demand policy,
  explicit provenance budgets, and record retry leases. Exact list entries
  live only in the aggregate runtime value region.
- `src/provenance/tests.rs`: Structural record sharing, packed-key, allocation,
  readback, retry, fork, list-region budget, and rollback provenance controls.
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
- `src/stores.rs`: Internal aggregate store tuple that coordinates interner,
  env, token, provenance, glue, font, input, and rollback/shipout scope state;
  node lifetimes remain entirely in the structural `NodeListRef` fields of
  those aggregates, while the derived TeX82 memory projection survives
  unchanged operation boundaries and follows canonical root mutations. Its
  direct-operation admission/commit advances the environment first-write
  epoch and node watermark without creating an aggregate snapshot.
- `src/stores/handles.rs`: Store-boundary admission checks for symbols, token, provenance, glue, font, macro, and node coordinates.
- `src/stores/low_memory.rs`: Compact TeX variable-size free-ring and rover projection.
- `src/stores/exact_identity.rs`: Commutative current-cell accumulator and constant-size rollback image for canonical identities of non-default environment cells.
- `src/stores/node_semantic.rs`: Canonical node encoding and bottom-up semantic-identity composition at aggregate freeze.
- `src/stores/format.rs`: Deterministic versioned format-image DTO capture, reachable token/macro/glue/node closure remapping, direct Env/PDF node-owner installation, validation, and fresh-store reconstruction.
- `src/stores/format/frozen_core.rs`: Fixed-width schema-11 names, token-list, macro, and glue section codecs plus direct validated dense-store restoration.
- `src/stores/format/frozen_non_node.rs`: Schema-11 font, code-table, and hyphenation section codecs plus direct validated store restoration.
- `src/stores/format/frozen_node.rs`: Schema-11 fixed-record reachable node-graph codec, semantic-identity validation, and frozen arena installation metadata.
- `src/stores/format/frozen_env.rs`: Schema-11 fixed-record environment-cell codec and validated immutable-base installation input.
- `src/stores/format/node.rs`: Handle-free serialized node/math DTO graph and validated conversion to and from live nodes.
- `src/stores/format/tests.rs`: Focused schema-11 frozen-store round-trip tests for nodes, registers, e-TeX reset behavior, and control-sequence namespaces.
- `src/stores/format/font_validation.rs`: Pre-publication validation of detached font metrics, identifiers, and serialized Env font banks, plus test-only corruption fixtures.
- `src/stores/state_hash.rs`: Store snapshot cursor and semantic hashing implementation for changed cells and store-owned slices.
- `src/stores/tests.rs`: Unit tests for aggregate store rollback, builders, handle validation, parameters, boxes, and state hashes.
- `src/tests.rs`: Crate-level integration-style unit tests for `Universe`, snapshots, world effects, and module test wiring.
- `src/tests/handle_matrix.rs`: Table-driven aggregate rollback, fork, and cross-Universe liveness coverage for every production opaque handle class.
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
- `src/token_store.rs`: Copy-only token-list ids, reusable scanner scratch, semantic-identity framing, and borrowed registry views.
- `src/stores.rs`: Also owns the unified runtime-value registry and published
  region-root set; checkpoints and private-operation rollback restore published
  roots before truncating registry suffixes.
- `src/universe.rs`: Top-level TeX state timeline and sole state API, with
  snapshots, effect commits, execution-side dependency-aware getters and
  barriers, precise World mutation-guard stamping, the strong frozen-macro
  primitive-registry sidecar, immutable provenance demand/budgets, direct
  artifact-root resolution, and the input-open capability context. Restoration traces resolve
  named parameters through the installed primitive registry rather than a
  state-local spelling table. It also admits snapshot-free direct-operation
  commit only outside private revision allocation domains.
- `src/universe/tests.rs`: Unit tests for `Universe` mutation, snapshots, contexts, effects, and boundary behavior.
- `src/world.rs`: External-effect boundary for files, atomic downstream
  file-set publication, streams, clocks, randomness, shell policy, printing,
  coordinate-valued deferred-write effects, artifact-owned rendered-source
  roots/recipes, weak snapshot-root mounts, and field/key-specific
  allocation-independent dependency projections.
- `src/world/tests.rs`: Unit tests for world snapshots, file records, streams, printing, randomness, shell escape, effect replay, and snapshot-owned effect-root reclamation.
- `tests/it.rs`: Integration test harness that includes capability-boundary and live-boundary test modules.
- `tests/structural_node_lifecycle.rs`: Focused success, committed-failure, rollback, retry, rejection, checkpoint, and generation-fork controls for structural node-list ownership.
- `tests/it/capability_boundaries.rs`: Compile-fail integration tests asserting restricted input and transaction capabilities fail to compile.
- `tests/it/handle_serialization.rs`: Downstream compile-fail probe proving serde and private constructors cannot mint live handles or handle-bearing nodes.
- `tests/it/live_boundary.rs`: Downstream compile-fail assertion ensuring private stores and raw environment mutation stay inaccessible.
- `tests/ui/input_open_context_forbidden.rs`: Compile-fail fixture that attempts forbidden reads, world access, and mutations from `InputOpenContext`.
- `tests/ui/arena_transaction_exclusive.rs`: Compile-fail fixture proving suffix-owning transactions exclusively borrow the aggregate timeline.
- `tests/ui/*-boundary-forbidden.rs`: Independent compile-fail fixtures attempting to bypass private live-state stores, omit paired editor-layout validation, or bypass the `Universe` facade.
- `tests/ui/handle_serialization_forbidden.rs`: Compile-fail fixture attempting to serialize, deserialize, or construct live handles downstream.

## Boundaries

- Do not expose raw substores, raw checkpoint/restore hooks, raw word decoders, or opaque handle constructors outside crate-private or test-only APIs.
- Do not let downstream crates mutate state directly; keep the live-store boundary production-like, including under `shadow`.
- Do not place expansion or execution policy here when it belongs in `tex-command` or `tex-exec`; state should provide the substrate and invariants.
- Keep all host I/O and effectful facts behind `World`; engine crates should not reach for `std::fs`, clocks, random sources, or shell execution directly.
- Validate symbol-keyed or handle-keyed writes against the owning interner/store liveness before accepting them.

## Validation

Run `cargo test --tests -p tex-state` for state changes. For boundary-sensitive changes, include the live-boundary, replay, shadow, and compile-fail coverage that exercises the affected facade.
State performance benchmarks live in the standalone `benchmarks/tex-state` crate and are run explicitly with `cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets`.
The fixed-size HotCore substrate has an independent assertion-bearing gate at
`cargo run --manifest-path benchmarks/hot-core-snapshot/Cargo.toml`; its
Criterion rows compile with `cargo bench --manifest-path benchmarks/hot-core-snapshot/Cargo.toml --bench snapshots --no-run`.
