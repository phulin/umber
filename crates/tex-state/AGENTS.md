# tex-state Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the live TeX state layer and the primary boundary between engine logic, durable snapshots, and host effects.

## Crate Role

`tex-state` owns `Universe`, the aggregate facade for live engine stores, and `World`, the controlled interface for files, streams, clocks, randomness, shell escape policy, and effect records. It stores meanings, registers, code tables, token lists, glue specs, nodes, boxes, fonts, hyphenation data, input summaries, grouping/journaling state, epochs, and snapshot/replay support.

All production mutation of live TeX state should pass through `Universe` or similarly aggregate facades. This crate also owns the barriered APIs that keep rollback, grouping, effect commit, and replay behavior coherent.

## File Map

- `AGENTS.md`: Local guidance for agents working in the `tex-state` crate.
- `Cargo.toml`: Crate manifest, dependencies, features, library target, and integration test wiring.
- `src/cell.rs`: Packed environment cell identifiers and bank tags shared by journals, raw storage, dependency tracking, and semantic hashing; assignment scope is stripped through the canonical `CellId` helper.
- `src/capacity.rs`: Typed compact-conformance and pinned TeX Live 2026 executable-process capacity profiles used by format validation, runtime selection, and terminal accounting.
- `src/cell/tests.rs`: Unit tests for cell id packing, bank decoding, and global-bit handling.
- `src/code_tables.rs`: Storage-independent code-table value vocabulary; the
  live cat/lc/uc/sf/math/delcode banks and INITEX virtual defaults are owned by
  `DenseState`.
- `src/checkpoint.rs` and `src/checkpoint/tests.rs`: Move-only coarse-owner
  checkpoints, bounded cursor tuples, mutation-free restore planning, and the
  owner/state/root/truncation/release ordering barrier.
- `src/command_context.rs`: Already-admitted session/generation borrow for
  direct command and execution work, including name-free compact
  control-sequence meaning delivery and the destination-directed packed-token
  resolution entry implemented beside the token encoding, typed register mutation,
  page-list/page-builder access, font metrics and detached artifact recipes,
  generated-font lookup, grouped box transfer, page marks, hyphenation,
  detached paragraph-shape and e-TeX penalty-array projections with journaled
  assignment, and bounded assignment-diagnostic rendering,
  including e-TeX's assignment-free forced-online diagnostic scope,
  PDF traversal/form/color operations, output-stream normalization,
  definitions, token/glue allocation, and dependency-aware mutations without
  per-read owner admission. Page-material allocation/copy counters remain
  observable through this boundary so retained-range zero-copy gates are not
  inferred from a separate arena owner.
- `src/dependency.rs`: Region-scoped dependency keys with scope-free `CellId` environment identity, typed recorder lifecycle and first-reason poison barrier, detached observations, changed-at validation, conservative page/PDF family clocks, registered World-backed mutation keys, semantic backdating, and opaque cross-Universe memo validation stamps.
- `src/dependency/tests.rs`: Dependency mutation matrix, generic tracked-region lifecycle and journal-write records, deterministic ordering, rollback failure closure, and handle-independent observation tests.
- `src/diagnostic.rs`: tex.web §245's shared `begin_diagnostic`/`end_diagnostic` print channel, which every `\tracing*` parameter's text is routed through.
- `src/diagnostic/tests.rs`: Destination-selection, admitted forced-online
  routing without eqtb assignment, `print_nl` line-break, and scalar-formatting
  tests for the diagnostic channel.
- `src/definition_arena.rs`: Private generation-branded non-atomic shared
  macro-definition owners and the checked destination-policy builder which
  constructs their single allocation in place. Publication adds the serial,
  accounting charge, and semantic owner without allocating or copying words;
  successful attempt retirement releases scratch ownership, while the last
  semantic owner releases both accounting and payload. The exact publisher
  cursor returns a rejected checkpoint loan to its private suffix mark without
  retaining released bodies. Generic promotion borrows an already-checked
  attempt builder through complete batch preflight; failure leaves it in place.
  Preflight must validate every preserved identity policy before the first row
  is published and must never reconstruct a destination-policy builder from
  parameter/replacement slices.
