# Umber Architecture Refactor Synthesis

## Executive thesis

Umber’s largest safe simplifications come from deleting competing ownership models, not from introducing more shared utility layers. The repository already contains several newer authorities beside older implementations:

- canonical typed TeX control beside the legacy `Executor`;
- typed bibliography entries beside raw/eager/intermediate Biber models;
- canonical distribution data beside native, browser, and publisher copies;
- transactional VFS state beside project-local resource maps;
- canonical oracle streams beside observer, replay, and TRIP-specific transports.

The highest-value strategy is therefore convergence followed by deletion: establish a narrow compatibility adapter, prove externally visible equivalence, then remove the losing lifecycle and its tests.

This synthesis read all 36 authoritative crate reports, the repository `AGENTS.md`, and selected source locations for reconciliation. No builds, tests, formatters, package managers, network tools, or writes were used. `tex-lex` and `tex-expand` were explicitly excluded as refactor targets because they are scheduled for removal; none of their potential deletions is included below.

“Observed” means supported by repository source or dependency/call-site inspection. LOC figures are model estimates of net deletion after migration code is removed. Production/tool and test/support code are estimated separately. A moved algorithm is not counted as deleted.

## Ranked summary

| Rank | Refactor program                                                            | Affected crates                                                                                                                        | Production/tool LOC | Test/support LOC | Confidence  |
| ---: | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------: | ---------------: | ----------- |
|    1 | Make canonical TeX execution the only production engine                     | `tex-command`, `tex-exec`, `tex-incr`, `umber`, `tex-state`, `tex-fonts`, `tex-typeset`, `tex-out`, `umber-wasm`, `tex-command-stream` |        6,000–10,000 |      3,500–7,000 | Medium      |
|    2 | Collapse Biber into one typed semantic worker                               | `bib-input`, `bib-model`, `bib-graph`, `bib-sort`, `bib-label`, `bib-engine`                                                           |           700–1,200 |          400–900 | Medium-high |
|    3 | Make distribution catalog semantics single-owner                            | `umber-distribution`, `umber`, `umber-wasm`, `texlive-wasm-publish`                                                                    |             500–900 |          200–500 | Medium      |
|    4 | Create one project workspace and resource-binding ledger                    | `umber-vfs`, `umber`, `tex-incr`, `bib-engine`, `umber-wasm`                                                                           |             400–800 |          400–800 | Medium-low  |
|    5 | Replace stringly evidence with one typed oracle pipeline                    | `tex-command`, `tex-observe`, `tex-oracle`, `tex-command-stream`, `parity-harness`, `umber`                                            |           650–1,100 |        600–1,200 | Medium      |
|    6 | Unify canonical step, resource, and revision transactions                   | `tex-exec`, `tex-incr`, `umber`, `tex-state`, `tex-out`, `tex-command-stream`, `umber-vfs`                                             |           700–1,200 |          400–800 | Medium      |
|    7 | Establish one closed fixture and corpus authority                           | `fixturegen`, `test-support`, `corpus-manifest`, `corpus-sync`, `parity-harness`, `tex-command-stream`, `tex-oracle`                   |           700–1,300 |        700–1,400 | Medium-low  |
|    8 | Privatize and compact the classic BibTeX runtime                            | `bib-bst`, `bib-engine`, `bib-input`                                                                                                   |             500–900 |          250–600 | Medium      |
|    9 | Make bibliography output a single closed protocol                           | `bib-output`, `bib-model`, `bib-engine`, `umber`, `umber-wasm`                                                                         |             300–550 |          100–250 | Medium      |
|   10 | Use one DVI body compiler and page currency                                 | `tex-out`, `tex-exec`, `umber`, `tex-incr`, `tex-command-stream`                                                                       |             350–650 |          100–250 | Medium      |
|   11 | Move detached content and operational memo ownership out of aggregate state | `tex-state`, `tex-exec`, `tex-incr`, `umber`, `tex-out`                                                                                |             550–950 |          250–600 | Low-medium  |
|   12 | Use one PDF graph cursor and paint program                                  | `tex-out`, `umber`, `tex-exec`, `tex-fonts`                                                                                            |             450–750 |          100–220 | Medium-low  |
|   13 | Consolidate node schema and source-independent traversal                    | `tex-state`, `tex-typeset`, `tex-exec`, `tex-out`, `tex-command-stream`                                                                |             450–850 |          150–400 | Medium-low  |
|   14 | Separate native acquisition from format-cache policy                        | `umber-fetch`, `umber`, `umber-distribution`                                                                                           |             400–750 |          250–600 | Medium      |
|   15 | Merge shaping into the validated font owner                                 | `tex-fonts`, `tex-shape`, `tex-exec`, `tex-typeset`, `tex-out`, `umber`                                                                |             250–450 |          250–500 | Medium-high |

The rank is a blend of deletion potential, conceptual leverage, evidence strength, feasibility, and the amount of externally visible behavior that must be preserved. It is not the execution order.

## Portfolio estimate

The figures below are scenario totals after removing overlaps. They are not arithmetic sums of every individual endpoint.

| Scenario     | Production/tool reduction | Test/support reduction | Combined reduction |
| ------------ | ------------------------: | ---------------------: | -----------------: |
| Conservative |              9,500–11,500 |            5,000–6,500 |      14,500–18,000 |
| Likely       |             13,500–16,500 |           7,500–10,500 |      21,000–27,000 |
| Upside       |             18,000–22,000 |          11,000–15,000 |      29,000–37,000 |

The conservative case assumes the legacy TeX engine is removed but the deepest state-graph and fixture changes are deferred. The likely case includes the first ten programs plus the high-confidence parts of the remaining work. The upside case includes the node/content substrate reductions and all host-tool consolidation.

Double-counting rules used here are strict:

- Legacy `Executor` and old assignment/font/math paths are counted only in rank 1.
- Canonical loop consolidation is counted only in rank 6.
- DVI writer duplication is counted only in rank 10.
- PDF graph and paint duplication is counted only in rank 12.
- Font/shaping boundary deletion is counted only in rank 15, excluding legacy font execution already counted in rank 1.
- Distribution model duplication is counted in rank 3; native acquisition and cache mechanics are counted in rank 14.
- Evidence transport and typed observation are counted in rank 5; fixture staging and publication transactions are counted in rank 7.
- The nine delegated large-crate reviews were used as component evidence, but their entire crate sizes are not treated as removable.

