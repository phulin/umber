# Docs Guidance

Read the repository-level `AGENTS.md` before editing here. Documentation should describe the current fixture workflow: `cargo test --tests` exercises every host-testable workspace member against committed fixtures, its coverage is bound by `crates/test-support/tests/workspace_selection.rs`, expensive environment-specific checks remain explicit opt-in commands, and `scripts/regen-fixtures.sh` is the only supported live-reference regeneration entry point.

When documenting tests or parity workflow, point fixture changes to `scripts/regen-fixtures.sh` modes rather than cargo-test environment variables or retired scripts.

Documents here embed TeX syntax, and `dprint` rewrites markdown content rather
than only its layout. Follow the "Writing Markdown" rules in the repository-level
`AGENTS.md` before adding a code span, a fenced block, or a wrapped link.

`snapshot_performance.md` defines the focused snapshot latency and retained-allocation gate, including its asymptotic budgets and measurement semantics.

`aggregate_checkpoint_contract.md` defines the component-by-component final
checkpoint marks, restore order, retention charges, complete optional identity,
representative pre-refactor benchmark, and later flat-scaling gates.

`profiling.md` documents the persistent in-process Gentle profiler, its
Samply wrapper, prerequisites, counters, measurement controls, and capture
analysis workflow. Historical measurements belong in Git history or Beads,
not as chronological release receipts in `docs/`.

`native_batch_kernel.md` records the independently audited direct mutable batch
ceiling, the first production-owned canonical-tokenizer/output seam, its typed
fallback boundary, and the staged single-engine migration that deletes each
layered predecessor as it lands.

`cargo_feature_axes.md` is the contract for what a Cargo feature may mean:
the four axes, the crate that owns each declaration, why `required-features`
is not one of them, and why no engine crate has a non-empty `default`. Read it
before adding a feature to any manifest.

`testing_policy.md` is forward-looking guidance for test design and placement.
`testing_infrastructure.md` inventories the current test commands, budgets,
fixtures, corpora, and harnesses; update it when those implementation facts
change.

`tooling_surface_inventory.md` records the owner-approved disposition of
reference/parity commands, one-time fixture migrations, benchmark and trace
rows, prototypes, fuzz tiers, and their scripts for `umber2-vgjr.18`; absence
from routine CI is never deletion evidence.

`typesetting_assertion_ledger.md` maps every aggregate typesetting assertion
removed by `umber2-vgjr.10.3` to an active case-level owner and records retained
unique evidence.

`typesetting_browser_compaction_ledger.md` records the post-program 7, 10, and
13 audit of typesetting, HTML, WASM, Node, DOM, worker, hostile-input, and real
browser evidence. It names the active owner of the one retired dormant wire
case and the independently valuable cases that must remain.

`command_assertion_ledger.md` closes the scanner/delivery compaction audit. It
maps value, event-order, recovery, lifecycle, rollback, source-context, and
identity evidence to active owners, records narrative/matrix dispositions, and
limits deletion to proven duplicate setup scaffolding.

`golden_corpus_dispositions.md` records the final owner and test tier for every
legacy execution golden area, the exact reasons retained integration cases do
not belong in command-semantic, and the property-scoped replacements for
retired duplicates.

`frozen_format.md` defines the portable format-image container ABI, exact
fingerprints, deterministic lookup-table representation, validation and
checksum coverage, immutable/job-local split, and migration from schema 9.

`format_cache.md` defines generated-format cache identity, validated atomic
native entries, corruption recovery, and the browser portability boundary.

`texlive_release_selection.md` defines annual TeX Live selection, the boundary
that reserves locks and manifests for downloaded files, generated-format cache
binding, and multi-release pdfTeX parity without compiling each historical
engine.

`arxiv_census/` contains machine-readable captures for the recent arXiv sample.
Its README records the exact interpretation and partial-capture status.

`incremental_v1.md` fixes the named-boundary schedule, editor-session
retention, edit mapping, pruning, and schedule-relative convergence contract
for the first incremental engine.

`patch_allocation_domains.md` defines private-revision allocation ownership,
single-operation marks, exact rollback, explicit root transfer, and rejection
without compaction or historical-domain registries.

`tracked_region_coverage.md` defines the exact ordinary main-control operation
covered by generic dependency recording, its begin/finish and fail-closed
ownership, the exhaustive semantic read/barrier matrix, the command/execution
implementation split, and the required perturbation proof. It does not
authorize replay or paragraph continuation.

`alignment_brace_semantics.md` is the canonical TeX82/pdfTeX mapping for
`align_state`, token-delivery corrections, nested alignment ownership,
template retirement, and recovery.