- `src/durable_arena.rs`: Private generation-branded non-atomic shared stored
  token-list owners, reusable publication builders, allocation-free owning
  views/cursors, and exact private-suffix rollback for token, glue, and
  provenance publication.
- `src/env.rs`: Generation-branded eqtb-equivalent current state, exact TeX
  local/global save semantics, group boundaries, and journal-cursor restore.
- `src/env/durable_boxes.rs` and `src/env/durable_boxes/tests.rs`: move-only
  durable node-closure register owners, exact group/operation/checkpoint
  owner swaps, bounded history-preservation copies, and lifecycle tests.
- `src/env/font_runtime.rs`: Direct-index generation-owned mutable per-font
  dimensions, character settings, PDF code tables, and ligature state.
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
- `src/effect_journal.rs` and `src/effect_journal/tests.rs`: Validated
  in-session effect-ledger ownership, aligned runtime-local publication
  metadata, prefix splicing, and terminal materialization. Cold consumers
  detach the owned `EffectRecord` values; publication identities and ordering
  sidecars are never serialized.
- `src/etex_tracing.rs` and `src/etex_tracing/tests.rs`: e-TeX 2.6's `\tracinggroups` group-enter/leave transcript trace, printed through the shared `\tracing*` diagnostic channel; `\tracingassigns`'s value rendering lives in `tex-exec` instead, against the primitives declared here, and `\tracingifs` renders directly in `tex-command` through the same channel.
- `src/file_framing.rs` and `src/file_framing/tests.rs`: tex.web §54's `open_parens` and the §537/§362/§1335 prints that maintain it, held as print-adjacent `World` state so the command core can close a file's paren at §362's own point, ahead of the `check_outer_validity` diagnostic that follows it.
- `src/font.rs`: Generation-owned fixed-capacity immutable font-context chunks,
  rollback-coupled logical font handles, null font, missing-character records,
  fixed checkpoint marks, and handle-free artifact-facing recipes whose
  generated sources are named by semantic identity. Publication validates font
  roots once; checkpoint capture and restore never scan meaning, node, or PDF
  prefixes for liveness.
- `src/fork_arena.rs` and `src/fork_arena/tests.rs`: Safe caller-owned
  fixed-byte-chunk coarse page pools, coordinate-only typed semantic-lane
  arenas, pool-chunk-local owner-relative positions without per-region sparse
  resolver prefixes, resident-slot page payload publication, move-only
  detached active-list builders with explicit pool mutation, constant-time opaque-root admission into stable borrowed
  views whose ordinary reads carry owner-relative chunk/offset cursors without
  repeating owner or incarnation validation, allocation-free logical-order
  chunk-slice and ranged callback visitation over the sole predecessor chain,
  linear forward callbacks whose Rust-stack continuation replaces successor
  metadata, mutation-compatible coordinate-only chunk cursors whose short
  value borrows do not survive an append, sequential compatibility iterators
  that retain their owner-relative cursor within each packed block, explicit
  cold structural audits, constant-size live-frontier authentication at
  lifecycle boundaries, and reverse `ChunkCursor` traversal, canonical
  nonrecursive range lists, partial operation rollback, whole-chunk retained
  marks, exclusive batch promotion, and exactly accepted-versus-forked
  settlement. Generated region values initialize their final resident slot
  before identity and direct-child dependency completion; rejection truncates
  that exact unpublished reservation.
- `src/format.rs` and `src/format/tests.rs`: Consuming destination-stamped
  format staging, decoded-row draining into final owners, and infallible atomic
  publication after complete validation.
- `src/format/schema.rs`: Handle-free schema-11 logical rows for names,
  immutable values, and sparse environment cells.
- `src/format_container.rs`: Portable schema-11 format-image header, section directory, authoritative fingerprints, checksum, compression, and structural validation; no compatibility codec is retained.
- `src/format_container/tests.rs`: Focused frozen-container header, directory, checksum-coverage, fingerprint, compression, and geometry tests.
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
- `src/hyphenation.rs`: Liang-style position lookup with one coarse immutable
  owner for the initialized pattern trie and one reversible direct-state
  journal for mutable exceptions, saved codes, and capacity scalars.