## Ranked refactor programs

### 1. Make canonical TeX execution the only production engine

Affected crates: `tex-command`, `tex-exec`, `tex-incr`, `umber`, `tex-state`, `tex-fonts`, `tex-typeset`, `tex-out`, `umber-wasm`, and `tex-command-stream`.

**Current design — implemented.** `tex-command` owns source delivery, expansion, and operand scanning, while `tex-exec::MainControl` is the sole stomach transition machine. Native, virtual, incremental, format-loaded, and WebAssembly sessions all drive that command-owned path. The retired executor, lexer/expander representations, assignment/alignment/math scanner fronts, replay adapters, and compatibility-only tests are absent from the workspace and tex-exec source graph.

**End state.** `MainControl` is the only production transition machine. `EngineSession` is its host-neutral session façade. The retired `Executor`, executor-only raw dispatch and assignment machinery, duplicate font/materialization paths, and duplicate checkpoint plumbing are absent. Shared execution code remains.

The completed cutover also deleted `tex-lex` and `tex-expand`; their required behavior now belongs to `tex-command` rather than a compatibility façade.

**Estimated net reduction.** 6,000–10,000 production/tool LOC and 3,500–7,000 test/support LOC. Confidence: medium. The source evidence for duplication is strong, but TeX’s observable behavior makes the deletion surface unusually risky.

**Why functionality can be preserved.** The engine path owns typed command state, resource needs, checkpoints, and committed transitions. Preservation requires equality of DVI/PDF output, diagnostics, tracing, source locations, fuel behavior, format bytes, state identities, resource suspension ordering, and incremental replay—not merely successful compilation.

**Migration status.** Runtime callers use canonical resource and checkpoint capabilities, and the executor-only assignment, font, math, alignment, checkpoint, output, and diagnostic fronts have been deleted. Rollback now means reverting the cutover commits; there is intentionally no session selector that can revive the old engine.

**Equivalence gates and risks.** Required gates include TRIP/e-TRIP transcripts, canonical semantic streams, byte-exact DVI where applicable, normalized and rendered PDF parity, format dump/restore, malformed input diagnostics, fuel/RSS bounds, resource unavailable/fulfilled replay, and cold-versus-incremental equality. The major risks are missing primitives, allocation-order changes, diagnostic timing, resource suspension at different boundaries, and hidden external users of `Executor`.

**Dependencies and conflicts.** Rank 5 should precede this so divergences are explainable. Rank 6 follows this and must not claim the same deleted code. Ranks 9, 10, 12, and 15 should consume canonical execution rather than extend the legacy path. Ranks 11 and 13 should avoid changing the state substrate until the canonical path is the accepted baseline.

### 2. Collapse Biber into one typed semantic worker

Affected crates: `bib-input`, `bib-model`, `bib-graph`, `bib-sort`, `bib-label`, and `bib-engine`.

**Prior design — observed.** The normal bibliography session parsed raw entries, converted them into typed `Entry` values, sent them through a public `GraphProcessor`, rebuilt processed sections, added label sources, constructed a temporary initial section, created a `DataList`, and froze another section. `bib-label`, `bib-sort`, and `bib-graph` each had only one production caller: `bib-engine`.

**End state.** The Biber path becomes one engine-private worker:

1. raw parser output;
2. one typed `EntryEditor` lowering;
3. one indexed relationship/inheritance pass;
4. one sort/label plan;
5. one frozen document.

`GraphInput`, `GraphOutput`, stage contexts, temporary processed sections, public sort/label transport, and duplicate entry rebuilding disappear. The algorithms remain, but their crate-level boundaries and intermediate representations do not. `bib-unicode` may remain the pinned collation implementation; its empty stage contexts are not preserved.

**Estimated net reduction.** 700–1,200 production LOC and 400–900 test LOC. Confidence: medium-high. The local dependency graph is narrow and the current session already exposes the intended sequence, but public crate compatibility must be resolved.

**Why functionality can be preserved.** The new worker must preserve duplicate-key selection, case normalization, crossref/xdata inheritance, relationship ordering, labels, sort stability, source provenance, diagnostics, and all output projections. It must not activate currently ignored configuration semantics merely because options are made easier to pass.

**Migration and rollback.**

1. Introduce an engine-private worker behind the existing session API.
2. Make it wrap the current `EntryBuilder`, `GraphProcessor`, sort, and label operations while emitting stage snapshots.
3. Replace the temporary section pipeline with one internal document builder.
4. Keep the old public crates as delegating compatibility modules until all local consumers disappear.
5. Remove the `bib-graph`, `bib-sort`, and `bib-label` package boundaries only after upstream corpus tests and downstream API checks pass.

Rollback now means reverting the cutover commits; no session selector or old stage transport remains.

**Migration status.** `bib-engine/src/biber` now owns the sole `EntryEditor`, indexed relationship/inheritance pass, label and sort plan, and final document builder. `GraphInput`, `GraphOutput`, all three empty stage contexts, the temporary processed section, and the `bib-graph`, `bib-sort`, and `bib-label` packages have been deleted. The session API and pinned Unicode boundary are unchanged; configuration semantics that were inactive before the migration remain inactive.

**Gates and risks.** Run the full BibTeX/Biber compatibility corpus, including the 51 upstream modules and 1,275 assertion identifiers, BBL/XML/DOT output, crossref and xdata cases, duplicate entries, malformed values, name handling, labels, and diagnostics. The main risks are external consumers of currently public crates, accidental option activation, Unicode collation changes, and losing provenance during entry editing.

**Dependencies and conflicts.** Rank 8 should establish the raw database contract that both classic and Biber paths consume. Rank 9 can then consolidate the output boundary. The estimate excludes output serializer deletion and classic VM deletion.

### 3. Make distribution catalog semantics single-owner

Affected crates: `umber-distribution`, `umber`, `umber-wasm`, and `tools/texlive-wasm-publish`.

**Current design — observed.** `umber-distribution` contains both a sharded root representation and a monolithic manifest representation at [`crates/umber-distribution/src/manifest.rs:22`](/home/phulin/umber/crates/umber-distribution/src/manifest.rs:22) and [`crates/umber-distribution/src/manifest.rs:62`](/home/phulin/umber/crates/umber-distribution/src/manifest.rs:62). It also owns a typed selection function at [`crates/umber-distribution/src/selection.rs:539`](/home/phulin/umber/crates/umber-distribution/src/selection.rs:539). The publisher has a parallel sharded wire model, native `umber` still performs its own probing and acquisition planning, and JavaScript has separate schema and resolver logic in [`crates/umber-wasm/js/manifest-schema.js:22`](/home/phulin/umber/crates/umber-wasm/js/manifest-schema.js:22) and [`crates/umber-wasm/js/manifest-resolver.js:275`](/home/phulin/umber/crates/umber-wasm/js/manifest-resolver.js:275).