`tex_command_core.md` defines the authoritative target architecture for the
canonical TeX82/e-TeX/pdfTeX command-machine replacement tracked by Beads epic
`umber2-johp`, including state ownership, exact and Unicode profiles, input
levels, command delivery, expansion, scanners, extensions, provenance,
incrementality, one-owner bounded command snapshots, exact in-session suspended
attempts, handle-free detached continuations, reference oracles, and
optimization promotion.

`engine_architecture_decision.md` selects bounded mutable semantic episodes
inside the one canonical engine from the `umber2-64v2` prototype evidence. It
records measurement comparability, retained and rejected substrate, migration
order, typed barrier/fallback rules, deletion criteria, and promotion gates.

`main_control_replacement.md` specifies the planned arena-backed,
snapshot-native canonical hot core tracked by `umber2-awgc`: packed token and
macro storage, fixed-size journal/arena marks, persistent fused dispatch, cold
evidence publication, module boundaries, migration order, and pinned arXiv
performance gates.

`runtime_storage_lifetimes.md` is the normative end-state contract for session
interning epochs, dense journaled TeX state, generation-scoped immutable
definitions, operation and node arenas, promotion, checkpoints, compaction,
provenance, and handle-free boundaries.

`node_region_ownership.md` is the authoritative node-specific ownership
contract. It preserves exact paragraph restart while defining exclusive page
and durable regions, TeX move/copy transitions, two-lineage suffix settlement,
held-over evacuation, and the static prohibition on naked owning list
coordinates. It supersedes page-batch dependency/refcount designs.

`expansion_memory_lifetimes.md` maps that end-state contract onto the current
expansion, scanner, input, suspension, incremental-candidate, and format code.
It also records the source-audited retention classes and verified migration
gaps; update it when an owner or exact reclamation point changes.

`runtime_storage_contract_tests.md` maps the external TRIP/e-TRIP, tracer,
artifact, CLI, format, retry, diagnostic, checkpoint, rollback, and incremental
contracts that survive the runtime-storage rewrite and records the deleted
ownership-era compatibility assertions.

`writeback/` records concise issue-scoped authority notes required by command
conformance work; each note names the governing TeX82 section and the adopted
semantic boundary, not temporary implementation plans.

`tex82_property_catalogue.md` defines the pinned 1,380-module TeX82 inventory, explicit disposition and executable-property schemas, reviewed shard contract, and hermetic completeness gate.

`pdftex_extension_property_catalogue.md` defines the separate pinned pdfTeX
extension property ownership and channel-disposition contract for retained
executor observations without duplicating the canonical primitive inventory.

`command_semantic_fixtures.md` defines repository fixture contract v1 for
committed canonical command streams, profile/tool/source/output identity,
mandatory WEB citations, hermetic correctness consumption, and explicit live
regeneration selection.

`etex26_oracle.md` defines the pinned canonical e-TeX 2.6 Web2C source and
toolchain boundary, explicit compatibility/extended INITEX profiles, final
schema-v1 base-command instrumentation seam, offline reuse, and build-record
contract.

`persistent_compile_sessions.md` defines the unified native/WASM compile
session lifecycle that composes typed resource retries with revision-checked
root-buffer patches and retained incremental execution.

`generated_input_stabilization.md` defines the implemented correctness and
lifecycle contract for positive and negative generated-input dependencies,
safe `JobStart` fallback, provisional editor output, bounded off-hot-path
fixed-point stabilization, and safe cold execution after external-input
changes.

`stepwise_execution.md` defines the owned `tex-exec` run, atomic per-step
snapshot/replay protocol, typed resource sites, lifecycle, cumulative fuel and
cancellation rules, and the migration from whole-attempt retries.

`mode_list_rollback_journal.md` records the measured retained-COW mode-list
cost, the required nested inverse-journal invariants, and the mutation-boundary
gate that must be satisfied before replacing the aggregate rollback root.

`incremental_memoization.md` records the deleted changed-document paragraph
memoization design and points to the current restart-from-summary contract.

`paragraph_replay_deletion_baseline.md` records the reproducible before/after
deletion measurements and workload identities.

`retained_group_roots.md` specifies the proposed persistent/COW environment
history needed for durable paragraph checkpoints inside ordinary open groups,
including store ownership, reclamation, hashing, rollout, and validation.

`source_spans_and_provenance.md` specifies the adopted compact source-map,
source-span, derived-provenance, packing, capacity, and validation contract.

`structural_provenance.md` defines the reachability-owned source-registration,
token-position, origin-list, expansion-frame, diagnostic, node, artifact,
retry, and private-revision ownership model which supersedes append-history
retention without changing packed ids or rendered results.