- `src/hyphenation/storage.rs`: Coarse initialized-trie owner, fixed checkpoint
  mark, move-only accepted/candidate settlement for the mutable runtime, and
  cold format wire decomposition.
- `src/hyphenation/tests.rs`: Unit tests for hyphenation patterns, exceptions,
  bounds, overlapping matches, frozen-owner semantics, and exact mutable
  checkpoint settlement.
- `src/identity.rs`: Shared generation-tagged runtime identity allocator for rollback-truncated stores.
- `src/generation.rs`: Fresh invariant generation brands, private publisher
  construction, episode-level guarded admission, cloneable coarse generation
  ownership, bounded publisher cursors for checkpoint loans, and
  whole-generation retirement of inline arenas/capacity.
- `src/ids.rs`: Opaque snapshot and font handles retained outside the deleted runtime-value ownership substrate.
- `src/input.rs`: Storage-independent input policy, replay-kind, alignment
  phase, and source-id vocabulary shared with the command-owned input stack.
- `src/packed_input.rs`: Fixed-width packed input-frame metadata and flags;
  token/source replay payload ownership remains in `tex-command`.
- `src/interner.rs`: Bounded append-only session epoch for control-sequence
  names and retained spellings, with one-probe find-or-intern status, explicit
  foreign-session admission, and whole-epoch retirement.
- `src/interner/tests.rs`: Session budget, namespace, stability, admission,
  and retirement tests.
- `src/session_epoch.rs`: Cloneable coarse owner and exclusive physical lease
  for one append-only interning epoch shared by successive revision
  generations.
- `src/shipout_scratch.rs` and `src/shipout_scratch/tests.rs`: One
  generation-owned reusable output-attempt lane, typed Page/Durable/Scratch
  traversal coordinates and token/node source handles, scalar suffix marks,
  warmed direct-row construction, and compile/runtime escape controls.
- `src/journal.rs`: Separate compact TeX group saves, typed chunk-arena named-
  checkpoint deltas holding one reversible alternate per written cell, fixed
  save-stack projections, exact capacity-change accounting for constant-time
  budget reads, prefix-independent accepted/current suffix settlement, and
  reusable operation-local undo with owner-checked stable cursors.
- `src/journal/cell.rs`: Private packed encoding for the typed dense-state
  coordinates stored by narrow journal records.
- `src/journal/tests.rs`: Split-lifetime rollback, packed-width, fixed-mark,
  first-alternate deduplication, cursor, and foreign-owner rejection tests.
- `src/lib.rs`: Public module declarations and re-exports forming the `tex-state` API surface.
- `src/macro_definition.rs`: Storage-independent allocation-free macro parameter programs retained for the replacement definition arena.
- `src/math.rs`: Immutable math-list model for noads, fields, fractions, styles, choices, and math font families.
- `src/meaning.rs`: Static packed TeX meanings plus generation-typed shared
  macro owners; raw integers never materialize runtime definition handles.
- `src/meaning/tests.rs`: Static codec, primitive, and typed macro-meaning tests.
- `src/measurement.rs` and `src/measurement/hot_core.rs`: Profiling-feature-only
  allocation attribution, structural dispatch census, and coarse retained
  generation lifetime counters. Ordinary builds compile neither the module nor
  any associated fields, branches, or atomics.
- `src/memory_accounting.rs`: Generation-local constant-time TeX main-memory
  totals updated by immutable payload publication/final release and node-arena
  publication/release; it contains no root registry or liveness index.
- `src/memo.rs`: Opaque schema-versioned detached memo envelopes, handle-free
  transition/effect/result DTOs, explicit validation staging, and
  generation-local token/glue/macro publication.
- `src/memo/tests.rs`: Cross-generation spelling/value import, corruption,
  bounds, staging-before-publication, and semantic round-trip tests.
- `profiling-allocator/`: isolated profiling-only `GlobalAlloc` forwarding
  shim used by executable profiling builds to attribute allocation calls and
  requested bytes to nested hot-core owner scopes.
- `src/node.rs`: Storage-independent TeX node and box values with copy-only
  provenance, directly owned glue, shared immutable stored-token payloads, and
  typed list coordinates.