**End state.** `umber-distribution` owns the pure catalog model, schema compatibility readers, request keys, selection order, dependency hints, misses, and validation. The publisher emits that model directly. Native code maps TeX-specific requests to the shared selection plan and retains only local-file and cache policy. Browser JavaScript becomes a thin compatibility adapter over the same plan or over a WASM-exported resolver; it no longer maintains an independent manifest schema.

Legacy schema readers remain where required, but duplicate internal models and serializers disappear.

**Estimated net reduction.** 500–900 production/tool LOC and 200–500 test LOC. Confidence: medium.

**Preservation argument.** Manifest field order, omission rules, schema versions, shard indexes, object hashes, font identities, request order, breadth-first dependency hints, and miss behavior are wire contracts. The recommendation explicitly preserves them rather than replacing them with a new generic schema.

**Migration and rollback.**

1. Freeze golden root, shard, and selection fixtures.
2. Add publisher adapters that construct canonical distribution values while retaining the old writer.
3. Migrate native selection and browser resolution to the shared `Selection` result.
4. Remove duplicate JavaScript and publisher parsing/validation.
5. Retain legacy readers for previously published catalog versions.

Rollback is a per-consumer parser switch; old manifests remain readable throughout.

**Gates and risks.** Compare serialized roots and shards byte-for-byte, selection order and misses, dependency-hint order, browser resolver behavior, native offline snapshots, and malformed-schema errors. Risks include JavaScript/Rust numeric and string differences, schema compatibility, and accidentally moving local-first policy into the pure catalog crate.

**Dependencies and conflicts.** Rank 14 consumes this pure plan but owns native storage and cache behavior. Rank 4 consumes the same request/resource identities. Their estimates exclude catalog model deletion.

### 4. Create one project workspace and resource-binding ledger

Affected crates: `umber-vfs`, `umber`, `tex-incr`, `bib-engine`, and `umber-wasm`.

**Implementation — completed by `umber2-fjfh.3`.** `umber-vfs::ProjectWorkspace` owns layered storage, validated limits, the typed `ResourceLedger`, immutable snapshots, and the pending generated transaction. Compile attempts borrow the ledger beside the disjoint build overlay; incremental execution, bibliography, and WASM receive snapshots or the existing session adapter. The project response cache, per-attempt resolver maps, and reverse resolved-path map have been deleted.

**End state.** One `ProjectWorkspace` owns layered storage, the path/generation
index, typed `ResourceLedger`, pending overlay, and limit/accounting policy.

Compilation, incremental editing, bibliography, and WASM sessions receive narrow workspace views instead of cloning VFS state and rebuilding resource maps. `umber-vfs` remains the low-level implementation owner; the project-level duplicate ledgers disappear.

**Preservation argument.** The ledger must retain local-file precedence, aliases, generated-file provenance, immutable bindings, retry behavior, snapshot retention, and limits. The change is an ownership consolidation, not a change to resource lookup policy.

**Migration status.** Complete. Focused parity tests protected precedence,
aliases, bindings, retries, snapshots, generations, provenance, and limits as
the old maps were removed. No compatibility façade or production switch remains.

**Gates and risks.** Test generated-file paths, local shadowing, font/image/input acquisition, retry and limit errors, snapshot retention, incremental generations, LaTeX/Biber resource provenance, and WASM behavior. Risks include changing lookup precedence, retaining too much generation state, and introducing borrow/lifetime complexity.

**Dependencies and conflicts.** Rank 3 supplies distribution request identities; rank 14 supplies native acquisition. Rank 6 owns execution-time resource suspension and must not also claim workspace storage deletion.

### 5. Replace stringly evidence with one typed oracle pipeline

Affected crates: `tex-command`, `tex-observe`, `tex-oracle`, `tex-command-stream`, `tools/parity-harness`, and `umber`.

**Current design — observed.** `tex-command` records observation fields in several stringly or loosely structured forms, including [`crates/tex-command/src/observation/mod.rs:569`](/home/phulin/umber/crates/tex-command/src/observation/mod.rs:569). `tex-observe` reparses locations and reconstructs alignment state in a second shadow machine at [`crates/tex-observe/src/translation.rs:237`](/home/phulin/umber/crates/tex-observe/src/translation.rs:237) and [`crates/tex-observe/src/translation.rs:454`](/home/phulin/umber/crates/tex-observe/src/translation.rs:454), even though source registration is already owned by command input at [`crates/tex-command/src/input/source.rs:305`](/home/phulin/umber/crates/tex-command/src/input/source.rs:305).

`tex-oracle` now owns canonical JSONL streams and the canonical detached
semantic/geometry `OracleBundle`, including the byte-compatible evidence
codec, validation, and resource limits. `tex-observe` only projects live
engine records into that owned bundle; format construction, replay, and TRIP
parity bind its independently sequenced channels through the same oracle API.
Command-owned source/alignment context and typed scanner, mutation, and effect
values now project directly; the observer-side shadow machines and structured
string decoders have been deleted.

**End state.** The engine emits a typed neutral observation record containing source identity, source location, command identity, alignment identity, semantic effect, and geometry references. `tex-observe` becomes a thin adapter from engine-owned records to oracle values. `tex-oracle` owns the detached evidence codec, canonical JSONL transport, normalization, sequence checks, and stream identity. Replay, TRIP capture, and live parity all consume one bundle format.

**Estimated net reduction.** 650–1,100 production/tool LOC and 600–1,200 test/support LOC. Confidence: medium.

**Preservation argument.** The typed record must retain event sequence, source-ID reuse, caret-decoded locations, pseudo-source names, alignment nesting, geometry schema, manifest identity, nonfallible observer behavior, and current size limits. The current canonical transport already validates these properties at [`crates/tex-oracle/src/transport.rs:33`](/home/phulin/umber/crates/tex-oracle/src/transport.rs:33).

**Migration and rollback.**