`node_word_arena.md` is the authoritative compact node-word arena document: it
defines the adopted word encoding, generation-tagged identities, sidecar and
survivor ownership, access boundary, hashing, and validation. Do not create a separate
`node_word_layout.md` whose encoding or rollback rules could drift.

`wasm_resource_acquisition.md` specifies the implemented typed, batched
resource state machine and the remaining OpenType rollout, including
required-versus-hint semantics, client-owned distribution, font reuse,
caching, and native parity.

`resource_lifecycle.md` is the normative cross-subsystem contract for resource
keys, request intent, verified acquisition, VFS admission, engine suspension,
candidate ownership, and native/browser scheduling boundaries.

`web_font_bundles.md` specifies the OpenType-first native/WASM font-resource
model: OTF/TTF native containers, WOFF2 browser containers, canonical program
identity, batched acquisition, client-owned distribution, retained HTML asset
reuse, modern `OpenTypePreferred` versus `ClassicTfmExact` layout policy,
positioned OpenType MATH output, and the single linear migration rollout.

`cross_output_fonts.md` is the normative contract for the complete cross-output
font system and the deliberately smaller hosted HTML MVP. It fixes layout
authority, output-specific closures, typed identity, placement, precedence,
ownership, failure, licensing, compatibility migration, and the exact catalog;
all `umber2-nobk` implementation work must cite it.

`html_font_catalog.md` is the implemented machine-auditable inventory and
supported-family statement for that contract's three-entry HTML MVP catalog.

`incremental_html.md` defines long-lived render identity, canonical equality,
typed patch planning and validation, browser application, recovery, resource
ownership, backpressure, disposal, and bounded-session behavior.

`unicode_opentype_shaping.md` specifies rustybuzz text shaping, mapped
TFM-style text in modern mode, shape/break/reshape integration, and the
positioned-math output boundary. `html_output.md` remains the exact current
HTML schema contract and defines its planned fixed-position OpenType text and
math extension.

`etex_primitives.md` is the extension-only e-TeX V2 primitive checklist and
maps each family to its short-reference-manual contract and conformance gate.

`pdftex_primitives.md` pins the pdfTeX 1.40.29 source-level primitive
inventory, records exact-name coverage above TeX82/e-TeX, and defines the
dependency-ordered completeness plan for the PDF engine layer.

`pdf_backend.md` defines the deterministic PDF ledger, detached structural
model, canonical writer, checkpoint identity, and structural/rendering parity
contracts.

`virtual_fonts.md` defines the canonical bounded VF parser, immutable local
font and character-packet model, recursion metadata, authority mapping, and
the acquisition/lowering ownership boundaries.

`pdf_test_architecture.md` defines the lightweight oracle mix and complete
`lopdf` migration inventory for PDF tests, including the minimal Hayro trailer
accessor, stable-identity/cycle/content observations, raw fixture boundary, and
external validator matrix.

`pdftex_font_microtype.md` defines immutable copied/letterspaced/expanded font
identity, expansion and protrusion arithmetic, line-material ownership, margin
enquiries, and the detached `pdf_writer` resource boundary.

`pdftex_graphics_state.md` defines literal modes and expansion timing, typed
graphics-state lowering, color-stack page/form scope, saved positions and
snapping, and the timer/random integration boundary.

`pdftex_navigation.md` defines destination scanners and name trees, outline
hierarchy and actions, article-thread bead lifecycles, object ownership,
diagnostics, reserved codecs, and the typed PDF writer boundary.

`latex_dvi.md` defines the separate LaTeX-DVI and pdfLaTeX engine identities,
their shared extension inventory, pinned format boundaries, output contracts,
and parity tiers.

`umber_vfs.md` defines the implemented host-neutral shared virtual
filesystem, including canonical paths, immutable input layers, generated-file
transactions, typed resource registration, build atomicity, native/WASM
parity, and validation.

`bib.md` defines the implemented pure-Rust in-process bibliography subsystem,
its `bib-*` crate boundaries, exact compatibility target, public API,
processing pipeline, direct upstream-test translation, shared-VFS dependency,
and multi-pass native/WASM composition.

`classic_bibtex_inventory.md` pins the merged classic BibTeX 0.99d Web2C
identity, construct and upstream-test ownership census, committed fixture
manifest, and the hermetic `--area bibtex` regeneration boundary. The reviewed
two-backend architecture and phase exit criteria remain fixed by
`classic_bibtex_bst.md` at commit `c676cfb0`.

`classic_bibtex_bst.md` defines the proposed classic BibTeX backend and `.bst`
stack-language compiler/VM, its separation from the Biber-compatible pipeline,
shared raw-input and orchestration boundaries, compatibility fixtures, bounded
execution model, phased integration, and exit criteria.