- `src/node_sequence.rs`: Explicit mirrored-or-distinct semantic and
  TeX-physical operation buffers. Mirrored hot lists store one node/inline
  lineage channel with demand-enabled composable semantic identity; cold detached extraction can
  materialize two owned channels. Named checkpoints retain direct child
  coordinates without publishing duplicate page-arena rows. The module also
  owns TeX-cell lineage metadata and semantic-only equality.
- `src/node_arena.rs`: Generation page and cold loaded-node arenas; copy-only
  typed/rebranded coordinates; shared immutable checkpoint rows; exact
  branch-local generation frontiers; owner-checked suffix cursors; borrowed
  resolution, including the replacement page-material `ArenaListView` cursor,
  direct linear `NodeCursor::for_each`/`try_for_each_range` callbacks, and
  compatibility start-position iteration;
  demand-enabled layout-independent list identities; and cold-only exact-root
  relocation.
- `src/node_arena/tests.rs`: Scratch/page/durable exact-closure relocation,
  owner-checked rollback, invalid-publication controls, completed-page release,
  and stale-coordinate rejection after bounded row reuse.
- `src/node_region.rs` and `src/node_region/tests.rs`: Exclusive move-only node
  regions above the shared fixed-chunk pool, generation-checked owner-relative
  roots and borrows, sealed `ClosureBuildMark` suffix loans, mutation-free
  recursive transfer preflight, direct mapped cross-region construction into
  final packed chunks without whole-node staging, address-stable
  detach/rollback/rebranding, and reason-counted structural-copy fallback;
  production durable carrier cutover remains a separate migration stage.
- `src/page_node_arena.rs` and `src/page_node_arena/tests.rs`: Page-semantic
  identity facade, checked destination construction, and focused warmed
  1/4,096-node allocation/copy/chunk-work proof over the generic arena.
- `src/page.rs`: Exclusive move-only `PageRegion` ownership over page payload,
  the four checked `PageListSpan` PageBuilder roots, scalar state, reversible
  same-region journal, and private owner-relative checkpoint rows; active
  insertion classes and sparse mark classes retain canonical iteration order
  beside dense direct lookup indexes. One fixed-chunk prior/current journal
  owns every reversible alternate; duplicate insertion/mark mutation lanes and
  canonical-lane rebuild scans are absent.
  Exact edit settlement and shipout succession keep roots and payload suffixes
  atomic, direct node parent/child publication is same-region checked, and
  held-over evacuation uses the explicit semantic-copy boundary. One
  `PageRegionHistory`-owned `NodePool` physically backs current and retained
  page regions; aggregate checkpoint release removes the private row, and the
  last row of a noncurrent region retires its envelopes and stale id.
  Succession preparation consumes the executor's move-only rootless-mode
  receipt; durable box/form carriers still keep the production tail on the
  existing region.
- `src/pdf.rs`: Checkpointed pdfTeX document mode with generation-typed token
  coordinates in catalog/page collections, deterministic object allocation,
  durable form-list coordinates, allocation-free scalar checkpoint marks,
  exact inverse journals, one coarse image/form payload prefix plus private
  delta, committed-page ledger, and handle-free PDF format wire
  capture/materialization hooks.
- `src/pdf/completion.rs`: handle-free terminal projection of the checkpointed
  PDF ledger, including artifacts, resources, raw objects, actions, and final
  document state.
- `src/pdf/tests.rs`: Generation-typed page/action/object coordinates, owned
  image payloads, atomic PDF checkpoint rollback, and handle-free format-ledger
  round-trip/rejection tests.
- `src/provenance_resolver.rs`: Explicit cold-demand admission from
  generation-typed provenance coordinates to owned handle-free diagnostic and
  generated-source presentation DTOs.
- `src/provenance_resolver/tests.rs`: Cold source resolution, detached
  presentation survival, owned generated recipes, bounded traces, and invalid
  range tests.