1. Add typed producer fields while continuing to emit the old records.
2. Compare old and new translations for every live observation. _(Completed for source identity and alignment nesting: the complete committed TeX82 command suite is byte-equivalent after deleting both projection-side stacks.)_
3. Move detached evidence encoding behind the oracle crate while retaining the old decoder. _(Completed: the existing schema-2 `UMBREVID` bytes and all limits moved unchanged.)_
4. Migrate command-stream, TRIP, and parity consumers. _(Completed through the oracle-owned bundle and typed effect projection.)_
5. Delete shadow source/alignment state and string conversion only after byte-level stream parity. _(Completed after the committed TeX82 semantic fixtures remained byte-identical.)_

Rollback retains the old translator and codec readers behind a schema/profile switch.

**Gates and risks.** Require canonical JSONL byte equality, detached-evidence byte equality, source-location and alignment differential tests, TRIP phase parity, geometry parity, malformed-stream rejection, and allocation/size-limit tests. The main risk is changing the evidence schema while believing only an internal type changed.

**Dependencies and conflicts.** This should precede rank 1 and supports rank 7. It does not include fixture staging or transaction deletion; those belong to rank 7.

### 6. Unify canonical step, resource, and revision transactions

Affected crates: `tex-exec`, `tex-incr`, `umber`, `tex-state`, `tex-out`, `tex-command-stream`, and `umber-vfs`.

**Current design — implemented.** `tex-exec::CanonicalStepRunner` is the one bounded native and incremental step protocol. It returns `Progress`, `ResourceNeed`, `Committed`, `Completed`, or `Failed`; `OutputLedger` owns named-checkpoint capture, exact resource registration, authoritative absence, and host-visible suspension accounting. `umber::EngineSession` and `tex-incr::RevisionCandidate` retain only lifecycle and host policy. Virtual compilation delegates to the revision candidate, while command-stream observation delegates to the engine session, so neither has a replay loop or checkpoint side channel.

**End state.** The canonical transaction protocol is split into two explicit responsibilities:

- `StepRunner`: advances one bounded step and returns `Progress`, `ResourceNeed`, `Committed`, `Completed`, or `Failed`;
- `RevisionTransaction`/`OutputLedger`: owns checkpoint capture, state publication, artifact ordering, resource fulfillment, and rollback.

Frontends retain policy—blocking versus asynchronous resource resolution, cold versus incremental mode, diagnostic verbosity—but do not implement their own loops.

**Estimated net reduction.** 700–1,200 production LOC and 400–800 test LOC. Confidence: medium. This estimate excludes legacy engine deletion from rank 1 and VFS storage deletion from rank 4.

**Preservation argument.** The protocol must retain command fuel semantics, suspension boundaries, answered-resource replay, checkpoint schema, cancellation, artifact order, and candidate acceptance/rejection behavior.

**Migration and rollback.**

1. Wrap the existing canonical control in the new result enum without changing transitions.
2. Adapt `umber` first, then `tex-incr`, then command-stream replay.
3. Move output and checkpoint ownership into the ledger while preserving current wire formats.
4. Remove duplicated loops and local side channels.

The public whole-run and advance-until-waiting methods are thin policy adapters over the shared runner; there is no alternate engine loop behind them.

**Gates and risks.** Compare one-step and whole-run semantic events, DVI/PDF/effect output, checkpoints, state hashes, fulfilled and unavailable resources, cancellation, fuel exhaustion, and cold-versus-incremental results. Risks include committing a state before all effects are captured and changing the exact point at which a resource request suspends execution.

**Dependencies and conflicts.** Rank 1 is a prerequisite. Rank 5 supplies the evidence needed to diagnose differences. Rank 4 supplies workspace-level resource state. Rank 11 must not move memo ownership into this transaction unless the boundaries are explicit.

### 7. Establish one closed fixture and corpus authority

Affected crates: `tools/fixturegen`, `crates/test-support`, `crates/corpus-manifest`, `tools/parity-harness`, `tex-command-stream`, and `tex-oracle`.

**Implemented design.** `fixturegen` owns the single `CasePlan`, `ArtifactSpec`, and `AtomicCaseTransaction`. Ordinary generation, layout/PDF migration, externally staged cohorts, corpus synchronization, parity reference-DVI publication, and command-semantic batch publication all prepare complete byte inventories before that transaction mutates an authority. `corpus-manifest::Entry` is the one validated support/document record. The standalone `corpus-sync` workspace and the parity/command-stream replacement protocols are gone. `test-support` loads and asserts committed cases without publication ownership.

**End state.** `fixturegen` owns one `CasePlan`, one `ArtifactSpec`, and one `AtomicCaseTransaction`. `corpus-manifest` becomes one validated entry model or a private module consumed by that tool. `corpus-sync` is folded into the host tooling rather than remaining a standalone workspace. `parity-harness` remains an execution/comparison library, not a filesystem mutator. `test-support` retains runtime assertions and case loading but no competing publication authority.

**Estimated net reduction.** 700–1,300 production/tool LOC and 700–1,400 test/support LOC. Confidence: medium-low because fixture tools mutate repository-controlled data when executed, even though this review did not.

**Preservation argument.** The transaction must preserve atomic replacement, backup/recovery behavior, SHA-256 locks, offline mode, support-before-document ordering, file ordering, shell exit statuses, status messages, and failure cleanup.

**Migration and rollback.**

1. Define the plan and manifest model as read-only adapters over current formats.
2. Route main, layout, PDF, and cohort generation through the plan while retaining old transaction implementations.
3. Fold corpus synchronization into the same host command and reduce shell scripts to thin launchers.
4. Migrate parity and command-stream publication.
5. Delete old transaction and manifest types only after fixture-tree digests are unchanged.

Rollback remains transaction-level: pre-commit failures restore every authority, incomplete restoration retains named backups, and post-commit cleanup failures retain the complete installed tree plus its owned transaction root for retry.

**Gates and risks.** Compare complete fixture-tree hashes, exact bytes, manifest ordering, atomic failure/recovery behavior, offline locator fallback, lock verification, TRIP/DVI/PDF publication, and shell status output. The major risk is accidental deletion or partial publication in a tool that is normally trusted to modify fixtures.

**Dependencies and conflicts.** Rank 5 should stabilize evidence bundles first. This program does not count evidence codec deletion or distribution catalog deletion.

### 8. Privatize and compact the classic BibTeX runtime

