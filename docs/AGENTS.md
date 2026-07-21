# Docs Guidance

Read the repository-level `AGENTS.md` before editing here. Documentation should describe the current fixture workflow: `cargo test --tests` exercises the default native correctness members against committed fixtures, `scripts/check-tools.sh` gates opt-in host tools, and `scripts/regen-fixtures.sh` is the only supported live-reference regeneration entry point.

When documenting tests or parity workflow, point fixture changes to `scripts/regen-fixtures.sh` modes rather than cargo-test environment variables or retired scripts.

`snapshot_performance.md` defines the focused snapshot latency and retained-allocation gate, including its asymptotic budgets and measurement semantics.

`profiling.md` documents the persistent in-process Gentle profiler, its
Samply wrapper, prerequisites, counters, measurement controls, and capture
analysis workflow. Historical measurements belong in Git history or Beads,
not as chronological release receipts in `docs/`.

`testing_policy.md` is forward-looking guidance for test design and placement.
`testing_infrastructure.md` inventories the current test commands, budgets,
fixtures, corpora, and harnesses; update it when those implementation facts
change.

`frozen_format.md` defines the portable format-image container ABI,
compatibility fingerprints, deterministic literal lookup-table representation,
validation and checksum coverage, immutable/job-local split, and migration
from schema 9.

`format_cache.md` defines generated-format cache identity, validated atomic
native entries, corruption recovery, and the browser portability boundary.

`arxiv_corpus.md` is the durable accounting record for source-side limitations
and reproducible entrypoint decisions in the pinned 100-document arXiv sample.
Keep engine failures in Beads rather than classifying them there prematurely.

`arxiv_census/` contains immutable machine-readable 100-row census captures.
Its README records the exact interpretation and provisional-status rules; keep
engine ownership and follow-up work in the linked Beads epic and children.

`incremental_v1.md` fixes the named-boundary schedule, editor-session
retention, edit mapping, pruning, and schedule-relative convergence contract
for the first incremental engine.

`persistent_compile_sessions.md` defines the unified native/WASM compile
session lifecycle that composes typed resource retries with revision-checked
root-buffer patches and retained incremental execution.

`generated_input_stabilization.md` defines the proposed correctness contract
for positive and negative generated-input dependencies, safe `JobStart`
fallback, provisional editor output, bounded off-hot-path fixed-point
stabilization, and later paragraph reuse across unchanged-root rerun passes.

`stepwise_execution.md` defines the owned `tex-exec` run, atomic per-step
snapshot/replay protocol, typed resource sites, lifecycle, cumulative fuel and
cancellation rules, and the migration from whole-attempt retries.

`incremental_memoization.md` defines the changed-document slow path: stable
source alignment plus an ordered accepted-history paragraph replay cursor,
per-paragraph dependency validation, accepted-history-owned shared finished-line
mounts, lazy output provenance, cold-equivalent boundary publication, and
path-separated verification. It deliberately does not use a reverse suffix
hash, hierarchical execution trace, or prepared-hlist fallback tier.

`retained_group_roots.md` specifies the proposed persistent/COW environment
history needed for durable paragraph checkpoints inside ordinary open groups,
including store ownership, reclamation, hashing, rollout, and validation.

`source_spans_and_provenance.md` specifies the adopted compact source-map,
source-span, derived-provenance, packing, capacity, and validation contract.

`node_word_arena.md` is the authoritative compact node-word arena document: it
defines the adopted word encoding, generation-tagged identities, sidecar and
survivor ownership, access boundary, hashing, and validation. Do not create a separate
`node_word_layout.md` whose encoding or rollback rules could drift.

`wasm_resource_acquisition.md` specifies the implemented typed, batched
resource state machine and the remaining OpenType rollout, including
required-versus-hint semantics, client-owned distribution, font reuse,
caching, and native parity.

`web_font_bundles.md` specifies the OpenType-first native/WASM font-resource
model: OTF/TTF native containers, WOFF2 browser containers, canonical program
identity, batched acquisition, client-owned distribution, retained HTML asset
reuse, modern `OpenTypePreferred` versus `ClassicTfmExact` layout policy,
positioned OpenType MATH output, and the single linear migration rollout.

`unicode_opentype_shaping.md` specifies rustybuzz text shaping, mapped
TFM-style text in modern mode, shape/break/reshape integration, and the
positioned-math output boundary. `html_output.md` remains the exact current
HTML schema contract and defines its planned fixed-position OpenType text and
math extension.

`etex_primitives.md` is the extension-only e-TeX V2 primitive checklist and
maps each family to its short-reference-manual contract and conformance gate.

`pdftex_primitives.md` pins the pdfTeX 1.40.27 source-level primitive
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