- `src/pdf/action.rs`: Typed, checkpointed PDF action model carrying copy-only token coordinates shared by catalog, link, and outline scanners.
- `src/pdf/annotation.rs`: Checkpointed general-annotation reservations with copy-only token coordinates, running dimension specs, and logical/open-link records.
- `src/pdf/outline.rs`: Immediately allocated, checkpointed PDF outline entries owning their attributes, title, action, and action/item/title identities.
- `src/pdf/object.rs`: Copy-on-write raw PDF object reservations, coordinate-valued initialization payloads, and last-object state.
- `src/pdf/document.rs`: Copy-on-write coordinate-valued raw document dictionary and trailer fragments in source order.
- `src/page/sequence.rs`: Direct page-lifetime current-page suffix buffer.
- `src/page/state_hash.rs`: Handle-free bounded page semantic cursors and direct
  component framing; no page COW root is retained for hash reuse.
- `src/page_node_arena.rs` and `src/page_node_arena/tests.rs`: Runtime
  page-material facade pairing the canonical coarse-arena coordinate with its
  checked traversal span and demand-maintained semantic identity scalar,
  including span-native zero-allocation compose/slice, coordinate-only direct
  chunk continuation across append-interleaved walks, zero-hash disabled
  execution, identity-preserving split/compose/fork settlement, and exact
  recursive cross-region semantic copy used only by lifetime transitions. Its
  payload is explicitly `Node<PageListId>` so the replacement topology cannot
  retain child coordinates from the superseded row arena during migration.
- `src/page/tests.rs`: Page snapshot value isolation, mark-value, and semantic
  hash rollback tests.
- `src/print.rs`: tex.web §54's print `selector`, §§57--65's print primitives, §73's `print_err`, and §82's `error` report channel.
- `src/primitive.rs`: generation-typed packed handles into a completely
  constructed immutable primitive registry; handles validate the registry
  extent before direct indexed dispatch and never name mutable eqtb cells.
- `src/print/error_context.rs`: tex.web §§316--318's `show_context` two-line pseudoprint for one command-selected level and §314's token-list labels. The command input owner applies §310's `\errorcontextlines` selection during its live stack traversal, before constructing bounded before/after strings.
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
- `src/reachability_store.rs`: One caller-owned session-epoch reachability
  store with same-thread non-atomic ownership, fixed inline prior/current
  physical-generation slots, and
  allocation-free slot reuse across accepted, rejected, and suspended
  candidates, including one typed exclusive pair-admission seam for aggregate
  sidecar settlement before either slot becomes independently live.
- `src/retained_generation.rs`: Opaque move-only physical-revision slot lease,
  lifetime binding to its external store, universally generic admission
  operations, typed accepted/current sidecar settlement, and owner-relative
  engine-sidecar keys that prevent runtime coordinates from escaping the
  external session store.
- `src/pure_memo/tests.rs`: Collision, eviction, retention-release, and disabled-cache tests.
- `src/scaled.rs`: Compatibility re-export for shared TeX scaled-point arithmetic.
- `src/source_map.rs`: Rollback-coupled logical source regions, validated positions/spans, and immutable World/generated backing identities.
- `src/source_map/tests.rs`: Source-region anchors, validation, overflow, rollback/reuse, and O(1)-mark tests.
- `src/source_fragments.rs`: Session-scoped immutable source fragments, editor
  piece tables, demand-selected generation backing, rebound root registration,
  and layout-aware stable-recipe resolution.
- `src/source_fragments/layout_index.rs`: Fragment-and-offset index for logarithmic current/deleted piece resolution across repeated views.
- `src/source_fragments/tests.rs`: Fragment range, deletion, fork-liveness, anchor, allocator, snapshot, and line-index cache tests.
- `src/state_hash.rs`: Deterministic semantic state hasher used by snapshots,
  replay convergence checks, and the Universe-owned executor-boundary builder
  which resolves child/font/value coordinates and erases diagnostic identity.
- `src/stores.rs`: Coarse generation state owner, immutable/mutable admitted
  episode views, named-boundary exact bank materialization, private-suffix
  loan rollback, typed shared-value publication, cold live-format payload
  capture, and whole-generation retirement.
- `src/stores/tests.rs`: Direct arena resolution, generation-id bank
  installation, and coarse retirement tests.
- `src/string_pool.rs` and `src/string_pool/tests.rs`: Dense append-only UTF-8
  ownership, compact end-offset/open-addressed recycling index, O(1) nested
  operation marks with cold in-place suffix rollback, cold format projection,
  and layout, allocation-reuse, collision, and rollback tests.