Affected crates: `bib-bst`, `bib-engine`, and `bib-input`.

**Former design — observed.** Raw BibTeX data originated in `bib-input`, classic processing created a second public-facing database representation in `bib-engine`, and the single-consumer `bib-bst` package owned the program model while cache and pool ownership was divided across the boundary.

**End state.** Move the classic runtime behind a private `bib-engine::classic_style` boundary. Retain the raw parser as the sole parse result, then use one compact database/read arena, one string-pool ledger, one bounded cache, one builtin registry, and direct callable instructions. The algorithmic VM remains recognizable, but the public `bib-bst` boundary, duplicate database, synthetic callable functions, and parallel pool/cache ownership disappear.

Moved classic algorithms are not counted as deleted; only duplicate representations and boundaries are.

**Estimated net reduction.** 500–900 production LOC and 250–600 test LOC. Confidence: medium.

**Preservation argument.** Classic BibTeX compatibility requires exact string evaluation, `.aux` and `.bst` behavior, pool identifiers, trace output, error timing, nested function behavior, limits, and output bytes. This is not an opportunity to replace the VM with a more modern but semantically different evaluator.

**Migration and rollback.**

1. Re-export the existing `bib-bst` API from a private engine module.
2. Add the compact database and pool behind the existing VM interfaces.
3. Differentially compare classic runs, including malformed styles and tracing.
4. Move caches and builtin resolution.
5. Delete the package boundary and obsolete public representations.

A compatibility crate façade can remain temporarily if external publication obligations require it.

**Migration status.** Complete. `bib-engine::classic_style` now privately owns the moved lexer/compiler/VM algorithms, compact immutable `READ` state, the job string-pool ledger, and the sole classic cache owner. Classic execution consumes `bib-input::RawBibDatabase` directly, callable instructions carry builtin/function/variable targets without synthetic wrapper functions, and the builtin name table has one definition. The public `bib-bst` package and the standalone `classic_database`/`classic_vm` module boundaries have been deleted. Moved algorithms are excluded from the deletion total.

Rollback is a single revert of the migration commit; there is no retained compatibility façade or dual execution path.

**Gates and risks.** Use classic `.bbl`, `.blg`, `.aux`, trace, malformed-style, nested-function, memory-limit, and string-pool parity cases. Risks are external users of `bib-bst`, subtle string-pool identity changes, and error/trace ordering.

**Dependencies and conflicts.** Rank 2 should first establish the raw-entry boundary shared by classic and Biber. The cache and pool savings here are not shared-cache savings from rank 14.

### 9. Make bibliography output a single closed protocol

Affected crates: `bib-output`, `bib-model`, `bib-engine`, `umber`, and `umber-wasm`.

**Current design — observed.** The model exposes a document/output contract at [`crates/bib-model/src/document.rs:470`](/home/phulin/umber/crates/bib-model/src/document.rs:470), while `bib-output` adds a serializer trait, router, multiple failure wrappers, and format-specific finalizers through [`crates/bib-output/src/lib.rs:24`](/home/phulin/umber/crates/bib-output/src/lib.rs:24), [`crates/bib-output/src/lib.rs:45`](/home/phulin/umber/crates/bib-output/src/lib.rs:45), and [`crates/bib-output/src/router.rs:98`](/home/phulin/umber/crates/bib-output/src/router.rs:98). BBL, BibTeX, DOT, and XML each repeat selection/finalization logic. Format dispatch is repeated in engine and WASM callers.

**End state.** Use one closed `OutputPlan` over one frozen document projection. A single router owns format selection and common failure handling. Format modules remain separate because their wire formats genuinely differ, but they receive the same selected entries, sections, ordering, provenance, and output policy. Open-ended serializer traits and repeated finalization/selection loops disappear.

**Estimated net reduction.** 300–550 production/tool LOC and 100–250 test LOC. Confidence: medium.

**Preservation argument.** The plan must preserve BBL first-occurrence behavior, BibTeX section-local deduplication, DOT graph ordering, XML dialect structure, output naming, failure classification, and exact bytes.

**Migration and rollback.**

1. Add the common document projection while keeping current serializers.
2. Route one format at a time through the projection.
3. Keep old public serializer names as thin delegating wrappers.
4. Consolidate error and finalization handling.
5. Remove duplicated selection loops after all four formats compare identically.

Rollback is per format: any serializer can continue using the old projection until its output gate passes.

**Migration status.** Complete. `bib-output::OutputPlan` is the single frozen projection and policy boundary for BBL, BibTeX, DOT, BibLaTeXML, and BBLXML. `OutputRouter` owns compatibility validation, dispatch, newline/encoding finalization, byte limits, and one typed failure. The former open serializer trait and four independent failure objects are deleted; the old named serializer structs remain only as thin forwarding entry points. Engine, tool, native, and WASM callers continue to use the same closed request enum and generated-file contract.

Rollback is a single revert of the output-protocol migration; there is no parallel production router.

**Gates and risks.** Require byte-exact BBL/BibTeX/DOT/XML fixtures, generated-file naming, empty-section behavior, duplicate handling, error diagnostics, and WASM output parity. Risks are format-specific omissions hidden by a common projection and accidental changes to output ordering.

**Dependencies and conflicts.** Rank 2 supplies the single typed document; rank 8 supplies classic compatibility. This estimate excludes model-entry conversion and graph deletion already counted in rank 2.

### 10. Use one DVI body compiler and page currency

Affected crates: `tex-out`, `tex-exec`, `umber`, `tex-incr`, and `tex-command-stream`.

**Current design — implemented.** `tex-out` owns one private `DviBodyCompiler` and one private `DviFileWriter`. Owned artifacts, serialized artifact streams, live `tex-exec` shipout events, incremental page publication, and coordinate inspection adapt to the same explicit-frame compiler and use `DviPagePlan` as the detached page currency. The file writer alone owns preamble/postamble framing, backpointers, cross-page font definitions, maxima, page counts, and bounded flushing. The former recursive owned traversal and separate page-extent pass are deleted.

**End state.** Live execution, owned page plans, incremental publication, and inspection use thin adapters over the same movement, framing, font-definition, and special-command implementation. The design remains one-pass and streaming: live shipout compiles scalar events as they arrive, serialized artifacts decode one node list at a time, and the file writer retains at most one encoded page.

**Estimated net reduction.** 350–650 production LOC and 100–250 test LOC. Confidence: medium.