- `src/tests.rs`: Crate-level semantic unit tests and module test wiring.
- `src/tests/node_semantics.rs`: Canonical node equality/hash coverage proving
  diagnostic provenance, physical topology, and allocator sidecars do not
  affect semantic identity.
- `src/tests/replay.rs`: Feature-gated generated invariant test for exact
  generation-typed checkpoint replay.
- `src/token.rs`: Token and catcode value definitions, constructors,
  classification helpers, destination-directed packed meaning resolution
  which returns its already-decoded literal catcode for command-side delivery
  interception, and inline-small rooted traced-token buffers with
  sparse provenance ownership and spillover storage. Generated runs sharing
  one origin pack every word against its id and move that structural root into
  the sparse owner set once; they never clone it per token before deduplication.
- `src/token/tests.rs`: Unit tests for token constructors, catcodes, parameter tokens, and display/debug behavior.
- `src/token_show.rs`: tex.web §§49/262--294's printable token spellings -- `show_token_list`, `print_cs`, and `\string` rendering over the interner, catcodes, and `\escapechar`.
- `src/universe.rs`: Public session/generation aggregate, typed scalar
  mutation and allocation facade, exclusive retained-checkpoint state/node
  bank loans, immutable primitive-registry sharing, owner-checked journal
  cursors, admitted command/execution views, callback-scoped hot command
  admission that constructs and consumes the reference aggregate in its callee
  slot, borrow-only pure-memo capability, root-before-suffix shipout
  transactions, and whole-session retirement.
- `src/universe/tests.rs`: Session/generation admission, rollback-independent
  interning, foreign-session rejection, and retirement tests.
- `src/world.rs`: External-effect boundary for files, atomic downstream
  file-set publication, streams, clocks, randomness, shell policy, printing,
  handle-free deferred-write memos, artifact-owned detached rendered-source
  recipes, value-stamped snapshot-root mounts, direct stream state with fixed
  input/path cursors and scalar printer offsets, reusable detached-prior
  effect/input/artifact journals for candidate settlement, and
  field/key-specific allocation-independent dependency projections.
- `src/world/tests.rs`: Focused detached effect, owned artifact/provenance,
  input cloning, fixed stream-mark capture, repeated candidate settlement,
  detached-buffer and payload-address reuse, snapshot rollback, and
  effect-root tests.
- `tests/it.rs`: Integration test harness that includes capability-boundary and live-boundary test modules.
- `tests/structural_node_lifecycle.rs`: Focused success, committed-failure, rollback, retry, rejection, checkpoint, and generation-fork controls for structural node-list ownership.
- `tests/it/capability_boundaries.rs`: Compile-fail integration tests asserting restricted input and transaction capabilities fail to compile.
- `tests/it/handle_serialization.rs`: Downstream compile-fail probe proving serde and private constructors cannot mint live handles or handle-bearing nodes.
- `tests/it/live_boundary.rs`: Downstream compile-fail assertion ensuring private stores and raw environment mutation stay inaccessible.
- `tests/ui/input_open_context_forbidden.rs`: Compile-fail fixture that attempts forbidden reads, world access, and mutations from `InputOpenContext`.
- `tests/ui/arena_transaction_exclusive.rs`: Compile-fail fixture proving suffix-owning transactions exclusively borrow the aggregate timeline.
- `tests/ui/*-boundary-forbidden.rs`: Independent compile-fail fixtures
  attempting to bypass private live-state stores or the `Universe` facade.
- `tests/ui/handle_serialization_forbidden.rs`: Compile-fail fixture attempting to serialize, deserialize, or construct live handles downstream.
- `tests/ui/reachability-store-boundary-forbidden.rs`: Compile-fail fixture
  proving downstream code cannot open the store epoch, access physical slots,
  or name a raw slot key.

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
State performance benchmarks live in the standalone `benchmarks/tex-state`
crate. The `pdf_checkpoint_gate` and `pdf_fork_metadata` binaries measure
allocation-free scalar marks and exclusive transactional candidate setup by
PDF field family; `hyphen_checkpoint_gate` proves capture, checkpoint clone,
restore, and fork allocation are flat across initialized trie sizes while a
fixed bounded mutable payload remains rollback-safe. The broader state budgets
run explicitly with
`cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets`.