**Preservation argument.** DVI movement compression, preamble/postamble fields, page counts, font definitions, specials, coordinate movement, and streaming behavior are externally observable.

**Migration status.** Complete. Direct shipout retains its simultaneous artifact/DVI walk, owned and serialized pages compile to the same plan representation, and incremental and inspection consumers retain their existing public adapters. There is no alternate production traversal or output-policy switch.

**Gates and risks.** Require byte-exact DVI oracle parity, disassembler coordinates, page hashes, font-definition ordering, incremental output, and memory/streaming bounds. Risks are movement optimization changes and page-origin or postamble timing.

**Dependencies and conflicts.** Rank 1 should provide the canonical page material. Rank 15 must settle font identity before font-definition duplication is removed. The estimate excludes legacy shipout deletion already included in rank 1.

### 11. Move detached content and operational memo ownership out of aggregate state

Affected crates: `tex-state`, `tex-exec`, `tex-incr`, `umber`, and `tex-out`.

**Current design — observed.** `tex-state::Universe` embeds operational pure-memo state at [`crates/tex-state/src/universe.rs:1210`](/home/phulin/umber/crates/tex-state/src/universe.rs:1210) and exposes a large forwarding surface around [`crates/tex-state/src/universe.rs:2150`](/home/phulin/umber/crates/tex-state/src/universe.rs:2150). `tex-incr::Session` owns another memo runtime at [`crates/tex-incr/src/lib.rs:892`](/home/phulin/umber/crates/tex-incr/src/lib.rs:892) and transfers it through candidate setup at [`crates/tex-incr/src/lib.rs:2959`](/home/phulin/umber/crates/tex-incr/src/lib.rs:2959). Memo, format, PDF, and page transport also use nested detached envelopes.

**End state.** Separate operational memo service ownership from immutable aggregate engine state. Introduce one validated detached content graph/envelope used by format, memo, PDF, and page transport, while keeping distinct state hashes, content hashes, and memo-integrity identities. `Universe` receives a narrow memo capability; incremental sessions own retention and acceptance policy.

This is not a recommendation to turn all state into a generic object graph.

**Estimated net reduction.** 550–950 production LOC and 250–600 test LOC. Confidence: low-medium.

**Preservation argument.** Format schema bytes, memo hit/miss ordering, malformed-payload rejection, provenance, rollback, forks, page/paragraph memo behavior, PDF envelopes, and state identity must remain unchanged.

**Migration and rollback.**

1. Add a detached graph validator behind existing format and memo APIs.
2. Keep current envelopes as adapters and dual-read both representations.
3. Move operational memo calls behind an execution-service capability.
4. Migrate incremental acceptance and retention.
5. Delete aggregate forwarding and nested envelopes only after all readers agree.

Rollback retains old envelope readers and the `Universe`-backed memo implementation.

**Gates and risks.** Test memo-disabled execution, paragraph and page memo hits, failed-candidate rollback, state hashes, format loading, PDF output, malformed payloads, budgets, and cold-versus-incremental parity. Risks include borrow complexity, accidentally moving provenance out of `tex-state`, and retaining mutable state in a supposedly immutable graph.

**Dependencies and conflicts.** Rank 1 and rank 6 are prerequisites. Rank 13 must be kept distinct: it concerns node schema/traversal, not memo ownership. This program should be stopped if the detached graph adds more code than it removes.

### 12. Use one PDF graph cursor and paint program

Affected crates: `tex-out`, `umber`, `tex-exec`, and `tex-fonts`.

**Current design — observed.** `PdfDocument::semantic_hash` is exposed at [`crates/tex-out/src/pdf.rs:753`](/home/phulin/umber/crates/tex-out/src/pdf.rs:753), with separate object and value walks at [`crates/tex-out/src/pdf.rs:1358`](/home/phulin/umber/crates/tex-out/src/pdf.rs:1358) and [`crates/tex-out/src/pdf.rs:1647`](/home/phulin/umber/crates/tex-out/src/pdf.rs:1647). Validation has another traversal at [`crates/tex-out/src/pdf.rs:1039`](/home/phulin/umber/crates/tex-out/src/pdf.rs:1039), while serialization repeats preflight and recursive emission at [`crates/tex-out/src/pdf/serialize.rs:483`](/home/phulin/umber/crates/tex-out/src/pdf/serialize.rs:483) and [`crates/tex-out/src/pdf/serialize.rs:805`](/home/phulin/umber/crates/tex-out/src/pdf/serialize.rs:805). Page-content lowering also has compact and ordered paths beginning at [`crates/tex-out/src/pdf.rs:144`](/home/phulin/umber/crates/tex-out/src/pdf.rs:144) and [`crates/tex-out/src/pdf.rs:171`](/home/phulin/umber/crates/tex-out/src/pdf.rs:171).

**End state.** Introduce one private `PdfGraphCursor`/`PdfGraphView` for validation, semantic hashing, preflight, and serialization. Introduce one internal `PdfPaintProgram` with explicit compact and ordered policies. Page/form coordinates, resource dictionaries, annotations, images, object-stream policy, and allocation policy remain explicit.

**Estimated net reduction.** 450–750 production LOC and 100–220 test LOC. Confidence: medium-low because PDF allocation and byte order are sensitive.

**Preservation argument.** Preserve object IDs and allocation order, semantic hashes, dictionary ordering, stream lengths, form-local coordinates, page resources, font resources, images, compression, and rendered output.

**Migration and rollback.**

1. Route semantic hashing through the cursor first.
2. Route validation and serializer preflight through it.
3. Build the paint program behind both current page-content entry points.
4. Compare bytes and rendered pages.
5. Delete duplicate walks and policy reconstruction.

Retain the current serializers as fallback visitors until byte parity is established.

**Gates and risks.** Require normalized PDF structure, byte parity where committed, rendered-page parity, object-order checks, forms, annotations, images, fonts, and compressed streams. Risks include changed object allocation, error timing, and resource dictionary order.

**Dependencies and conflicts.** Rank 1 supplies canonical page effects; rank 10 supplies DVI-side page currency. Rank 11’s detached content work must not be counted again as PDF graph deletion.

**Migration status.** Complete. `tex-out` now owns one private `PdfGraphView`
and nested-value cursor used by validation, semantic hashing, serializer
preflight, ordinary serialization, and object-stream selection. One private
`PdfPaintProgram` owns compact and ordered page/form policies and the shared
graphics/text-state interpreter. `umber` passes the canonical operation stream
directly to either policy instead of reconstructing rectangle and text lists.
Allocation, coordinate conversion, resource construction, annotations, images,
fonts, object streams, and compression remain explicit in their existing
owners.

### 13. Consolidate node schema and source-independent traversal

Affected crates: `tex-state`, `tex-typeset`, `tex-exec`, `tex-out`, and `tex-command-stream`.

**Current design — observed.** The node model is repeated across owned nodes, references, storage, copying, semantic views, handles, and format transport beginning at [`crates/tex-state/src/node.rs:12`](/home/phulin/umber/crates/tex-state/src/node.rs:12). Storage has a separate arena abstraction at [`crates/tex-state/src/node_arena/arena.rs:62`](/home/phulin/umber/crates/tex-state/src/node_arena/arena.rs:62). `tex-typeset` has independent cursor, packing, width, and physical-node paths, including [`crates/tex-typeset/src/packing.rs:363`](/home/phulin/umber/crates/tex-typeset/src/packing.rs:363) and [`crates/tex-typeset/src/packing.rs:477`](/home/phulin/umber/crates/tex-typeset/src/packing.rs:477).

**End state.** Define one declarative node schema containing categories, fields, width/height/depth behavior, and traversal metadata. Use it to centralize exhaustive matches and provide one source-independent `NodeCursor` with semantic, packed, and physical views. Keep distinct storage arenas and semantic/physical meanings initially; do not merge `NodeArena` and `SurvivorArena` as part of this estimate.

**Estimated net reduction.** 450–850 production LOC and 150–400 test LOC. Confidence: medium-low. A schema table or generated visitor is valuable only if it deletes existing matches; a new code-generation framework that adds scaffolding is a failure.

**Preservation argument.** Preserve node serialization, line-breaking widths, packing, math and alignment structures, copy semantics, source spans, page traversal, DVI/PDF lowering, and performance characteristics.

**Migration and rollback.**

1. Inventory repeated node matches and define a private schema behind existing enums.
2. Replace width and packing visitors first.
3. Replace semantic/physical cursor duplication.
4. Keep old APIs as generated or delegating wrappers.
5. Delete duplicate matches only after output and state-format parity.

Rollback is possible by retaining the existing visitors while the schema feeds only differential checks.

**Gates and risks.** Require line-break, packing, math, alignment, node-copy, state-format, DVI, PDF, and semantic-event parity. The principal risk is an abstraction that increases compile-time or source size while hiding behavior.

**Dependencies and conflicts.** Rank 1 is required. Rank 11’s content graph work must not include node-schema savings, and vice versa. This is deliberately ranked below output and execution convergence because it is the most likely program to add scaffolding if poorly scoped.

### 14. Separate native acquisition from format-cache policy

Affected crates: `umber-fetch`, `umber`, and `umber-distribution`.

**Current design — implemented.** `umber-fetch` owns one bounded `BlobStore`
and `VerifiedBlobSpec`, backed by one anchored per-key lock, quarantine, digest,
and atomic-publication implementation. Its store-owning `DistributionClient`
acquires verified manifest and object bytes without exposing caller-managed
cache publication. `umber` owns format identity, closure and local-first
selection, compatibility metadata, opaque construction evidence, and full
`Universe` validation. `umber-distribution` remains dependency-free and I/O-free.

The shared store writes `blobs-v1` entries and compatibility-reads the former
`objects`, `manifests`, and `formats-v2` layouts. A successfully verified old
entry is republished through the new substrate, so upgrades warm-migrate rather
than invalidate persistent data.

**Implemented line-count change.** See issue `umber2-fjfh.13` for the measured
production and test/support receipt; moved-aware counts include the new generic
envelope and compatibility readers rather than treating moved format policy as
deletion.

**Preservation argument.** Preserve offline mode, cache corruption handling, per-key locking, quarantine, retry policy, pinned snapshot selection, local-file precedence, format identity, and exact format bytes.

**Migration and rollback.**

1. Extract a bounded verified-blob primitive from object acquisition.
2. Migrate object and manifest storage.
3. Migrate format-cache persistence to the same substrate while retaining TeX policy in `umber`.
4. Add a native distribution façade consuming rank 3’s selection plan.
5. Delete caller-owned verification loops.

Rollback keeps the old cache readers and can repopulate the new cache from verified objects.

**Gates and risks.** Test cache hits/misses, corruption, concurrent locks, quarantine, offline snapshots, partial warming, retry ordering, format closure, and local-file precedence. No network behavior was exercised during this review.

**Dependencies and conflicts.** Rank 3 must stabilize request and object identities first. Rank 4 consumes the resulting resource ledger. This estimate excludes distribution schema and selection deletion.

### 15. Merge shaping into the validated font owner

Affected crates: `tex-fonts`, `tex-shape`, `tex-exec`, `tex-typeset`, `tex-out`, and `umber`.

**Implementation — completed by `umber2-fjfh.14`.** `tex-fonts` owns validated SFNT data, cached shaping faces, OpenType context, font-unit conversion, and the private rustybuzz adapter. `LoadedFont::shape_run` is the one typed shaping operation and consumes the context already validated into `OpenTypeFont`. The duplicate `OpenTypeProgramSelection`, `OpenTypeFontSelection`, `ShapingFont`, and shaping-direction projection are gone. `tex-exec` performs run segmentation but neither reconstructs nor overrides font instance context.

**End state.** `tex-fonts` owns one immutable font/shaping context and one typed shaping operation. The temporary `tex-shape` package has been removed. Output lowering reads program, object, instance, variation, feature, direction, script, and language values from the same validated font instead of a second selection projection.

**Estimated net reduction.** 250–450 production LOC and 250–500 test LOC. Confidence: medium-high for the boundary deletion; lower for any run-plan redesign, which is intentionally not counted.

**Preservation argument.** Preserve glyph IDs, clusters, advances, offsets, direction, script, language, variation, features, line-breaking widths, MATH behavior, fallback, DVI font definitions, PDF subsetting, resource names, and WASM behavior.

**Migration and rollback.**

1. **Completed.** Move the exact shaping implementation and fixtures into a private `tex-fonts` module.
2. **Completed.** Compare the forwarding façade and owner through the unchanged fixture snapshots.
3. **Completed.** Migrate `tex-exec` and layout consumers.
4. **Completed.** Reuse validated font data in output and resource serialization.
5. **Completed.** Remove the crate and duplicate context types.

The historical rollback point retained the old crate façade and shaping entry point until all consumers and fixtures had migrated.

**Gates and risks.** Require TFM/OpenType, Arabic/Devanagari, ligature, mark attachment, variation, explicit language/script, break-suppression, line-breaking, DVI, PDF, and WASM parity. Risks include subtle shaping-library lifetime changes and font resource identity changes.

**Dependencies and conflicts.** Rank 1 must remove the legacy font execution path before its deletion is counted. Rank 3 and rank 14 must preserve distribution font identities. Rank 10 and rank 12 consume the resulting canonical font/resource values.

## Recommended dependency-aware execution order

### Wave 0: establish evidence and reversible authorities

1. Implement the typed evidence adapter from rank 5 while retaining the current stream.
2. Define the fixture/case authority from rank 7 as a read-only plan and compatibility adapter.
3. Freeze golden DVI, PDF, format, bibliography, semantic-event, resource, and fixture-tree comparisons.
4. Establish rollback switches at the current session, serializer, resolver, and fixture-command boundaries.

No deletion should begin until a divergence can be reduced to a named event, artifact, resource request, or state identity.

### Wave 1: collapse independent data and host boundaries

1. Establish the raw bibliography database boundary.
2. Execute rank 2’s Biber worker and rank 8’s private classic runtime behind old APIs.
3. Execute rank 9’s bibliography output protocol after both classic and Biber document paths are stable.
4. Make rank 3’s distribution catalog and selection model authoritative.
5. Migrate rank 14’s native acquisition/cache policy to consume that plan.

These changes have meaningful value without changing TeX execution and reduce the number of representations available to later work.

### Wave 2: consolidate project and canonical execution state

1. Introduce the workspace/resource ledger from rank 4 without deleting old maps.
2. Migrate `umber` and incremental resource views.
3. Switch production TeX sessions to canonical execution under rank 1.
4. Once the canonical path is the only path, implement rank 6’s common step and revision transaction.
5. Remove old executor, assignment, font, math, alignment, and checkpoint subsystems.

This is the critical deletion wave. It should be one reversible sequence, with full canonical/legacy differential evidence retained until the final removal.

### Wave 3: converge output and font backends

1. Merge shaping into `tex-fonts` from rank 15.
2. Establish one DVI compiler from rank 10.
3. Establish the PDF graph cursor and paint program from rank 12.
4. Re-run canonical, incremental, format, and browser parity after each backend.

Output refactors should follow canonical execution so new writers do not accidentally preserve legacy behavior that is about to disappear.

### Wave 4: simplify state and tooling after behavior stabilizes

1. Apply rank 13’s node schema/traversal consolidation only after measuring a real deletion.
2. Apply rank 11’s detached content/memo ownership change last among runtime changes.
3. Rank 7’s fixture/corpus authority consolidation is complete; preserve its one-plan transaction boundary as later output schemas evolve.

Every wave should leave the preceding compatibility reader or adapter available until the next wave’s gates pass.

## Coverage appendix

The nine crates over 10,000 Rust lines were reviewed through delegated component reviews and reconciled at synthesis: `bib-engine`, `tex-command-stream`, `tex-command`, `tex-exec`, `tex-fonts`, `tex-out`, `tex-state`, `tex-typeset`, and `umber`.

| Reviewed crate         | Synthesis disposition                                                                               |
| ---------------------- | --------------------------------------------------------------------------------------------------- |
| `bib-bst`              | Rank 8                                                                                              |
| `bib-engine`           | Delegated component review; ranks 2, 8, 9                                                           |
| `bib-graph`            | Rank 2                                                                                              |
| `bib-input`            | Ranks 2 and 8                                                                                       |
| `bib-label`            | Rank 2                                                                                              |
| `bib-model`            | Ranks 2 and 9                                                                                       |
| `bib-output`           | Rank 9                                                                                              |
| `bib-sort`             | Rank 2                                                                                              |
| `bib-unicode`          | No standalone program; collation remains a lower-layer dependency of rank 2                         |
| `corpus-manifest`      | Rank 7                                                                                              |
| `corpus-sync`          | Rank 7 complete; the standalone boundary was deleted and acquisition moved into `fixturegen`        |
| `fixturegen`           | Rank 7                                                                                              |
| `parity-harness`       | Ranks 5 and 7                                                                                       |
| `profile-analyzer`     | No independent program survives the high bar                                                        |
| `refexec`              | Retain the reference runner; its staging and comparison responsibilities fold into rank 7           |
| `test-support`         | Rank 7; small logging and harness cleanups are not separately counted                               |
| `tex-arith`            | No independent program survives the high bar; arithmetic ownership changes are too small alone      |
| `tex-command-stream`   | Delegated component review; ranks 5, 6, 7                                                           |
| `tex-command`          | Delegated component review; ranks 1, 5, 6, 13                                                       |
| `tex-content`          | No standalone program; its identity boundary is intentionally retained                              |
| `tex-exec`             | Delegated component review; ranks 1, 6, 10–13, 15                                                   |
| `tex-fonts`            | Delegated component review; ranks 1, 12, 14, 15                                                     |
| `tex-incr`             | Ranks 1, 4, and 6                                                                                   |
| `tex-observe`          | Rank 5                                                                                              |
| `tex-oracle`           | Ranks 5 and 7                                                                                       |
| `tex-out`              | Delegated component review; ranks 1, 6, 10–12, 15                                                   |
| `tex-shape` (removed)  | Rank 15                                                                                             |
| `tex-state`            | Delegated component review; ranks 1, 6, 11, and 13                                                  |
| `tex-typeset`          | Delegated component review; ranks 1, 13, and 15                                                     |
| `texlive-wasm-publish` | Rank 3                                                                                              |
| `umber-distribution`   | Rank 3 and consumer of rank 14                                                                      |
| `umber-fetch`          | Rank 14                                                                                             |
| `umber-interrupt`      | No independent program survives the high bar; its small watch integration is not a portfolio target |
| `umber-vfs`            | Rank 4                                                                                              |
| `umber-wasm`           | Ranks 3, 4, and 9                                                                                   |
| `umber`                | Delegated component review; ranks 1, 3, 4, 6, 9–12, 14, and 15                                      |

`tex-lex` and `tex-expand` were explicitly excluded from review synthesis and are not refactor targets here.
