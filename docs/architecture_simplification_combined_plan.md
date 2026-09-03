# Combined Architecture Simplification Plan

## Authority and scope

This plan synthesizes two independent repository-wide reviews:

- the Luna architecture simplification review of commit `b5b50aeefbef5da68c545a98055ad30cea394746`;
- the Codex [code-reduction architecture review](code_reduction_architecture_review.md) of commit `1ca11903c45b33e5a12e42ae09756a6d2d3db41e`.

The Codex snapshot is 13 commits newer and contains 1,658 inserted and 3,516 deleted lines relative to the Luna snapshot. Recommendations must therefore be revalidated against the implementation tip allocated to each Beads issue. This plan names architectural destinations and proof obligations; it does not freeze old line numbers as current truth.

The combined inventory contains all 35 non-root Cargo packages at the reviewed snapshots, including the independent `fuzz/` workspace package `umber-fuzz`. The earlier Codex report's claim of 34-of-34 coverage omitted that package. `umber-fuzz` has no independent simplification program and remains the justified isolated Cargo-fuzz boundary.

This document is an implementation portfolio, not a substitute for normative subsystem contracts. When it changes a current contract, the governing design document must change in the same issue.

## Portfolio rules

Every program follows these rules:

1. Name one surviving authority for each fact before deleting a predecessor.
2. Preserve byte, value, error, ordering, identity, resource, and performance contracts unless a separately approved decision explicitly changes one.
3. Use temporary differential implementations only for migration. Do not retain permanent dual paths.
4. Count only net deletion. Moving code, expected bytes, or logic into generated Rust is not a reduction.
5. Keep authored source, repetitive declarative/generated lines, and binary assets in separate accounting categories.
6. Treat public Rust APIs, CLIs, benchmarks, ignored tests, manual tiers, and documented migration commands as functionality until their owner approves retirement or a compatibility adapter exists.
7. Require a case-level assertion and citation ledger before compacting tests that are normative, ignored, or currently unreachable.
8. Create implementation issues in Beads. This document defines program boundaries and dependencies, not an ad hoc task checklist.

## Accounting

The non-overlapping planning ranges are deliberately lower than the sum of both reviews.

| Category                                          | Expected net reduction | Treatment                                                                                           |
| ------------------------------------------------- | ---------------------: | --------------------------------------------------------------------------------------------------- |
| Scheduled behavior-preserving authored source     |      21,000-32,000 LOC | Rust, JavaScript, tests, and scripts whose behavior moves to a named surviving authority            |
| Repetitive declarative/generated records          |    13,600-14,200 lines | Reported separately; primarily TeX82 dispositions and command-semantic manifests                    |
| Compatibility- and coverage-gated authored source |      23,000-29,000 LOC | Not counted until the named API, CLI, roadmap, or assertion-ledger gate closes                      |
| Approved paragraph-replay retirement              |        1,000-1,700 LOC | Scheduled optimization removal that changes the documented incremental-performance contract         |
| Maintainer-retirement candidates                  |          900-1,400 LOC | Historical benchmarks, traces, and prototypes; not behavior-preserving without an explicit decision |
| Binary fixture assets                             |      About 1.2-1.4 MiB | Optional font-fixture subsetting; excluded from all LOC totals                                      |

The scheduled authored range includes about 6,000-9,000 test LOC from declarative Biber compatibility cases. Without that test expansion, the scheduled production/tooling range is approximately 15,000-23,000 authored LOC, close to Luna's independently estimated 12,350-22,150 handwritten-line portfolio.

## Resolved design decisions

### Browser ownership

Rust owns engine policy, validated result DTOs, catalogue semantics, and the detached HTML producer model. JavaScript remains the actual asynchronous transport, worker, browser cache, hostile-input validation, DOM mutation, focus/scroll, and resource-lifetime boundary. Do not activate a main-realm Rust/WASM HTML receiver merely to delete the working JavaScript receiver.

### Resource ownership

Adopt Luna's canonical request identity and admission lifecycle, but do not create one giant resolver. `umber-vfs` owns typed identity, layers, and transactional admission; `umber-fetch` owns verified host acquisition; `tex-exec` consumes a narrow retained-resource capability; `tex-incr` owns candidate lifetime and publication; JavaScript owns asynchronous scheduling. Delete the two unused Rust Umber control planes after compatibility review instead of promoting them into a third scheduler.

### Observation ownership

Keep command observations and the immutable `tex-oracle` wire schema distinct. Replace repeated exhaustive walks with schema-owned borrowed/mutable views, typed producer enums, one finalizer, and one host comparison result. Do not make `tex-command` depend directly on the oracle wire model.

### Reference and fixture tooling

Fixturegen owns the minimal feature-gated reference-process and publication kernel. `test-support` owns generic DVI equality and focused fixture helpers. `tex-command-stream` owns semantic comparison policies. Parity remains comparison and triage only. Preserve compatibility commands until their CLI retirement is approved.

### Fixture inventory

Use one typed closed-case contract and normalized Git authority, but retain `corpus-manifest` as the small dependency-free external-corpus parser. Sharing the contract does not justify absorbing the package or introducing host dependencies into it.

### Bibliography mutation

Use one engine-owned mutable entry/section draft and one freeze into `bib-model`. Do not generalize `bib_model::EntryBuilder` into the mutable engine draft: duplicate policy, replacement ordering, and freeze-only validation are phase-specific. Generate the compatibility suite first so the production pipeline rewrite has exact behavioral evidence.

### PDF test inputs

Use `pdf-writer` for ordinary valid synthetic fixtures and Hayro's borrowed model for canonical projections and focused queries. Retain explicit raw-byte builders for malformed inputs, classic-xref-specific cases, cycles, depth limits, and tests whose value is independent writer/parser construction.

### Paragraph replay

Retained cross-revision paragraph replay is scheduled for retirement because the owner has approved replacing it with named restart checkpoints and suffix convergence. This is not classified as behavior-neutral: the current latency and retained-allocation contract changes and must be replaced explicitly.

### Benchmarks and prototypes

Do not delete a benchmark merely because CI does not invoke it. Classify it as an active gate, reproducible diagnostic, or historical experiment. Only the last class is eligible for deletion after maintainer approval.

## Ranked master programs

Rank reflects architectural leverage, dependency centrality, expected deletion, and evidence quality. Implementation order appears later.

| Rank | Master program                                              | Scheduled authored net |                 Additional gated net |
| ---: | ----------------------------------------------------------- | ---------------------: | -----------------------------------: |
|    1 | Main-control operation, assignment, and evidence            |            1,650-2,400 |           Test compaction in rank 15 |
|    2 | Oracle evidence, finalization, and comparison               |              900-1,350 |                                    - |
|    3 | Resource identity, admission, acquisition, and publication  |            1,200-1,900 |                  950-1,100 API-gated |
|    4 | Paragraph-replay retirement and revision/effect transaction |            1,400-2,100 | 1,000-1,700 approved contract change |
|    5 | Artifact codec and geometry traversal                       |            1,450-1,900 |                                    - |
|    6 | Detached PDF finalization in `tex-out`                      |              800-1,400 |                                    - |
|    7 | HTML producer and JavaScript receiver                       |                300-500 |                    550-700 API-gated |
|    8 | Executable state and node schemas                           |            1,000-1,500 |                2,100-2,700 API-gated |
|    9 | Primitive/profile/WEB catalogue                             |              900-1,400 |                                    - |
|   10 | Native typesetting authorities                              |              900-1,250 |            700-950 test-ledger-gated |
|   11 | Canonical font runtime                                      |              800-1,200 |                    150-250 API-gated |
|   12 | Fixture contract and catalogue compaction                   |                500-900 |      13,600-14,200 declarative lines |
|   13 | WASM wire, catalogue, and session driver                    |            2,600-3,600 |                    300-400 API-gated |
|   14 | Bibliography cases and production stages                    |           6,900-10,700 |          Public legacy APIs excluded |
|   15 | Dormant and repeated test authorities                       |                      - |         16,950-20,750 coverage-gated |
|   16 | PDF test support                                            |              700-1,000 |                                    - |
|   17 | VFS transaction and maps                                    |                      - |          750-1,000 roadmap/API-gated |
|   18 | Package, migration, benchmark, and prototype retirement     |                      - |           2,250-3,250 decision-gated |

## 1. One main-control operation, assignment, and evidence pipeline

**Outcome.** One `execute_operation` owns snapshot, delivery strategy, scanning/application, resource suspension, commit/rollback/fatal behavior, and an optional evidence sink. An `AssignmentCommitter` performs every TeX write once and returns typed mutation and tracing receipts. The zero-behavior runtime capability disappears.

**Combines.** Luna ranks 1 and 12; Codex ranks 7, 13, and 15.

**Counted reduction.** Approximately 1,200-1,800 scheduled authored LOC, plus 450-600 LOC from runtime construction plumbing where not already included. Scanner-test compaction is excluded until program 15's ledger closes.

**Proof.** Compare ordinary and observed state, resource requests, receipts, event bytes, effects, artifacts, fatal partial commits, alignment/nested execution, assignment tracing, local/global policy, glue identity, `afterassignment`, and performance.

**Dependencies.** Primitive descriptors in program 9 should stabilize before the final assignment migration. The evidence views in program 2 may proceed in parallel.

**Receipt and differential foundation.** The crate-private
`tex-exec::execution_receipt` boundary defines one typed aggregate-operation
receipt across state mutations, typed resource requests, semantic and live
effects, artifact identities, diagnostics, and termination. Its optional
evidence sink is allocation-free when disabled and record-bounded when active.
While the four predecessor entry shapes coexist, a temporary harness compares
exact detached state preimages, complete receipts, and ordered command evidence
for ordinary, observed, nested, and alignment operations. State and receipt
capture enforce their ceilings while appending, rather than after an unbounded
detached allocation; evidence retention and comparison steps are independently
bounded. The harness and entry shape selector are migration scaffolding to
delete with the predecessor paths; they introduce no `tex-oracle` schema or
other public wire change.

**Assignment authority.** `AssignmentCommitter` owns the write boundary for
scalar, glue, token, code-table, meaning, box, font, page, mode, and
hyphenation assignment families. Each commit makes the local/global and e-TeX
redundant-local decision once, emits `\tracingassigns` from the same pre- and
post-image, and returns the typed mutation receipt at that point. The former
pre-application mutation classifier and dormant variable/admissibility write
wrappers are deleted. Receipts remain operation-local until the existing
bounded evidence/receipt publication seam commits, so resource retries and
`\afterassignment` retain their prior atomic ordering without a wire change.

**Final program closeout.** The exact pre-closeout implementation tree
`bfbe33682891249f37796477543828fdb7d40097` retains one
`execute_operation`/`apply_operation` authority and one
`AssignmentCommitter`; `CommandRuntime`, `PendingMutation`, the temporary
shadow harness, and the three predecessor step branches remain absent. The
observer seam consumes every receipt category and verifies its termination
against the published result. Mutation, resource, semantic effect, world
effect, artifact, geometry, and diagnostic records reject before vector
growth, and success, fatal, alignment, `PdfXFormVoidBox`, and other
irreversible commit paths close and check the receipt before commit. Ordinary
execution retains no observation slot and therefore allocates no receipt or
ordered-evidence buffer.

The optimized exhaustive tracer compared every registered microfixture plus
Plain, Story, and Gentle to exhaustion under `MemoryMax=512M`, reporting zero
semantic divergences and zero advisory geometry differences in 12.02 seconds
wall time at 424,776 KiB maximum RSS. Its serial document loading, on-demand
alignment keys, exact-prefix release, and complete first-mismatch suffix keep
the comparison policy and all 1,107,729 document events intact. Cumulative
selected authored Rust accounting, including both receipt/tracer repair rounds,
is 3,819 additions and 4,617 deletions, or 798 net deleted lines; documentation
and guidance are excluded. Focused executor, incremental, command-semantic,
source-audit, exact DVI, text-channel, resource-retry, Story-locator, and
snapshot gates passed under 512 MiB; the full routine suite passed under 1 GiB;
and the uncapped six-job quality gate passed all four checks. The issue-scoped
receipt is recorded in Beads issue `umber2-vgjr.1.4`.

## 2. One oracle evidence, finalization, and comparison pipeline

**Outcome.** `tex-oracle` owns exhaustive event views for normalization, locations, position erasure, alignment keys, concise rendering, and typed profile projection. `tex-observe` enriches and finalizes once into semantic and geometry evidence. `tex-command-stream` owns named strict and ordinary comparison policies and returns divergence plus accounting in one parsed result. Parity consumes that result.

**Combines.** Luna rank 2 and the evidence half of rank 1; Codex rank 13.

**Counted reduction.** Approximately 900-1,350 scheduled authored LOC after excluding executor work counted in program 1 and fixture metadata counted in program 12.

**Proof.** Preserve schema-v1/v2/v3 bytes, independent sequence spaces, source coordinates, geometry provenance, strict TRIP order and macro/group proof, bounded realignment, report precedence, normalization hashes, and million-event bounds.

**Implemented authority and accounting.** The implementation commits are
`d819194fc`, `c29f2f232`, and `b3e141f75`; the exact integrated implementation
tree `b3e141f7520c1ce889221b8eeb9b560db8ea0df6` passed the uncapped full
`cargo test -q --tests --no-run` build, full execution under `MemoryMax=1G` and
a 30-minute timeout, and all four `scripts/check.sh` gates. `tex-oracle`'s
exhaustive event views are the surviving schema traversal authority,
`tex-observe`'s typed single-pass finalizer is the surviving evidence authority,
and `tex-command-stream::policy` is the surviving strict and ordinary comparison
and accounting authority. The duplicate normalization, location, alignment,
grouping, parity TRIP profile/geometry observer, strict projection, second JSONL
parse, and separate accounting walks were deleted. Schema-v1/v2/v3 canonical
bytes and validation behavior, detached evidence and source coordinates, strict
TRIP reports, malformed-input rejection, and the 1,000,000-event bound remained
covered; parity retains presentation only, so no permanent dual migration
authority remains. The only observed canonical TRIP mismatch was the separately
owned TeX Live 2025/2026 transcript banner tracked by `umber2-sfc.4`; command
evidence, geometry evidence, and normalized DVI were identical.

Exact commit `--numstat` accounting is 1,588 additions and 1,602 deletions in
production Rust (14-line net reduction), plus 414 additions and 34 deletions in
authored Rust proof tests (380-line net growth). Authored Rust therefore totals
2,002 additions and 1,636 deletions, or 366 lines of net growth rather than the
forecast reduction. Declarative/generated changes are four added Cargo manifest
or lockfile lines; compatibility-gated retirement and binary-fixture changes are
both zero. Documentation adds 46 lines and deletes three, making the total
tracked change 2,052 additions and 1,639 deletions, or 413 lines of net growth.
**Forecast reconciliation.** Follow-up `umber2-vgjr.20` audited every retained
production traversal, finalization, profile, comparison, accounting, and report
caller against the three surviving authorities. The original estimate assumed
1,800--2,400 lines of repeated implementation would be replaced by 800--1,050
lines of shared code. The implementation instead deleted 1,636 authored Rust
lines and added 2,002: the required schema-owned operations and strict TRIP
state machine replaced their predecessors nearly one for one, while 380 net
new lines are independent proof for exhaustive schema carriers, byte-identical
profiles, malformed input, report compatibility, and the million-event bound.
The audit found no second production owner. The few retained direct event
matches are typed profile validation, observation-to-schema translation, or
strict comparison semantics owned by their named layer, not alternate generic
walks or finalizers.

The portfolio owner therefore closes the original 900--1,350-line reduction
forecast at the measured +366 authored-Rust result and carries no unimplemented
deletion forward. Production remains net -14 lines; proof tests remain net
+380 lines. No moved code, generated source, fixture bytes, documentation, or
historical deletion is credited. Further deletion would weaken independent
schema/report/performance evidence or collapse the deliberately separate
engine-observation, wire-schema, and host-comparison boundaries, so it requires
a new contract decision. The retained-source inventory and exact accounting
are recorded in Beads issue `umber2-vgjr.20`.

## 3. One resource identity, admission, acquisition, and publication lifecycle

**Outcome.** A typed request key and state vocabulary connect VFS admission, native and browser scheduling, verified acquisition, engine suspension, incremental candidate ownership, and bibliography resource closure. `DistributionClient` owns source selection and one verified downloader/store state machine. Domain validation remains with fonts, images, formats, and bibliography.

The normative identity, transition, verification, publication, and phase-owner
contract is [Canonical resource identity and lifecycle](resource_lifecycle.md).

**Combines.** Luna rank 3; Codex ranks 16 and 22, plus the acquisition portion of rank 14.

**Counted reduction.** Approximately 1,200-1,900 scheduled authored LOC. Deleting exported non-driving Umber resource planes adds 950-1,100 conditional LOC only after API review.

**Proof.** Preserve request identity and ordering, required-versus-probe promotion, local/cache/remote precedence, offline behavior, exact-versus-bounded length, hash validation, retries, bounded workers, cancellation publication barriers, VFS isolation, revision rollback, and WASM asynchronous delivery.

**Implemented authority and accounting.** `umber_vfs::ResourceLifecycle` is the
sole ordered admission-transition authority used by file, OpenType, and PK
callers. `umber_fetch::VerifiedDownloader` is the sole native bounded transport,
retry, cancellation, length, and digest authority; `BlobStore::resolve_entry`
is the sole per-key verification, quarantine, migration, construction, and
publication transition; and `DistributionClient` remains the native
source-selection facade over them. Native scheduling stays in `umber`, browser
scheduling stays in authored JavaScript, domain validation stays with its
domain owner, and revision publication stays in `tex-incr`. The generic
lifecycle and downloader remain small independent state machines rather than a
resolver that absorbs those owners.

The duplicate object and manifest download/read/verify loops, four parallel
font/PK positive and negative admission maps, and the non-driving exported Rust
`OutputResourcePlan` and `CompositeResourceResolver` families were deleted.
The API decision was to provide no deprecated Rust adapter because neither
family had a production Rust caller. The authored JavaScript composite resolver
remains because it drives the live asynchronous browser protocol; it is not a
second Rust or engine-state authority.

Exact `--numstat` accounting across implementation commits `fbdaaa997`,
`cb8cd4a96`, `c576d4fb9`, `8e5af7538`, `3923f4167`, and `d5f896aa8` is 1,005
additions and 1,307 deletions in production Rust (302-line net reduction), plus
74 additions and 440 deletions in authored Rust proof tests (366-line net
reduction). Authored Rust therefore totals 1,079 additions and 1,747 deletions,
or 668 lines of net deletion. Declarative configuration adds four lines;
documentation and repository maps add 421 and delete 43. The implementation
change totals 1,504 additions and 1,790 deletions, or 286 lines of net deletion.
The linked missing-review repair `db0473664` adds 561 documentation lines, so
all five children together total 2,065 additions and 1,790 deletions, or 275
lines of net growth. No generated source or binary asset changed. The authored
reduction is below the 1,200-line forecast floor; the remaining opportunity or
explicit forecast revision is tracked by `umber2-vgjr.22` rather than credited
here.

Exact implementation tree `d5f896aa82e51d39f11b9112917b69060d480c74`
passed uncapped focused, full-workspace, and wasm32 `--no-run` builds; the full
native suite under a 1 GiB hard cap; the two real Node WASM lifecycle tests and
89 JavaScript unit tests under a 1 GiB hard cap; and all four `scripts/check.sh`
gates. The tests cover offline/cache/remote acquisition, retry and cancellation,
atomic batch admission, revision rollback, bibliography pass isolation, and
WASM delivery. Documentation repair tree `db0473664` additionally passed the
repository-wide local-Markdown-target audit and all four check gates.

**Forecast reconciliation (owner decision, 2026-08-07).** Follow-up
`umber2-vgjr.22` audited every surviving lifecycle, acquisition, store,
scheduler, domain-value, phase-publication, caller, compatibility facade, and
proof boundary. It found no second writable authority and no production
deletion with a named surviving owner and identical behavior. Parsed font
stores, batch frontiers, and retained phase views are projections required for
domain validation, bounded progress, or publication; authored JavaScript owns
the live asynchronous browser scheduler rather than duplicating Rust admission.

The original estimate correctly identified the deleted acquisition loops,
admission maps, and non-driving Rust planes, but treated too much gross deletion
as net reduction. The 1,079 added authored Rust lines implement the sole generic
admission and download/store transitions plus independent proofs for ordering,
offline reuse, retries, cancellation, atomic admission, and rollback. Removing
them to recover the 532-line shortfall would recreate implicit authority or
weaken the preserved contract.

The portfolio owner therefore retires the original 1,200--1,900-line forecast
and accepts the measured 668-line authored-Rust reduction as Program 3's final
result. No further reduction is carried for this program, and no moved,
generated, historical, fixture, documentation, binary, or compatibility-gated
lines are credited. The complete retained-authority audit and independently
reproduced accounting are recorded in Beads issue `umber2-vgjr.22`.

**Dependencies.** The contract precedes program 13's browser orchestration and program 17's VFS contraction. Do not wait for API retirement to begin the canonical internal lifecycle.

## 4. Retire paragraph replay, then establish one revision transaction and effect log

**Outcome.** Named restart checkpoints and suffix convergence become the only cross-revision reuse mechanism. After replay-only fields are removed, one immutable revision payload and transaction owns restart identity, resources, effect bundles, artifact rows, DVI plans, convergence, and publication. `tex-state` exposes an `EffectJournal`; executor commit closes a validated `RevisionOutputPatch`; `tex-incr` applies prefix, patch, and validated suffix.

**Combines.** Luna ranks 4 and 5; Codex rank 9.

**Counted reduction.** Approximately 1,400-2,100 scheduled authored LOC for effect/publication consolidation, plus 1,000-1,700 LOC from the separately approved paragraph-replay retirement.

**Proof.** Before replay deletion, record the replacement performance contract and fixed baseline. Preserve cold-equivalent artifacts, effects, resources, state hashes, rollback, pruning, accepted prefix/suffix boundaries, recursive output, terminal phases, OpenOut positions, suspension safety, and two-phase prepare/accept.

**Dependencies.** Replay retirement fixes the durable revision shape and therefore precedes final revision payload publication. State façade work in program 8 may be staged first.

**Implemented authority and accounting.** The implementation commits are
`dd261df36`, `c4791384a`, `7ea3520ac`, `c997f8415`, and `bc8230926`; the exact
integrated implementation tree `bc8230926956e8fafc397c2319712a1a7fe1d4a0`
passed the uncapped full `cargo test -q --tests --no-run` build, full serial
execution under `MemoryMax=1G` and a 60-minute timeout, the explicit 1,000-edit
cold-equivalence tier, and all four `scripts/check.sh` gates. The owner-approved
performance change is fixed by the before/after workload identities and ratio
budgets in [Paragraph replay deletion baseline](paragraph_replay_deletion_baseline.md):
all seven measured workloads remained byte-identical to cold execution, generic
suffix convergence remained effective, and paragraph transactions, recorders,
mounted line graphs, endpoints, and their retained-work counters became absent
or zero.

`tex-state::EffectJournal` is the surviving aligned effect-ledger authority;
executor-closed `tex_exec::RevisionOutputPatch` is the surviving artifact,
publication-row, and DVI-plan authority; and `tex_incr::RevisionTransaction`
with its immutable `RevisionPayload` is the surviving prepare/accept authority.
Named checkpoints and `exact_future_state_matches` are the only cross-revision
restart and convergence mechanism. Paragraph boundary/cursor identity helpers,
accepted paragraph transactions and mounts, `PendingRevision`, `AdvanceSetup`,
`RevisionRun`, and their positional effect/artifact assemblers were deleted.
Prefix/patch/suffix convergence now slices and recomposes validated ownership
units; detached accepted-output views do not retain a second publication owner,
so no permanent dual replay, revision, effect, or artifact authority remains.
Recursive output, terminal and `OpenOut` ordering, suspension without partial
publication, stale-base rejection, rollback, provenance rebasing, pruning, and
two-phase materialize/accept behavior remained covered.

Exact implementation-commit `--numstat` accounting is 695 additions and 938
deletions in production Rust (243-line net reduction), plus 117 additions and
55 deletions in authored Rust proof tests (62-line net growth). Authored Rust
therefore totals 812 additions and 993 deletions, or 181 lines of net reduction.
Declarative/generated evidence adds the 12-line workload checksum manifest;
compatibility-gated retirement and binary-fixture changes are both zero.
Documentation and repository guidance add 141 lines and delete 13. The total
tracked implementation change is 965 additions and 1,006 deletions, or 41 lines
of net reduction. By child boundary, replacement-contract evidence is 118/10,
approved replay cleanup is 6/253, effect-journal/output-patch work is 514/145,
and revision-lifecycle consolidation is 327/598 additions/deletions. The
forecast shortfall is reconciled below by `umber2-vgjr.21`; no historical
deletion or future compatibility-gated retirement is silently credited to
this program.

**Forecast reconciliation (owner decision, 2026-08-06).** Follow-up
`umber2-vgjr.21` audited every surviving constructor, owner, caller, host
adapter, and proof test. It found and removed two genuine remnants in
`77994b6e6`: the unused public `EffectJournal` positional decomposition and a
stored DVI-publication vector that duplicated the executor-validated artifact
ledger. The accessor now derives those rows from the ledger, preserving its
public behavior and the executor-closed validation boundary. This adds 6 and
deletes 39 production Rust lines, so the complete program outcome becomes 701
additions and 977 deletions in production Rust (276-line net reduction), 117
additions and 55 deletions in proof tests (62-line net growth), and 818
additions and 1,032 deletions in authored Rust overall (214-line net
reduction). Moved, generated, declarative, historical, and compatibility-gated
lines remain uncredited.

The remaining values are role-separated rather than duplicate authorities.
`World`'s live effect columns are the mutable recording substrate from which
`EffectJournal` is closed; `ArtifactLedger` owns artifact/publication
alignment while DVI plans carry distinct backend work; `RevisionDraft` owns a
mutable execution attempt, `RevisionPayload` freezes its selected output and
history, and `RevisionTransaction` is the stale-base-checked acceptance token.
Accepted-output and virtual-compile values are detached projections or
state-machine wrappers, not writable ledgers. Removing any of these would
either move the same behavior elsewhere or weaken executor closure, lifecycle
validation, two-phase acceptance, named-checkpoint/suffix convergence, cold
equivalence, or the accepted performance contract.

The owner therefore retires the original 2,400-3,800-line combined forecast
and accepts the measured 214-line authored-Rust reduction as program 4's final
portfolio result. The estimate incorrectly treated necessary replacement
types and proof growth as if they would disappear with the predecessor
authorities. No further reduction is carried for this program, and its
completion remains based on sole authority plus preserved behavior rather
than a line-count target.

## 5. One artifact codec and geometry traversal authority

**Outcome.** One iterative validated node-event cursor/emitter owns the versioned artifact grammar. Owned decode, zero-copy DVI planning, scan, validation, and production adapt to it. One explicit-frame geometry walker owns boxes, glue, leaders, snapping, ordinals, and sibling lookahead; DVI and positioned sinks retain backend policy. Fresh and memo-hit DVI derive from canonical artifact bytes.

**Combines.** Luna rank 7; Codex rank 10.

**Counted reduction.** Approximately 1,450-1,900 scheduled production LOC after counting the `tex-exec` dual-emission materializer only once.

**Proof.** Preserve artifact v23 and legacy bytes, error precedence and limits, nonrecursive replay, Unicode/classic validation, ligature source units, DVI movement/font/leader bytes, positioned effects, throughput, and RSS. The extra fresh-page byte pass has an explicit performance stop gate.

**Fresh-DVI authority and accounting.** Commits `27d9e24cf` and `dcb9fbda7`
complete the final program child. Live shipout now performs one immutable
compact-list pass into canonical artifact bytes. One executor helper calls
`tex_out::dvi::DviPagePlan::compile_v10` for both fresh and memo-hit pages and
owns their identical error conversion and DVI-disabled policy. The paired
live artifact/DVI builder arguments, streamed-plan branches, and 336-line
leader materializer are deleted; positioned shipout continues to consume the
same artifact when saved-position or snapping effects require it.

The initial final-child implementation changed production Rust by 34 additions
and 538 deletions, a 504-line net reduction. Active proof tests and their exact
source-audit coordinate added 79 and deleted one Rust line. Guidance added two
and deleted two lines. Including its initial plan/writeback, that child changed
176 authored lines and deleted 541, a 365-line net reduction. No artifact, DVI,
format, fixture, generated source, lockfile, or binary asset changed.

Fresh-versus-memo tests prove byte-identical artifacts, equal page plans, and
byte-identical serialized DVI, plus equal plan omission when DVI is disabled.
All 145 `tex-out` and 20 active `tex-exec` tests pass under 512 MiB. The
exhaustive canonical tracer reports zero semantic and geometry divergences;
Story and canonical Gentle match their byte-exact DVI oracles. Against the
stored predecessor benchmark, the 1,024-node ordinary and deferred-math rows
improve by about 12% and 42% respectively and peak at 49,836 KiB RSS. The
complete native suite passes under 1 GiB at 315,388 KiB maximum RSS.

Two adversarial closeout challenges then closed depth paths missed by the
initial proof. The first made canonical-byte scan, validation, ordinary box
replay, boxed-leader reconstruction, and temporary-tree retirement iterative.
The second replaced nested-leader re-entry in both DVI and positioned lowering
with scalar continuation frames. Active maximum-depth cases now cover a root
plus 4,095 ordinary box levels and a root plus 4,093 nested leader payloads;
the latter also pins depth-first event order, owned/canonical DVI equality, and
the first malformed-font error before a later sibling.

Final cumulative accounting across all three children and both repairs is
2,225 authored additions and 2,182 deletions, a 43-line net increase.
Production Rust is 1,505 additions and 2,082 deletions, a 577-line net
reduction; the remaining 720 additions and 100 deletions are active proof,
guidance, and durable closeout documentation. The original 1,450--1,900-line
production forecast is therefore retired: it counted codec and geometry proof
and replacement frames as deletion, while the measured surviving-authority
implementation removes 577 production lines.

Final exact-tree verification at `560be7d695b28debc807d7ac63b6ef32a12104c4`
passed all 151 `tex-out`, 466 `tex-exec`, and 40 active `tex-incr` tests under
512 MiB. Maximum-depth canonical bytes and nested leaders passed at 45,136 KiB
RSS; fresh/memo DVI and DVI-disabled parity passed; Story and the selected
canonical Gentle gate remained byte-exact. The exhaustive tracer was `CLEAN`
with zero semantic and zero advisory geometry divergences at 424,708 KiB. The
shipout diagnostic measured 1.0987 ms ordinary and 6.7585 ms deferred-math
midpoints at 249,736 KiB. The complete native suite passed under 1 GiB in
27.91 seconds at 308,156 KiB, and the final uncapped quality gate passed all
four gates.

## 6. Move detached PDF finalization into `tex-out`

**Outcome.** `tex-out` owns pure form validation, artifact lowering, page content, font usage, font-object emission, object allocation, and deterministic final serialization behind a typed `PdfFinalizationInput`. `umber` becomes a host/session compatibility adapter; `tex-fonts` remains the validated program/metrics owner.

**Source.** Luna rank 6. Codex's artifact and PDF-test findings support the migration but did not independently identify this ownership move.

**Counted reduction.** Approximately 800-1,400 scheduled production LOC, excluding artifact traversal in program 5 and font representation in program 11.

**Proof.** Preserve exact structure where normative, rendered pages, form reuse and cycle rejection, Type 1/TrueType/Type 3/PK behavior, subset identity, object order, resource limits, diagnostics, and incremental artifacts.

**Implemented ownership boundary.** `tex-out::pdf::PdfFinalizationInput` is the
complete host-neutral handoff. It owns accepted page and form artifact bytes,
realized font identities/metrics/programs, virtual-font inputs, external-image
bytes and validated metadata, expanded raw objects and document fragments,
navigation records, the committed object ledger, and every form/import/VF
limit that was previously a hidden finalizer constant. The allocation cursor
is a deterministic monotonic value type. Umber's compatibility adapter is the
last engine/host-aware step: it expands token lists, reads committed artifacts
and raw-object files, and copies validated resources before returning the
detached input. Pre-migration differential coverage froze two independent
legacy runs and established the exact detached boundary before the legacy
implementation and self-comparison were deleted.

Virtual-font finalization now retains each exact acquired local TFM transport
and its `tex-fonts` validation receipt. The adapter privately replays bounded
first-use allocation to freeze packet-sized realized identities and pdfTeX
resource/object numbers; `tex-out` reparses those transports at each declared
size and owns recursive packet lowering with no engine or host callback.
Nested size, width, identity, recursion bounds, and resource allocation are
checked at the detached boundary and against independently parsed output.

**Migration closeout and accounting.** Umber now constructs one
`PdfFinalizationInput` and calls `tex_out::pdf::finalize_pdf` for every native
and incremental PDF. The former handwritten finalizer, its test-only
self-comparison oracle, duplicate form/font/image/navigation/object lowering,
and duplicate selected-page resource importer are deleted. The retained
adapter only freezes accepted engine and host data, translates errors and
diagnostics, and replays the already validated allocation receipt; there is no
fallback or second serialization path. Exact PDF structure, committed fixture
bytes, Poppler render attestations, and focused `tex-out` form/font/VF/resource
tests provide independent evidence instead of comparing two Umber
implementations.

Production Rust adds 310 and deletes 9,991 lines, a 9,681-line net reduction.
Removing differential-only tests adds one and deletes 101 lines. Guidance is
line-neutral, and documentation adds 24 and deletes six lines. The complete
tracked migration therefore adds 339 and deletes 10,102 lines, a 9,763-line
net reduction; fixtures, generated source, lockfiles, and binary assets are
unchanged.

Fresh closeout verification restored two independent proof surfaces without
restoring the predecessor: the external qpdf matrix now generates its six
temporary compression/raster artifacts through native CLI jobs, and a compact
detached-only nested-VF test retains 12pt -> 6pt -> 9pt exact TFM identity,
width, allocation, tamper, cycle, depth, and Hayro-parse evidence. The repairs
add 248 and delete 23 lines. Commit arithmetic is 587 additions and 10,125
deletions; the base-normalized exact-tree diff is 580 additions and 10,118
deletions, a 9,538-line net reduction, because the test-module replacement
overlaps the migration hunk.

**Final program closeout.** The independent closeout audited implementation
tree `710ca9de0a66f7e513d1a93289feb5ad9b76b61a`. Repository-wide definition,
call-site, dependency, and inactive-source searches found one production call
to `tex_out::pdf::finalize_pdf`, no Umber `pdf-writer` dependency or second
serializer, no fallback, and no duplicate selected-page resource importer.
Umber's remaining PDF page parser only inspects host input metadata before
detachment. Its VF walk runs against a private cloned ledger to freeze
first-use allocation order; `tex-out` alone repeats packet lowering into the
final graph, imports selected-page resources, validates it, and serializes it.

Across all seven Program 6 commits, production Rust adds 6,412 and deletes
10,091 lines, a 3,679-line net reduction. Active proof Rust adds 213 and
deletes 102; the external-validator script adds 24 and deletes eight; manifests
and the lockfile add six; documentation and guidance add 68 and delete 26.
The complete commit-arithmetic total is therefore 6,723 additions and 10,227
deletions, a 3,504-line net reduction. No generated source, fixture, lockfile
resolution, or binary asset is credited as deletion. The exact category and
verification receipt is recorded in Beads issue `umber2-vgjr.6`.

## 7. One canonical HTML producer and JavaScript receiver

**Outcome.** A keyed `RenderDocument` or `RenderRevision` resolves positioned events, fonts, specials, accessibility, and math once. Standalone HTML/assets and incremental patch plans derive from it. JavaScript remains the browser trust and DOM transaction boundary.

**Combines.** Luna rank 8; Codex rank 19 and the render portion of rank 4.

**Counted reduction.** Approximately 300-500 scheduled producer LOC. Retiring the unused exported Rust receiver adds 550-700 conditional LOC after compatibility review.

**Proof.** Preserve exact standalone bytes, event ordinals, resource order and identity, accessibility, math glyphs, stable DOM identity, focus/selection/scroll, atomic rollback, leases, validation limits, CSP, and large-patch performance.

**Receiver disposition and accounting.** The 2026-08-06 public-use audit found
no production or external caller, tag, release, package, fork, crates.io
publication, or npm publication for the three-day-old Rust receiver API.
Production had always projected `PatchPlan` directly into the JavaScript
receiver. The compatibility gate therefore closed without an adapter: the
Rust envelope, protocol validator, abstract applier, re-exports, and
receiver-only tests were deleted. `HtmlPatchMount` is the sole hostile-input,
bounded simulation, detached DOM, resource-lifetime, atomic-publication, and
resynchronization authority.

The receiver retirement adds 127 and deletes 831 production Rust, JavaScript,
and TypeScript lines, a 704-line net production reduction. Replacement proof
tests add 238 and delete 163 lines, so total authored source adds 365 and
deletes 994 lines, a 629-line net reduction inside the conditional 550--700
forecast. Documentation and guidance are accounted separately; generated
declarations, fixtures, lockfiles, and binary assets are unchanged.

**Program closeout and cumulative accounting.** Commits `8ecf0cc74`,
`e32fb7219`, `391039b43`, `f8ccf3d6e`, `ec0edc3ae`, `b3a8ccaa5`, and
`f62894ec8` complete the program. One detached `RenderDocument` is the sole
producer authority: it resolves positioned values and original event
ordinals, font bindings and content-addressed resource order, typed specials,
accessibility grouping, validated math drawings, stable keys, and canonical
digests. Standalone serialization consumes that exact document without a
resolver or artifact pass. The retained compile session keeps it for snapshot
or resynchronization and as the next patch base, and sends the resulting
`PatchPlan` directly through the WebAssembly projection. The duplicate
standalone translator, second artifact lowering and font-resolution pass,
revision-only producer wrappers and state, production receiver envelope, and
the complete unused Rust validator and abstract applier are absent.

Exact cumulative accounting is 879 additions and 1,573 deletions in production
Rust, JavaScript, and TypeScript, a 694-line net reduction. Authored proof tests
add 529 and delete 210 lines, so all authored source adds 1,408 and deletes
1,783 lines, a 375-line net reduction. Documentation and guidance add 127 and
delete 35 lines. The complete tracked program therefore adds 1,535 and deletes
1,818 lines, a 283-line net reduction. Generated and declarative source,
fixtures, lockfiles, and binary assets are unchanged. The receiver retirement
meets its conditional forecast, but the two producer children together add ten
net production lines rather than deleting the scheduled 300--500; the
remaining opportunity or explicit forecast revision is tracked by
`umber2-vgjr.26` and is not credited from gross predecessor deletion.

**Producer forecast reconciliation (owner decision, 2026-08-07).** Follow-up
`umber2-vgjr.26` audited every surviving artifact, positioned-page, retained
session, patch, WebAssembly projection, and JavaScript receiver path. Commit
`220fcd793` removes the one remaining duplicate producer step: the public
standalone `write_html` adapter no longer owns a second artifact-to-positioned
lowering loop and instead enters the same `build_render_document` authority as
the retained session. Exact standalone output remains equal across the
artifact, positioned-page, and detached-document entry points.

The surviving paths are not interchangeable predecessors. The artifact and
positioned-page functions are public input adapters; `build_render_document`
and its positioned worker are the sole detached construction path;
`write_render_document` is the required standalone serializer; `plan_patch` is
the typed revision diff; the WebAssembly projection materializes JavaScript
DTOs; and `HtmlPatchMount` remains the sole hostile-input validation, detached
DOM publication, focus/scroll, rollback/resynchronization, and resource-lifetime
authority. Removing any of them would delete a distinct contract or proof
boundary rather than duplicate production.

The reconciliation commit adds 29 and deletes 34 production Rust lines, a
five-line net reduction, and adds two proof-test lines. Across the two original
producer children and this follow-up, exact production accounting is therefore
747 additions and 742 deletions, five lines of net growth. Across the complete
HTML producer/receiver program, production accounting becomes 908 additions
and 1,607 deletions, a 699-line net reduction; proof tests become 531 additions
and 210 deletions. Generated and declarative source, fixtures, lockfiles, and
binary assets remain unchanged. The portfolio owner retires the original
300--500-line producer-reduction forecast and accepts the measured five-line
growth as Program 7's final producer result. The 704-line receiver reduction
remains separate, and no gross migration deletion, receiver deletion,
documentation, generated/declarative source, fixture, lockfile, binary asset,
or historical change is credited to the producer result.

The implementation tree at `220fcd793` passed uncapped six-job focused, full
native, wasm32 check, and wasm32 test compilation. The focused `tex-out` suite
passed 151 tests under 512 MiB, and the complete native routine suite passed
under 1 GiB. Biome and all 91 Node tests passed under 1 GiB. The optimized
package build passed uncapped; the packaged Node lifecycle and 36-file npm dry
run passed under 1 GiB. Browser execution remains unavailable rather than
passing: `check-wasm.sh` reports Firefox missing, and the retained Chromium
fixture stops before execution at `/usr/bin/google-chrome` `ENOENT`. Its active
DOM identity, `MutationObserver` isolation, accessibility, focus/scroll,
rollback, resynchronization, resource lifetime, disposal, and 200-patch
performance assertions remain unchanged. `scripts/check.sh` passed all four
gates with six build jobs.

Exact code tree `f62894ec802ebe4a5db6487c3670d250621c335d` passed fresh
uncapped six-job focused, full native, and wasm32 test compilation. Focused
`tex-out` and Umber execution passed under `MemoryMax=512M`, and the complete
native suite passed under `MemoryMax=1G`. The wasm32 check, Biome, all 91
authored Node tests, optimized package build, packaged Node project lifecycle,
npm inventory, and all four `scripts/check.sh` gates passed. The environment
had no Firefox executable, and the Chrome fixture stopped before execution at
`/usr/bin/google-chrome` `ENOENT`; neither result is a browser pass. The
committed browser fixture still owns real DOM identity, mutation isolation,
200-patch latency, and disposal, while `umber2-5zie` retains the environment
blocker. Under the explicit opt-in test policy, that precise environmental
`BLOCKED` result plus the retained runnable gate is sufficient for this
architecture closeout; it does not replace future browser execution.

## 8. Make state and node schemas executable and singular

**Outcome.** A `NodeRef`-centered exhaustive schema owns tags, semantic fields, handles, ordered children, remapping, equality, validation, hashing, copy, and format projection while compact storage remains specialized. Production frozen-format decode is the only restoration authority. Internally, `Universe` becomes the sole state façade and private `Stores` becomes field-oriented data.

**Combines.** Luna ranks 13 and 14; Codex ranks 6 and 12.

**Counted reduction.** Approximately 1,000-1,500 scheduled production/test LOC for the node schema and test-only format path. Removing public expansion/store forwarding adds 2,100-2,700 conditional LOC after API policy or a deprecation adapter.

**Proof.** Preserve schema tags and versions, origin exclusions, child order, allocation-free views, compact sidecars, survivor patching, malformed-reference rejection, lookup rebuilding, group invalidation, dependency observation, capability restrictions, state hashes, and nonrecursive behavior.

**Program closeout and cumulative accounting.** Commits `8394bbbb3`,
`01d37a893`, `a895bea84`, `070dc6408`, and `84621b1a7` complete all four
children. `node_arena::schema` is the one exhaustive `NodeRef`-centered
logical grammar: schema-owned borrowed views and typed handle events drive
semantic equality, live-handle validation, canonical child traversal,
survivor-remap validation, hashing, and frozen projection. The private compact
word/sidecar encoder and copier remain the specialized storage codec, and the
validated handle-free frozen DTO remains the portable schema-11 boundary;
neither is a second logical node model. `Universe::from_format` and
`Stores::decode_frozen_format` form the sole production format restoration
path. The former test-only `StoreFormat` replay and alternate recursive hash
helpers are deleted.

The public-use audit found no workspace or demonstrated external generic
consumer for the former `ExpansionState` façade. Under the pre-1.0
workspace-internal API policy it, `ExpansionContext`, and `MeaningCacheGuard`
were removed without a deprecation adapter. `Universe` is the state-facing
API; the `stores` module remains private and its `Stores` aggregate is retained
only as rollback-coupled implementation data. `CommandContext` and
`InputOpenContext` remain the intentional restricted capabilities. The exact
disposition and closeout evidence are recorded in Beads issues
`umber2-vgjr.8` and `umber2-vgjr.8.4`.

Across the four implementation commits, authored Rust totals 1,412 additions
and 3,248 deletions, a net reduction of 1,836 lines. Documentation and guidance
across those commits plus the format-authority repair total 75 additions and
20 deletions and are reported separately. The combined 3,100--4,200-line
conditional forecast is therefore short by 1,264--2,364 lines; moved code,
specialized compact storage, the portable DTO codec, and documentation are not
credited as deletion, and no further reduction is silently scheduled.

## 9. Define one primitive, profile, WEB, and installation catalogue

**Outcome.** One dependency-light declarative Rust catalogue owns stable operand, spelling and aliases, profile membership, expandable class, WEB identity, prefix/admissibility flags, installation policy, parameter cell/default, and documentation family. Execution bodies remain handwritten.

**Combines.** Luna rank 11; Codex rank 11.

**Counted reduction.** Approximately 900-1,400 scheduled authored LOC, counted once across state, command, executor, Umber, tests, and docs.

**Proof.** Preserve numeric operands, profile layout, install order, format-load rebuilding, aliases, frozen/private meanings, `nullfont`, `endwrite`, punctuation/control space, parameter slots, observation bytes, and documentation completeness.

**Implemented authority and accounting.** Commits `dcfa38478`, `748ce7604`,
`9ee00dd5a`, and `664d1a8b9` establish the schema, generated views, and final
consumer migration. `tex-command::primitives` is the surviving behaviour-free catalogue
authority. `tex-state` supplies typed cells and meanings but no primitive name
inventory; `tex-exec` retains handwritten dispatch and thin compatibility
wrappers; Umber applies catalogue-projected pdfTeX defaults and owns no second
158-name or parameter table. Fresh INITEX and format restoration share the
same installation views, including aliases, punctuation/control space,
`nullfont`, frozen `endwrite`, page/internal quantities, and exact profile
names. Observation identities continue to project from the enum rows; no
execution callback or dispatch body moved into the catalogue.

The migration commits contain 636 additions and 1,238 deletions in authored
Rust, a measured net deletion of 602 lines. They remove `tex-state`'s parameter
spelling tables, the TeX82 and
e-TeX executor parameter/page/internal inventories, Umber's pdfTeX name,
meaning, default, and special-meaning inventories, and their predecessor-parity
loops. The two foundation commits intentionally added the schema and generated
views before deletion: they contain 1,839 authored-Rust additions and 34
deletions, a net addition of 1,805 lines. Across all four implementation
commits, authored Rust therefore totals 2,475 additions and 1,272 deletions, a
net addition of 1,203 lines. The 602-line reduction is the retired-consumer
category, not a whole-program net reduction; moved or generated declarations
are not credited as deletions.

## 10. Replace typesetting shadow arenas and repeated topology with native authorities

**Outcome.** A detached native-node transaction replaces the second math arena. `ParagraphTape` owns `NodeSequence`, analyzed break sites, prefix metrics, trace ranges, and materialization actions. A shared metrics cursor supplies packing, line breaking, vertical contributions, and math without erasing domain-specific policy.

**Source.** Codex rank 8. Luna treated its smaller constituent proposals as individually sub-threshold; the combined transaction/tape program clears that bar.

**Counted reduction.** Approximately 900-1,250 scheduled production LOC. Another 700-950 test LOC is conditional on a rule-by-rule assertion ledger.

**Proof.** Preserve 20,000-depth stack safety, occurrence-ordered observations, selected OpenType glyphs, source-box geometry, semantic versus physical paragraph channels, discretionary topology, overflow behavior, trace routes, glue policy, and hot-path performance.

**Paragraph-tape authority and accounting.** Commit `d338ff3cb` implements the
paragraph slice (`umber2-vgjr.10.2`). `tex_typeset::linebreak::ParagraphTape`
is the surviving owner of the paired native `NodeSequence`, analyzed legal
break sites and wide prefix metrics, trace spans, and compact materialization
actions. Every tolerance and diagnostic pass reads those saved sites.
Post-line-break processing advances semantic and physical cursors together;
the executor no longer carries separate node, physical-node, and boundary
fields or reconstructs the projection. Nested discretionary width traversal
is iterative and is covered at depth 20,000; a 100,000-node paragraph proves
linear analysis storage, and a projected-channel test proves physical
diagnostic topology.

This slice adds 427 and deletes 179 production Rust lines, a net addition of
248. Proof tests and the exact dormant-source ledger add 148 and delete six
Rust lines, a net addition of 142. Documentation adds 11 lines. The complete
change therefore adds 586 and deletes 185 tracked lines, a net addition of
401. The deleted production category includes the three parallel
`LineBreakResult` topology fields, executor-side boundary reconstruction,
per-pass breakpoint/prefix walks, and the recursively nested physical
materializer. New tape records, paired-cursor machinery, iterative deep-list
measurement, and their invariants outweigh those deletions; none of program
10's forecast reduction is credited to this child.

**Shared-metrics authority and test accounting.** `umber2-vgjr.10.3`
establishes `tex_typeset::metrics` as the neutral metric-event and accumulation
authority for packing, line breaking, vertical contributions, and Appendix G.
Packing still owns glue setting and diagnostics; line breaking still owns
font-expansion capacity, prefix subtraction, route badness, and demerits;
vertical breaking still owns legal breakpoints, infinite-shrink reporting, and
typed overflow; math still owns transaction topology and occurrence-ordered
pack observations. The shared seam therefore removes arithmetic/topology
duplication without turning those policies into flags.

The implementation adds 556 and deletes 201 production Rust lines, a net
addition of 355. The new event/cursor vocabulary and explicit overflow-policy
boundary outweigh the removed local accumulators, so none of program 10's
forecast production reduction is credited. The closed
`typesetting_assertion_ledger.md` maps every assertion in the three removed
migration aggregate cases to active case-level owners and retains the unique
vertical-break tie case. Test Rust adds 81 and deletes 165 lines, a net deletion
of 84; this is the only test reduction credited to this child. The additional
focused tests preserve vertical-break overflow semantics and independently own
the compacted shifted-vpack and clean-character assertions.

**Program closeout and exact accounting.** The seven program commits are
`c0b05de33`, `082687614`, `cb2f9fb60`, `12e8be260`, `c93415e2b`,
`5cf76deb8`, and `56995b38f`; unrelated interleaved font commits are excluded.
Production Rust adds 1,099 and deletes 494 lines, a net addition of 605. Proof
and test Rust adds 283 and deletes 184 lines, a net addition of 99. Authored
Rust therefore adds 1,382 and deletes 678 lines, a net addition of 704.
Documentation and guidance adds 158 and deletes seven lines; declarative
property maps add seven and delete three. Across every category the program
adds 1,547 and deletes 688 lines, a net addition of 859. The only credited test
reduction is the 165-line assertion-ledger compaction; no forecast production
deletion is credited.

The surviving math authority is one postorder `MathLayout` native-node
transaction, published atomically only by the executor's
`commit_math_transaction`. The retired `MathLayoutReader`, `MathLayoutSink`,
`MathLayoutBuilder`, source-leaf round trip, and public sink publication API
remain absent. `ParagraphTape` is the sole paragraph-analysis owner and
`LineBreakResult` retains no parallel node, physical-node, or boundary fields.
`MetricEvent`, `MetricsCursor`, `ListMetrics`, and `WideMetricTotals` form the
shared neutral measurement authority, while packing, line-break,
vertical-break, and math policy remains explicit in the owning modules.

Active tests own the 20,000-depth math and discretionary bounds, 100,000-node
linear paragraph storage, paired semantic/physical diagnostic topology,
source-box geometry, occurrence-ordered math observations, undefined-family
recovery, wide-prefix overflow, packing differential, and vertical-break
overflow/order behavior. The closed assertion ledger names the exact owner of
every removed aggregate assertion and retains the unique artificial-end tie
case. Because the measured result does not meet either forecast reduction,
the variance is tracked by `umber2-vgjr.25`; no historical or prospective
deletion is credited here.

**Forecast reconciliation (owner decision, 2026-08-07).** Follow-up
`umber2-vgjr.25` audited every surviving math transaction and publication,
paragraph analysis and materialization, shared metric projection and
accumulation, caller, compatibility convenience, and proof boundary. It found
no second production authority and no deletion with a named surviving owner
and identical behavior. The manually selected-break post-line-break API is a
retained pure compatibility and focused-proof boundary, not another paragraph
analysis authority. Packing, line breaking, vertical breaking, and Appendix G
retain distinct policy around the neutral metric seam rather than encoding
those decisions as flags in a second generic engine.

Reaching the original floors would require another 1,505 net production and
799 net test deletions from the measured results. That inventory does not exist
without removing the transaction, tape, or metric authority; collapsing
semantic and physical topology or domain policy; weakening independent
deep-stack, geometry, observation, overflow, exact-byte, or performance proof;
or moving source into an excluded accounting category. The portfolio owner
therefore retires the 900--1,250 production and 700--950 conditional test
forecasts and accepts the measured 605-line production growth and 99-line
proof/test growth as Program 10's final result. Authored Rust growth is 704
lines. No shortfall or unimplemented deletion is carried forward, and no moved,
generated, declarative, documentation, or total-line change is credited. The
retained-authority audit, exact category accounting, and verification are
recorded in Beads issue `umber2-vgjr.25`.

**Allocation-budget repair.** Follow-up `umber2-vgjr.27` restores Program 10's
unchanged layout-allocation ceilings by storing breakpoint trace data in the
owning `BreakSite`, reusing transaction-private Appendix G planning and
conversion buffers across the postorder schedule, and representing an empty
observation replay without allocated sequence storage. The sole
`ParagraphTape`, detached math transaction, shared metric authority, benchmark
budgets, and Program 10 accounting remain unchanged. The bounded profile and
exact capped measurements are recorded in Beads issue `umber2-vgjr.27`.

## 11. Publish one canonical font runtime while preserving format-specific policy

**Outcome.** TFM parsing retains raw tables only through reference and error-precedence validation, then publishes canonical `FontMetrics` through one loaded-font constructor. OpenType MATH uses one strict eager validation walk and lazy borrowed queries through the existing scaled facade. A realized font identity feeds HTML, PDF, incrementality, and distribution boundaries without repeated decoding.

**Combines.** Luna rank 9; Codex rank 17.

**Counted reduction.** Approximately 800-1,200 scheduled production LOC. Raw public TFM/MATH model deletion adds 150-250 conditional LOC after compatibility handling. Binary fixture subsetting remains outside LOC accounting.

**Proof.** Preserve TFM error identity and precedence, lig/kern and absent-character rules, `font_info_words`, parameter padding, MATH strictness and budgets, variation/device policy, shaping, fallback precedence, glyph numbering, PDF subset identity, HTML paths, cache identity, and fuzz coverage.

**Implemented TFM authority and accounting.** Commit `991fc7fcf` completes
the TFM slice (`umber2-vgjr.11.1`) from source snapshot `6618b3441`.
`tex-fonts::FontMetrics` is the sole retained classic metric representation.
The binary parser keeps private raw character, lig/kern, kern, and extensible
records only through structural, reference, chain, and error-precedence
validation, then constructs `FontMetrics` directly. `TfmFont` retains that
canonical record plus header, selected size, padded parameters, and
`font_info_words`; its consuming constructor is the one path used by executor
and VF loading to create `LoadedFont`. The public raw TFM DTOs and the later
raw-to-runtime conversion are deleted. This is an intentional public Rust API
contraction: repository inventory found only parser tests and the live
reference tool using those DTOs, while formats, output, and wire contracts
already use canonical metrics. The following MATH entry records the completed
`umber2-vgjr.11.2` sibling; realized font identity remains assigned to `.11.3`.

The implementation adds 183 and deletes 242 production Rust lines (59-line
net deletion), adds 73 and deletes 57 Rust test lines, adds 53 and deletes 34
live-reference tool lines, and changes four documentation lines in each
direction. By deletion category, the raw public model and conversion boundary
add 54/delete 187, private validation plus direct projection add 125/delete
30, runtime caller construction add four/delete 25, and proof migration adds
126/deletes 91. Total authored change is 313 additions and 337 deletions, a
24-line net deletion. No fixture, format, wire, generated, lockfile, or binary
asset changed. The program-level 800--1,200 production-line forecast spans the
two open sibling slices, so no child-level reduction shortfall is inferred.

On the implementation tree, 80 focused font tests, nine VF tests, 99 selected
format tests, all 33 fixturegen tests, the live `tftopl` comparison, a built
TFM fuzz target with a 10,000-input smoke run, the complete native routine
suite, and all four `scripts/check.sh` gates pass. Focused execution remained
well below 512 MiB; independent font/VF RSS measurements found no runtime
growth. The final complete suite passed under `MemoryMax=1G` at a
631,353,344-byte peak without any memory event. The six-job quality gate also
passed at a 105,771,008-byte peak without any memory event.
**Implemented MATH authority and accounting.** `ttf-parser::math::Table` is the
sole MATH graph. `OpenTypeFont` retains canonical decoded SFNT bytes and only a
validated-presence bit; `OpenTypeMathMetrics` reparses a borrowed table and
performs lazy constant, glyph-info, kern, variant, construction, and assembly
queries at the selected size. The independent eager validator still walks all
offsets and records before publication and enforces the prior version, range,
coverage-correspondence, ordering, glyph-bound, device/variation-format,
record-budget, and assembly-part-budget invariants. The public owned types
`MathAdjustment`, `MathConstants`, `MathGlyphAssembly`,
`MathGlyphConstruction`, `MathGlyphInfo`, `MathGlyphPart`, `MathGlyphVariant`,
`MathKern`, `MathKernInfo`, `MathTables`, `MathValue`, and `MathVariants` are
deleted; `MathConstant`, `MathMetricsSource`, and `OpenTypeMathMetrics` remain
the consumer boundary, with `OpenTypeFont::has_math` as the capability query.
Before documentation, the MATH change contains 272 additions and 456 deletions
in production Rust (net deletion 184) and 51 additions and 94 deletions in
tests (net deletion 43). No moved or generated lines and no binary assets are
credited.

**Implemented realized-font authority and accounting.** Commit `71a8fb54f`
completes `umber2-vgjr.11.3`. `LoadedFont::realized_identity` is the sole
host-neutral digest of selected metrics, size, layout and fallback policy,
mapping, OpenType program and instance, and generated-font ancestry;
`FontSourceIdentity` remains an exact public compatibility alias.
`OpenTypeFont::instance_identity` is the one complete instance projection.
`PdfFontResourceIdentity` owns pdfTeX's intentionally narrower font-object
reuse view, preserving equal-TFM/program subset reuse across selected sizes.
The session's retained HTML resource view supplies the same validated
`OpenTypeFont` selected during layout, and the producer shares its decoded
SFNT allocation for explicit OpenType and mapped classic painting. External
`HtmlFontAssets` implementations remain source-compatible through a default
method and decode their supplied WOFF2 exactly once.

The active HTML path's second WOFF2 decode and parse, three independent
instance-context projections, the state-owned PDF TFM/program tuple, and
redundant non-emitted DVI consistency fields were deleted. Fallback
precedence, glyph IDs, DVI definitions, Type 1/TrueType/PK/VF finalization,
PDF subset identity, HTML paths and families, format and artifact bytes,
distribution/resource/cache keys, and cold/warm/offline behavior are
unchanged. No artifact, format, font-wire, distribution, JavaScript, or cache
schema changed.

The realized-font commit adds 217 and deletes 130 production Rust lines (87
lines of net growth) and changes no Rust test source. Guidance and normative
documentation add 40 and delete five lines, for 257 additions and 135
deletions overall. Across all three program children, production Rust adds 672
and deletes 828 lines, a 156-line net deletion; the remaining forecast audit
is tracked by `umber2-vgjr.11.4` rather than credited to moved or compatibility
code.

The exact implementation tree passed uncapped six-job focused and complete
native `--no-run` builds. Under `MemoryMax=512M`, 79 font tests peaked at
61,752 KiB, 145 output tests at 262,304 KiB, 728 state unit tests at 75,600
KiB, and the complete Umber package at 322,696 KiB. The full routine suite
passed under `MemoryMax=1G` at 314,712 KiB after one isolated and nonrepeating
RSS-guard timing failure. The final six-job `scripts/check.sh` run passed all
four gates under 1 GiB at 109,856 KiB after the known cold clippy compiler
cache was warmed uncapped. The WASM check, Biome, and 89 Node tests passed
under 1 GiB at 716,060 KiB; real wasm-bindgen, browser-package, and npm-pack
steps were blocked because this host lacks `wasm-pack` and Firefox.

Fresh verification after the rebase onto `12e8be260` found that HTML's decoded
mapping-coverage check still parsed collection face zero, even when the
artifact or retained realized program selected a different face. Repair commit
`bfc0f05a8` makes the realized or artifact face authoritative and adds a
synthetic two-face TTC regression whose second face owns a glyph absent from
the first. The repair adds 31 and deletes 16 production Rust lines and adds 63
and deletes one proof-test line. The realized-font issue therefore totals 248
additions and 146 deletions in production Rust (102 lines of net growth), plus
63 additions and one deletion in tests. Across all three program children,
production Rust adds 703 and deletes 844 lines, a 141-line net deletion; linked
forecast audit `umber2-vgjr.11.4` remains open with this corrected accounting.

Exact repaired tree `bfc0f05a89db0df1f81147d1096938963f1aef24` passed the
uncapped six-job complete native `--no-run` build. Under `MemoryMax=512M`, 79
font tests peaked at 91,029,504 bytes, 146 output tests at 278,241,280 bytes,
729 state unit tests at 97,615,872 bytes, and the complete Umber package at
415,879,168 bytes, all with zero memory events. The complete routine suite
passed under `MemoryMax=1G` at 681,955,328 bytes with zero events. The final
six-job `scripts/check.sh` run passed all four gates under 1 GiB at 150,532,096
bytes after an uncapped clippy cache warm. The wasm32 check, Biome, all 89 Node
tests, the built-package Node project consumer, and `npm pack --dry-run` passed;
wasm-bindgen remained blocked by absent Firefox and the browser smoke by absent
Chrome. The available cold and warmed release-package runs both exceeded the
1 GiB cap inside `wasm-opt`; the same package completed uncapped, and no Rust,
Node, browser, or package-validation failure occurred.

**Forecast reconciliation.** Issue `umber2-vgjr.11.4` closes the original
800--1,200-line production forecast at the measured result rather than
crediting output DTOs, compatibility aliases, generated source, documentation,
or binary fixtures. Its complete retained-source inventory and compatibility
rationale are recorded in Beads issue `umber2-vgjr.11.4`.
The only additional independently justified duplicate was native SFNT storage:
commit `a5df75c73` makes the transport and decoded views share one allocation for
OTF, TTF, TTC, and OTC containers, while WOFF2 correctly retains distinct
compressed and decoded allocations. That implementation changes production
Rust by +11/-11 and proof tests by +8/-0. Program 11 therefore totals +714/-855
production Rust, still net -141; the original forecast overstates the measured
reduction by 659--1,059 lines. There is no remaining scheduled production-LOC
forecast for this program. Further deletion would contract a named public or
serialized boundary and requires a new policy decision rather than being
silently credited here.

**Program closeout.** All four children are closed and the exact integrated
tree at `357d997cc3f4e79397233c8651e3a85786b5875a` has no permanent dual font
authority. The production categories are canonical TFM +183/-242, lazy MATH
+272/-456, realized identity and selected-face repair +248/-146, and shared
native SFNT storage +11/-11: +714/-855 in total, net -141. The corresponding
reduction is 659--1,059 lines below the retired forecast; no unimplemented
reduction is carried forward. Fresh closeout verification rebuilt the focused
font and complete native suites uncapped with six jobs, then passed all 79
font tests under 512 MiB and the complete routine suite under 1 GiB. The
10,000-input TFM fuzz smoke passed under 512 MiB. Under 1 GiB, the wasm32
check, Biome, all 89 authored Node tests, release package through `wasm-opt`,
the built-package Node consumer, and `npm pack --dry-run` passed. Firefox and
Chrome were absent, so their browser-only checks were unavailable and are not
reported as passes. The final exact-tree repository quality result is recorded
in Beads issue `umber2-vgjr.11`.

## 12. Establish one fixture contract while compacting repeated catalogues

**Outcome.** A typed closed-case contract owns identity, tracked inputs, expected outputs, statuses, xfail reasons, profiles, and publication metadata. Command-semantic V2 infers conventional fields and embeds capture policy. The TeX82 catalogue uses an implicit typed default disposition plus explicit overrides. Fixturegen alone mutates and publishes; test-support validates and stages; `corpus-manifest` remains the external-corpus leaf.

**Combines.** Luna rank 15; Codex ranks 3 and 5, plus the contract portion of rank 18.

**Counted reduction.** Approximately 500-900 scheduled authored LOC after overlap, plus 13,600-14,200 repetitive declarative/generated lines. All meaningful expected values remain explicit and are not counted as deletion.

**Proof.** Preserve Git authority, normalized paths, exact case membership/order, source closure, xfail reasons, schema compatibility, missing/extra rejection, traversal protection, command capture selection, TeX82 module census and ownership, atomic publication, and local fixture-edit workflow.

**Implemented authority and accounting.** Commits `5675a359f`, `fbcaa9616`,
`0634d2800`, `5a012b9e7`, and `1aceb8ae8` establish the contract and complete
consumer migration. `test-support::closed_case::FixtureCase` is the surviving
typed identity, Git inventory, path, role, source-closure, status, xfail,
profile, and publication-metadata boundary. `test-support` alone validates and
stages candidates and owns the canonical `case.inventory` serializer, but has
no authority-mutating operation. Fixturegen's cohort transaction is the sole
atomic publication authority; command-semantic, PDF, classic BibTeX, and
ordinary text regeneration all hand it complete staged candidates.
`tex-command-stream`'s V2 manifest is the sole command-semantic schema,
capture-policy, route, channel, and xfail authority. The TeX82 catalogue gate
initializes the exact ordered 1..=1380 inventory from one typed deferred
disposition and applies sorted shard overrides. `corpus-manifest` remains a
zero-dependency external-corpus leaf.

The detached 173-line capture list, 467-line duplicate command-semantic census,
10,448-line V1 manifests, 11,047-line TeX82 disposition catalogue and its
generator branch, repeated consumer validators, candidate inventory
serializers, and direct PDF/text mutation paths were deleted. Exact migrated
projections pin all 203 command-semantic cases, 1,233 meaningful expected
strings, selected capture routes, statuses, xfails, channels, and interaction
policy. The TeX82 resolved-map digest and 946 reviewed / 434 deferred module,
106 covered / 45 gap property census preserve the predecessor result exactly.
Missing, extra, reordered, unsafe, traversal, source-closure, hash, schema,
local-edit, rollback, retry, and partial-publication cases remain executable.

Exact `--numstat` accounting is 1,676 additions and 1,173 deletions in authored
source and configuration (503-line net growth), plus 136 additions and 33
deletions in documentation (103-line net growth). Declarative fixtures add
6,834 and delete 21,244 lines (14,410-line net reduction); generated structural
schema evidence adds 443 and deletes 199 (244-line net growth); generated
lockfiles add 333 and delete eight (325-line net growth). The complete program
therefore adds 9,422 and deletes 22,657 lines, a 13,235-line net reduction. No
binary fixture changed. The declarative reduction exceeds its forecast, but the
authored category misses its 500-900-line reduction forecast; that separate
shortfall is tracked by `umber2-vgjr.23` rather than credited to repetitive
fixture deletion.

**Forecast reconciliation (owner decision, 2026-08-07).** Follow-up
`umber2-vgjr.23` audited every surviving Git inventory, typed contract,
validation, staging, serialization, command-semantic, TeX82 catalogue,
publication, caller, and proof boundary. It found no second authority and no
authored deletion with a named surviving owner and identical behavior. Git
checkout validation and local candidate validation deliberately defend
different trust boundaries; typed roles, source closure, status, profile, and
publication metadata are the common consumer contract; and fixturegen's
transaction owns mutation, rollback, and publication rather than repeating
either validator.

Reaching the forecast floor from the measured 503-line growth would require at
least 1,003 further net authored deletions. That inventory does not exist:
removing the retained contract and adversarial proof would weaken exact
membership, xfail, capture, census, traversal, local-edit, rollback, retry, or
partial-publication guarantees. The portfolio owner therefore retires the
original 500--900-line authored-reduction forecast and accepts the measured
1,676 additions and 1,173 deletions in authored source/configuration as Program
12's final result. No further reduction is carried for this program, and no
declarative fixture, generated schema, generated lockfile, moved, historical,
documentation, binary, or total-line change is credited as authored reduction.
The retained-authority audit, category accounting, and verification are
recorded in Beads issue `umber2-vgjr.23`.

Exact implementation tree `1aceb8ae8e5bd6b3f8fbf2a9a3a8fd2b961d128a`
passed the combined fixture-consumer selection and all 33 fixturegen tests
after uncapped `--no-run` builds and under `MemoryMax=512M`. The complete native
workspace suite and all four `scripts/check.sh` gates passed on the closeout
tree under `MemoryMax=1G`, with one Cargo build job for the quality gate.

## 13. Use one WASM wire schema, catalogue boundary, and session driver

**Outcome.** Explicit host-neutral DTOs own options, requests, responses, attempts, outputs, diagnostics, metrics, and stable error codes. TypeScript derives from those DTOs with explicit `Uint8Array` handling. One JS `SessionDriver` and `WorkerRpcClient` serve one-shot, editor, worker, and asynchronous resource sessions. `umber-distribution` validates raw catalogues and returns authenticated typed transport plans; JS performs fetch/cache/abort.

**Combines.** Luna ranks 17 and 18; Codex ranks 4 and 14.

**Counted reduction.** Approximately 2,600-3,600 scheduled Rust/JavaScript/test LOC across wire and catalogue publication after excluding acquisition in program 3 and HTML in program 7. Legacy public distribution API deletion adds 300-400 conditional LOC.

**Proof.** Preserve `Uint8Array`, safe integers, omitted fields, unknown-field policy, error codes/messages, worker containment, transfers, cancellation, request order, catalogue duplicate rejection, root/shard bytes and authentication, platform selection, HTML allowlists, offline use, and package API behavior.

**Session-driver implementation and accounting.** `SessionDriver` is the one
authored JavaScript retry, resource-delivery, progress, cancellation, and
disposal core for ordinary, project, editor, and worker-realm sessions.
`WorkerRpcClient` is the one request-correlation, timeout, owner-abort,
progress, message-error, and teardown core for one-shot and retained workers.
The public facades and worker messages remain adapters with their existing
names and shapes; manifest networking, authenticated selection, and persistent
cache policy remain in JavaScript outside both cores. Worker-realm session
preparation now has one binding, resolver-composition, and format-selection
path.

Exact implementation commit `1fd5e1c20` adds 473 and deletes 462 production
JavaScript lines (11 lines of net growth), adds five proof-test lines, and adds
four guidance lines. The complete commit is 482 additions and 462 deletions,
or 20 lines of net growth. It deletes 462 lines of duplicate orchestration but
does not claim them as a portfolio net reduction because the two explicit
replacement authorities contain 377 lines. The program-level reduction
forecast remains shared with the wire, catalogue, binding-migration, and API
retirement children.

The implementation tree passed the uncapped six-job native, wasm32, package,
and native-browser-bin builds; 89 authored JavaScript tests and the packaged
Node WASM project under a 512 MiB cap; the complete native suite and all four
`scripts/check.sh` gates under a 1 GiB cap with six Cargo jobs; and the complete
real Chrome package integration under a 1 GiB cap. That integration also
proved complete schema-1 diagnostic equality across the direct and worker
facades. Browser closeout exposed and separately repaired a pre-existing
read-only `SVGElement.className` assignment (`umber2-p8rn`) and stale packaged
Plain format (`umber2-em5o`). A provisioned Firefox executed 33 Rust/WASM tests;
29 passed and four stale binding/fixture expectations are tracked by
`umber2-3slp` independently of the JavaScript session driver.

**Implemented catalogue authority and accounting.** Commits `b51d23160`,
`6a7547e86`, and `83f8a6559` complete the catalogue/publication child
`umber2-vgjr.13.3`. `umber-distribution::NamedFormat` is the sole strict parser
and canonicalizer for publisher metadata and published format records. The
same crate partitions canonical root/shard values, computes their exact shard
digests, rejects duplicate and mispartitioned keys, and authenticates exact raw
shard bytes before returning one required-before-hint batch plan. The WASM
boundary exposes only prepared-batch, authenticated-plan, and named-format
operations. Authored JavaScript retains request/response identity adaptation,
HTTP, cache, concurrency, cancellation, budgets, and response materialization;
its JSON scanner, shard hash/partition logic, catalogue record parsers, and
per-key selection walk are deleted.

`texlive-wasm-publish::PreparedPublication` is the single full/HTML staging
path. Profile preparation selects the complete full tree or the HTML
allow-listed runtime/format closure and reviewed font records, after which one
path writes objects, constructs canonical shared catalogue values, prunes,
applies the profile inventory, and performs complete read-after-write
verification. The publisher-owned format DTO, closure canonicalizer, root and
shard aliases, duplicated staging flows, and 228-line executable HTML MVP
catalogue are deleted. The committed catalogue remains the reviewed data
authority and its exact WOFF2/license objects and links are authenticated during
every HTML publication.

Exact tree accounting from `64a0766be` through `83f8a6559` is 481 additions and
519 deletions in production Rust, plus 92 additions and 462 deletions in
production JavaScript and declarations. Proof tests add 72/delete 40 Rust lines
and add 76/delete 46 JavaScript lines. Authored Rust and JavaScript therefore
total 721 additions and 1,067 deletions, a 346-line net reduction. Documentation
and guidance add 31/delete 39 lines, and standalone lock metadata adds one line;
the complete tracked change adds 753 and deletes 1,106 lines, a 353-line net
reduction. No generated source, committed catalogue record, fixture, or binary
asset changed. The result is below the catalogue child's 800--1,100 baseline
forecast; `umber2-vgjr.13.5` tracks revalidation after the DTO migration rather
than crediting conditional legacy API deletion here.

The exact code tree passed the 17 distribution and 17 publisher tests under
`MemoryMax=512M`, the complete native routine suite under `MemoryMax=1G`, the
wasm32 check, Biome, all 89 authored Node tests, the built-package Node project
lifecycle, and `npm pack --dry-run` under `MemoryMax=1G`. All four
`scripts/check.sh` gates passed with six Cargo jobs under the same cap. The cold
release package build exceeded 1 GiB during compilation; an uncapped six-job
build completed, after which its Node lifecycle and package inventory passed
under the cap. Real wasm-bindgen and browser smoke remained blocked by absent
Firefox and Chrome.

**Implemented binding migration and accounting.** Commit `151d15d94`
completes `umber2-vgjr.13.4`. The schema-1 DTOs in `umber-wasm::wire` are now
the sole structural authority for session, project, editor, bibliography,
clock, limit, patch, request, response, attempt, output, diagnostic, metric,
observation, rendered-source, and catalogue-plan values. Binding adapters
perform one conversion between those DTOs and private engine values.
`serde-wasm-bindgen` preserves typed arrays and omitted optional properties.
The checked-in low-level TypeScript custom section is generated from the DTOs
and a Rust test requires byte equality with the generator; the handwritten
declaration block and manual `Object`/`Reflect` conversion tables were deleted.

Catalogue exports now return typed prepared batches, authenticated plans, and
named formats rather than JSON strings. The authored resolver passes raw shard
objects and consumes those plans directly, while retaining HTTP, persistent
cache, concurrency, cancellation, budget, and response-materialization policy.
The legacy schema-1 monolithic distribution parser and `select` API remain
only for the documented `texlive-wasm-publish --shard-existing` conversion and
publisher assembly path. They are not exposed through the browser boundary;
their compatibility disposition is recorded in `distribution_manifest.md`.

Exact implementation-commit `--numstat` accounting is 1,348 additions and
1,656 deletions in authored Rust, plus 30 additions and 28 deletions in
authored JavaScript, TypeScript, and JavaScript proof tests. Authored Rust and
JavaScript therefore total 1,378 additions and 1,684 deletions, a 306-line net
reduction. The generated TypeScript declaration adds 67 lines and is reported
separately. Documentation and guidance add 47 and delete 23 lines. The
complete tracked commit adds 1,492 and deletes 1,707 lines, or 215 lines of net
deletion. No fixture, catalogue record, lockfile, or binary asset changed.

The exact implementation tree passed the focused Rust tests, wasm32 check,
uncapped six-job package and native builds, complete native routine suite under
`MemoryMax=1G`, all 89 authored Node tests, packaged Node
TeX--bibliography--TeX lifecycle, and `npm pack --dry-run` under the same cap.
All four `scripts/check.sh` gates passed with six Cargo jobs under that cap.
These gates cover malformed messages, offline catalogue reuse, worker startup
and transfer, package behavior, and native resource replay. Real
wasm-bindgen/Firefox and browser-package/Chrome execution remained blocked
because neither browser executable is installed; the browser runner fails at
its explicit `/usr/bin/google-chrome` prerequisite before executing a case.

**Catalogue forecast reconciliation.** Follow-up `umber2-vgjr.13.5` audited
every remaining monolithic, publisher, WebAssembly, JavaScript, native, and
package caller after the DTO migration. It removes the dead public monolithic
`select` planner and pretty writer, their writer-only helpers, and the stale
selection fixture. The compatibility decision is intentionally narrower than
removing schema 1: `Manifest::parse` and its record model remain because the
documented `texlive-wasm-publish --shard-existing` offline conversion consumes
them, and the prepared publisher constructs the same model before canonical
sharding. The publisher's filesystem/readback adapter, native single-shard
adapter, the three typed WASM catalogue exports, and JavaScript's resource-key
and response adapters all have live callers and own host/package policy rather
than catalogue semantics. The two formerly duplicated sharded selection walks
now share one internal implementation.

The follow-up changes authored Rust by 17 additions and 369 deletions, a
352-line net reduction. Its retired 12-line selection case and one-line closed-
case inventory row are declarative evidence and are reported separately;
documentation and guidance are likewise excluded from authored Rust/JavaScript
credit. Together with `.13.3`, the catalogue/publication work changes authored
Rust and JavaScript by 738 additions and 1,436 deletions, a 698-line net
reduction. The original 800--1,100 baseline therefore overstates the proven
reduction by 102--402 lines. No generated declaration, moved implementation,
retained compatibility reader, or `.13.4` wire-schema deletion is credited to
the catalogue forecast, and no remaining catalogue reduction is carried
forward.

**Program closeout.** All six children are closed on implementation tree
`bd75c2cc1c476c25c0d30535d57518266c325f4f`. The final proof repair migrates
the wasm-bindgen suites to the schema-1 response and optional metric/status DTO
aliases without changing production code, adding 62 and deleting 32 authored
Rust test lines. Recomputing the final integrated children also includes the
12-addition/9-deletion direct/worker diagnostic proof and the
8-addition/3-deletion catalogue provenance repair omitted from the earlier
implementation-only receipts. Across the six children, authored Rust,
JavaScript, and proof tests add 3,880 and delete 3,633 lines, a 247-line net
increase. The generated
67-line TypeScript declaration and the 13 deleted declarative fixture/inventory
lines remain separate; documentation, guidance, moved code, compatibility
surfaces, catalogues, and binary assets are not credited as authored reduction.
The final catalogue-only reconciliation is +746/-1,439, a 693-line net
reduction and 107--407-line shortfall against its retired forecast. No further
Program 13 reduction is scheduled.

Fresh closeout verification compiled the full native suite, publisher, wasm32
tests, package, and native browser binary uncapped with six Cargo jobs. Under
`MemoryMax=512M`, distribution passed 17 tests, the publisher passed 17,
native catalogue/offline coverage passed 21, `umber-wasm` passed three, and all
89 authored Node tests passed. Under `MemoryMax=1G`, real
`wasm-pack test --node` passed its schema golden and two virtual-font tests,
the packaged TeX--bibliography--TeX lifecycle and 36-file npm dry run passed,
and the complete native routine suite passed. Chrome stopped precisely at the
absent `/usr/bin/google-chrome`; Firefox, GeckoDriver, Chromium variants, and
ChromeDriver were also absent, so no browser execution is reported. The
standalone publisher lock refresh remains the separately owned `umber2-ss53`
follow-up and was restored after the passing publisher gate. The exact
authority, compatibility, accounting, and verification receipt is recorded in
Beads issue `umber2-vgjr.13`.

## 14. Generate bibliography compatibility cases, then collapse production stages

**Outcome.** One compatibility-case manifest and immutable runner preserve separately named upstream assertions, inputs, outputs, order, and xfails. With that proof layer active, Biber uses one engine-owned editable draft and one freeze; classic retains its explicit-frame VM but removes duplicate lexer/compiler/callable/READ/report authorities; input and output stages lose intermediate models that are converted and discarded.

**Combines.** Luna rank 10; Codex rank 2 and secondary findings from `bib-input`, `bib-output`, and `bib-unicode`.

**Counted reduction.** Approximately 6,000-9,000 scheduled test Rust LOC plus 900-1,700 scheduled production LOC. Public legacy `Bib*` and Unicode APIs are excluded unless separately deprecated.

**Proof.** Preserve per-assertion selection and failure, exact Unicode and bytes, upstream order, xfails, field/source order, duplicate and inheritance policy, case-insensitive lookup, configuration precedence, XML/XInclude limits and errors, classic Web2C bounds and allocation traces, diagnostic order, BLG/BBL bytes, and generated filenames.

**Dependencies.** Generate the compatibility suite first. Never replace it with one mega-test or self-generated expected values.

**Implemented authority and accounting.** The pinned Biber identity remains
commit `74252e608e5f8115375c532eb25416430a9f52eb`; one typed manifest owns all
51 modules and 1,275 assertions, including 417 separately selectable generated
cases. The immutable runner compares cache-enabled and cache-disabled complete
results before applying committed expectations. `bib-engine::biber` owns the
single editable `BiberDraft` and sole immutable document freeze. Classic keeps
one explicit-frame VM while compiler-owned scanner state, indexed `READ`
projection, typed cache inputs, one callable value, and ordered log events
replace the deleted lexer transfer, string-keyed projection, debug cache key,
synthetic callable, and cloned diagnostic models. `bib-input` owns one flat
include-aware XML projection arena, and `bib-output::OutputRouter` owns one
closed `OutputPlan` and bounded sink. Public result aliases, serializer entry
points, format-specific failure aliases, and Unicode values remain as
compatibility surfaces; none owns a second mutation, freeze, selection, or
serialization authority.

Exact `--numstat` accounting across implementation commits `1250f6a6a`,
`91e227ee8`, `4a495ece0`, `4e8faebce`, `8290c9274`, and `8c7c49691` is 1,334
additions and 1,743 deletions in production Rust (409-line net reduction), plus
3,017 additions and 7,429 deletions in authored Rust proof tests (4,412-line
net reduction). Authored Rust therefore totals 4,351 additions and 9,172
deletions, or 4,821 lines of net deletion. Documentation and repository maps
add 140 lines and delete 44. No fixture, generated source, binary asset, or
declarative configuration changed. The complete tracked program change is
4,491 additions and 9,216 deletions, or 4,725 lines of net deletion. The
production and test reductions are below their respective forecast floors;
the remaining opportunity or explicit forecast revision is tracked by
`umber2-vgjr.24` rather than credited here.

**Forecast reconciliation (owner decision, 2026-08-07).** Follow-up
`umber2-vgjr.24` audited every surviving Biber draft/freeze, classic
compiler/READ/VM/log, XML projection, output router/sink, compatibility facade,
caller, and proof boundary. It found no second writable or serialization
authority and no deletion with a named surviving owner and identical behavior.
The compatibility serializers and result/failure names are thin delegates or
aliases; classic artifact construction remains backend-specific by contract;
and the 417 generated cases retain unique expected values within the exact
51-module/1,275-assertion proof rather than duplicate test authority.

Reaching the original floors would require another 491 net production and
1,588 net test deletions. That inventory does not exist without removing a
retained public API, collapsing deliberately separate Biber/classic/input/output
semantics, weakening cache-off or byte/diagnostic/Unicode/generated-file proof,
or merely moving case data into an excluded category. The portfolio owner
therefore retires the 900--1,700 production and 6,000--9,000 test forecasts and
accepts the measured 409-line production and 4,412-line test reductions as the
final Program 14 result. No shortfall or unimplemented deletion is carried
forward, and no moved, generated, declarative, fixture, documentation, or total
line count is credited. The audit and independently reproduced accounting are
recorded in Beads issue `umber2-vgjr.24`.

Exact implementation tree `8c7c496916454ed6df47e628d1692e817a20e510`
passed the focused bibliography suite under `MemoryMax=512M`, the complete
routine suite under `MemoryMax=1G`, the Node WASM tests and packaged
TeX--bibliography--TeX lifecycle under `MemoryMax=1G`, and all four
`scripts/check.sh` gates with `CARGO_BUILD_JOBS=1`. One preceding full-suite
attempt produced an invalid story DVI despite a byte-correct provisioned
oracle; the exact test and unchanged full suite then passed. That independent
flake is tracked by `umber2-dgsn` and is not counted as a successful first
attempt.

## 15. Recover dormant tests, then compact repeated test authorities

**Outcome.** Every unreachable `tex-exec` test and every proposed command/typesetting test deletion receives a case-level ledger naming semantic assertions, fixtures, expected diagnostics/events, external citations, active replacement, and disposition. Unique cases become active compact integration/oracle/property tests. Redundant cases and their production scaffolding are deleted. A source audit rejects `cfg(any())` and colocated tests while `[lib] test = false` remains.

**Source.** Codex ranks 1 and 7 plus the test portion of rank 8. Luna did not identify the dormant `tex-exec` island.

**Counted reduction.** Conditional only: 15,000-17,400 authored LOC in `tex-exec`, 1,250-2,400 in command tests, and 700-950 in typesetting tests. Actual net is recomputed from the completed ledgers.

**Proof.** Preserve diagnostic bytes, failure granularity, rollback, resource suspension, alignment, insertion/page output, shipout, scanner identity, TeX rule coverage, documentation citations, ignored reasons, and independently selectable failures.

**Dormant tex-exec closure.** Commits `c8d7b8615`, `a5bee234b`, and
`bfbe33682` close `umber2-vgjr.15.2` and its catalogue-reachability child.
The original ledger's 447 cases resolve to 445 active crate-internal tests and
two named predecessor dispositions. All 64 `cfg(any())` sites and every source
audit exception are gone; compiler-backed cleanup removed callerless
scaffolding. The 35 affected TeX82 properties and 46 original dormant paths
now resolve through active assertion-complete evidence, with eight additional
repaired-boundary links and six stale gaps promoted only after their active
assertions landed. The combined Rust change adds 264 and deletes 652 lines, a
measured net deletion of 388 lines; the execution migration itself accounts
for 245 additions and 650 deletions, a net deletion of 405 lines. Active tests
pin exact diagnostics, effects, DVI output, insertion operands, and fatal
checkpoint replay.

**Command scanner/delivery compaction.** Commits `b03e4b49d` and `bfac58c56`
close `umber2-vgjr.15.3`. The closed
`command_assertion_ledger.md` maps scalar and structured values, literal token
and event order, exact recovery text and context, scanner lifecycle, rollback
and retry state, source framing, and identity-sensitive state to active owners.
Its narrative/matrix audit found no assertion-bearing case with a complete
replacement, so this child deletes zero semantic cases and retains every such
test. The only retired material is proven duplicate setup scaffolding.

`ProcessorScenario` owns a newly constructed mutable command state, universe,
and host-capability set; `ScannerRig` adds a newly constructed observation
recorder. Neither helper caches or shares state. Representative structured and
delivery cases now use those shallow owners, while the scalar, token-list, and
expansion suites share the same traced-token, ordinary-level, processor,
recorder, and diagnostic helpers without changing inputs or assertions. The
Rust change adds 291 and deletes 349 lines, a measured net deletion of 58
lines. Ledger and repository-map documentation adds 44 lines; no fixture,
generated expectation, digest, or production source changes.

**Typesetting/browser and source-authority closure.** Commits `61bc3793e`,
`8c6350dbe`, and `593c2c4a2` close `umber2-vgjr.15.4`. The typesetting ledger's
three aggregate retirements remain assertion-complete through active rule
owners. Exact rule matrices, 20,000-deep math and discretionary cases, the
100,000-node line-break bound, HTML bytes and semantic projections, WASM wire
values, worker containment, hostile DOM input, identity/lifetime behavior, and
real-browser fixtures remain independent evidence. One dormant wire case is
retired; its unique declaration and diagnostic cases are active WASM tests,
and the accepted `9_007_199_254_740_991` plus rejected
`9_007_199_254_740_992` boundaries cross the actual `JsValue` DTO. The Rust
proof-test change adds 13 and deletes 41 lines, a measured net deletion of 28.

Commit `4b3db8016` closes `umber2-vgjr.15.5`. The routine source audit scans
tracked production Rust using Cargo library-target metadata, rejects
unconditionally false `cfg(any())` attributes and test modules under disabled
library test targets, rejects stale exceptions, and carries an empty exception
set. Its positive and negative controls are active in the workspace-selection
executable.

**Program accounting and closeout.** Base-normalized child diffs add 879 and
delete 1,042 authored Rust lines, a measured net deletion of 163. The three
semantic compaction children contribute 474 lines of net deletion; the
recurrence audit adds 311 lines. Documentation and guidance add 1,614 and
delete 619 lines, while Cargo/property-catalogue declarative data adds 67 and
deletes 51. No generated source, fixture payload, JavaScript, or binary asset
changes receive credit. The original 447-case ledger closes as 445 active
tests plus two explicit retirements; all 64 inactive sites are gone. The 94-row
TeX82 audit retains 29 explicit gap-owned rows, and all 54 links across the 35
formerly dormant-citing properties resolve to active tests.

## 16. Consolidate PDF test support without weakening independent evidence

**Outcome.** Canonical structure projection walks Hayro's borrowed objects directly. Focused raw queries replace the copied `PdfProbe` graph. Ordinary valid synthetic inputs use `pdf-writer`; explicit raw-byte helpers retain malformed, classic-xref, cycle, depth, and independent-writer cases.

**Source.** Codex rank 18, modified by Luna's independent-oracle objection.

**Counted reduction.** Approximately 700-1,000 scheduled Rust/test LOC.

**Proof.** Preserve parser independence where it is the test's purpose, object/page order, xref and object streams, deterministic cycle labels, unresolved references, inherited resources, raw versus decoded streams, operation order, budgets, and intentionally malformed inputs.

**Implemented authority and accounting.** Commits `6dbcf5ce7`, `e4ff291a1`,
`a0ab55cb1`, `593e54150`, and `0982c358b` establish `PdfQuery` and
`normalize_structure` as the sole Hayro-backed semantic authorities. Query
values, arrays, dictionaries, pages, and inherited resource layers are shallow
borrowed handles; only explicitly requested stream bytes and decoded operation
operands are owned projections. `ValidPdfFixture` delegates ordinary valid PDF
framing, stream lengths, xrefs, and trailers to `pdf-writer`. The handwritten
`RawPdfFixture` remains only in canonical cycle/classic-trailer normalization,
focused malformed/deep/unresolved/classic-xref query coverage, and the importer
depth-limit rejection. The copied `PdfProbe` graph, its recursive projection
walks, the `pdf_probe` module and compatibility vocabulary, the old full valid
writer, and its 217-line self-test suite are deleted. No Rust consumer or
manifest retains the predecessor names or a `lopdf` dependency.

The final query and normalizer are larger because the accepted replacement
adds explicit depth/object/value/stream budgets, stable cycle and unresolved
identity, inherited resources, raw/decoded stream evidence, and ordered decoded
operations. Against the pre-program tree `d5f1a111d`, the exact scoped program
diff adds 1,751 and deletes 1,448 authored Rust lines (303-line net growth),
adds 109 and deletes 38 documentation/guidance lines (71-line net growth), and
adds two dependency-configuration lines. The complete tracked change therefore
adds 1,862 and deletes 1,486 lines, a 376-line net growth; an unchanged binary
xref/object-stream fixture was moved with its module. Within that total, fixture
writer support falls from 651 to 565 Rust lines (86-line net deletion), while
the query/normalization authority grows from 1,333 to 1,688 (355-line net
growth) and migrated consumers grow by 34 net lines. The scheduled 700--1,000
line reduction was not achieved and is not credited as deletion.

Exact integrated implementation tree `0982c358b72e8e338d94ec2436c85e4f42ccc6e5`
passed the focused `test-support`, `tex-out`, Umber PDF library, and PDF parity
tiers under `MemoryMax=512M`, the complete native suite under `MemoryMax=1G`,
and all four `scripts/check.sh` gates. The local external PDF gate explicitly
reported qpdf and both Poppler tools unavailable, so it skipped those matrices
rather than counting them as passes; their pinned CI gate remains
`scripts/check-pdf-external.sh --ci`.

## 17. Contract VFS only after the multipass roadmap is fixed

**Outcome.** If the project confirms that TeX -> bibliography -> TeX orchestration continues to publish one orchestrator-owned generated set, replace test-only multi-stage/build-plan machinery with one `GeneratedTransaction` and private shape-safe user/resolved/generated maps. Otherwise retain multi-stage semantics and simplify only internal representation.

**Source.** Codex rank 20, constrained by Luna's broader resource lifecycle.

**Counted reduction.** Conditional 750-1,000 authored LOC.

**Proof.** Preserve invisible writes until accept, drop rollback, whole-set replacement, count/byte limits, stale snapshots, COW generations, lookup precedence, ordering, retained-byte accounting, immutable conflicts, and shared bytes.

**Dependencies.** Program 3's request/admission lifecycle must land first. The roadmap decision belongs in the governing VFS and persistent-session documents.

**Implemented authority and accounting.** The roadmap decision in
`c0af1fa22` keeps TeX--bibliography--TeX pass scheduling, convergence, and
private complete-set assembly in `LatexProjectSession`; the VFS receives only
the final set. Commits `78706f0e5` and `b2b404fc3` implement one
`ProjectWorkspace`-owned `GeneratedTransaction`. Its private candidate map is
the sole pending-publication authority, while the durable copy-on-write
generation has exactly three private, differently typed maps for user,
resolved-resource, and accepted-generated files. Their narrow constructors
enforce root and origin shape. No second scheduling, pending-map, or generic
storage authority remains.

The retired predecessor comprises `VirtualFs`, `BuildPlan`,
`BuildTransaction`, `StageTransaction`, declared replacement and
invalidation, producer/build/stage identities, public `LayerKind`,
`FileLayer`, and `LayeredFileStorage`, and their test-only multistage and
generic-construction fixtures. Repository caller inventory proved every
production VFS build had one stage before deletion. Retained Rust and WASM
session adapters keep their attempt, resource, output, and retry contracts;
there is no deprecated multistage shim or pass-plan wire type.

The two implementation commits add 385 and delete 949 production Rust lines
(564-line net deletion), add 321 and delete 629 Rust test lines (308-line net
deletion), and add 86 and delete 98 documentation/guidance lines (12-line net
deletion). They therefore add 792 and delete 1,676 authored lines, an 884-line
net deletion inside the forecast range. Including the decision commit, the
complete program adds 706 and deletes 1,578 Rust lines (872-line net deletion)
and adds 179 and deletes 111 documentation/guidance lines (68-line net
growth), for 885 additions, 1,689 deletions, and 804 lines of total net
deletion. Declarative/generated records and binary assets are unchanged.

Exact combined implementation tree
`b2b404fc32b04c0926e99750eab5c30dbff21b45` passed focused VFS and production
caller tests after an uncapped `--no-run` build and under `MemoryMax=512M`.
The complete native workspace passed after its uncapped `--no-run` build and
under `MemoryMax=1G`. The wasm32 target, all 89 authored Node tests, and the
optimized packaged Node TeX--bibliography--TeX lifecycle passed under
`MemoryMax=1G`; all four `scripts/check.sh` gates passed with one Cargo build
job under the same cap. The unavailable browser-driver environment remains
tracked separately by `umber2-5zie` and does not replace the passing Node
lifecycle evidence.

## 18. Retire package, migration, benchmark, and prototype surfaces only by decision

**Outcome.** After programs 2 and 12 settle ownership, fixturegen contains the minimal reference kernel, parity is comparison-only, and `refexec` may become a compatibility command or retire. Completed fixture-layout migration commands may retire only after their documented compatibility is dropped. Benchmarks and traces are classified as active gate, diagnostic, or historical; only historical surfaces are deleted.

**Combines.** Luna ranks 16, 19, and 20; Codex rank 21 and its explicit benchmark rejection.

**Counted reduction.** Conditional 650-850 LOC for parity/refexec package and CLI retirement, 700-1,000 for fixture-layout migration retirement, and 900-1,400 for maintainer-approved historical benchmark/trace/prototype deletion. Moved code and lockfile churn are excluded.

**Proof.** Preserve command compatibility or record retirement, deterministic reference flags/environment/staging, DVI normalization and triage, fixture publication and rollback, all active performance budgets, reproducible investigations, and the Cargo-fuzz boundary.

**Implemented authority and accounting.** `fixturegen::reference` is the sole
TeX/TFtoPL process, staging, environment, flag, output, status, and
manifest-hash kernel. `fixturegen --reference-dvi` and
`fixture_transaction.rs` own reference-fixture and atomic cohort publication;
`refexec` and parity's live API/CLI remain compatibility/composition layers,
`test-support` owns generic DVI equality, and `tex-command-stream` owns semantic
comparison. The completed layout migration commands and registries retired by
explicit compatibility decision. Layout/width workloads moved to
`benchmarks/tex-typeset`, pure-memo edit moved to `benchmarks/tex-incr`, and
shipout remains with `benchmarks/tex-exec`. Every other inventoried benchmark,
trace, profile, fuzz, fixture, and public API surface remains with its named
owner.

The cumulative credited authored retirement is 1,980 lines: 46 lines of
duplicate parity reference recipes, 1,446 one-time fixture-migration production
and test lines, and 488 historical benchmark/prototype lines. This deliberately
excludes the relocated 284-line reference implementation, 941 moved transaction
and ordinary-publication test lines, 583 moved benchmark/baseline lines, and all
generated lockfile churn. Raw child implementation `--numstat` is 4,025
additions and 3,132 deletions, but is not used as reduction credit because it
mixes those moves, generated locks, proof, documentation, and owner scaffolding.
No fixture or benchmark baseline bytes changed.

**Program closeout.** All four children are closed. The fresh owner/caller audit
found no undeclared retirement or second reference/publication authority and
repaired one retained `tex-state` benchmark projection after the engine added
the canonical internal-control-sequence kind. Reference, publication,
comparison, affected benchmark, snapshot, scripted-fuzz, full native, and
quality gates pass under the closeout protocol. The unchanged foreign-host
width timing baseline and current layout-allocation ceilings remain explicitly
red and owned by `umber2-9508` and `umber2-dtis`; neither baseline was rewritten.
The exact command and accounting receipt is recorded in Beads issue
`umber2-vgjr.18`.

## Final portfolio reconciliation

The portfolio closed on integrated tree
`bd50a474138ec0f13f3c76caf6453113872fefd0`. All 18 selected program epics,
all 29 direct children of `umber2-vgjr`, and all 99 descendants are closed.
The final audit found no retained migration-only authority. Each surviving
adapter has a distinct compatibility, host, backend, validation, projection,
or proof role recorded in its program section and issue receipt.

The measured program results, not the planning ranges, are final. In
particular, the portfolio owner retires every remaining forecast variance at
the category-separated result recorded above. This includes Program 9's
`+2,475/-1,272` authored-Rust result (net growth 1,203; the 602-line retired-
consumer reduction is not whole-program credit), Program 16's
`+1,751/-1,448` authored-Rust result (net growth 303), and Program 18's 1,980
lines of explicitly approved authored retirement. Program 18's 1,808 moved
lines and generated lockfile churn remain excluded. No unimplemented
reduction, compatibility-gated deletion, test compaction, benchmark
retirement, moved implementation, generated/declarative change,
documentation change, or binary asset change is carried forward or recast as
authored deletion.

The raw mainline interval from the portfolio plan commit `0bf7219ea` through
the integrated closeout tree changes 828 paths by 57,017 additions and 86,766
deletions. Rename-aware category accounting is authored source
`+40,007/-63,660`, documentation and guidance `+6,597/-1,247`,
declarative/fixture/configuration text `+7,569/-21,607`, and generated
lockfiles `+2,844/-252`; two binary paths are excluded from line totals. This
interval is a repository reconciliation, not deletion credit: it contains 215
mainline commits, including independently tracked repairs and work outside the
18 programs. Program credit remains the non-overlapping issue-scoped
accounting in the sections above.

The final acceptance receipt is recorded in Beads issue `umber2-vgjr`. It
records the child audit,
superseding safety protocol, exact capped proof and allocation measurements,
full native result, and repository quality result.

## Dependency-aware execution plan

### Wave 0: decisions, baselines, and ledgers

Record the paragraph-replay replacement contract and baseline. Decide public Rust API policy, CLI and fixture-migration compatibility, the VFS single-stage roadmap, and benchmark ownership. Freeze wire, event, DVI, PDF, HTML, format, catalogue, bibliography, and resource fixtures. Start the dormant-test and test-compaction ledgers without deleting evidence.

### Wave 1: small authoritative contracts

Land the primitive catalogue, oracle event views, command-semantic V2 reader, implicit TeX82 dispositions, Biber compatibility manifest/runner, PDF valid-fixture adapter, shared verified downloader, and WASM DTO fixtures. These changes establish proof surfaces and remove isolated duplication without changing core execution.

### Wave 2: state, execution, and incrementality

Retire paragraph replay under its approved contract. Establish the executable node schema and production-only format restoration. Introduce the effect journal. Remove the command runtime capability, migrate assignment families to typed receipts, then merge ordinary and observed operation flow. Close the executor revision patch after the durable post-replay revision shape is known.

### Wave 3: typesetting and detached outputs

Build the native math transaction, paragraph tape, and shared metrics cursor. Introduce the artifact cursor, then the geometry walker, then switch fresh DVI to canonical artifacts. Canonicalize TFM before lazy MATH. Move PDF finalization into `tex-out`. Build the shared HTML render document and keep JavaScript as receiver.

### Wave 4: resource, distribution, and browser orchestration

Migrate callers to the canonical resource lifecycle. Establish the sharded catalogue/publication plan and source-aware acquisition client. Replace manual WASM conversions, then unify the JS session and worker RPC drivers. Delete unused Rust resource planes only after compatibility policy closes.

### Wave 5: bibliography, fixtures, and tooling

Use the generated compatibility cases to simplify production bibliography stages. Complete the typed fixture contract and migrate parity/tooling consumers. Apply public API, CLI, VFS, fixture-migration, benchmark, and prototype retirements only when their decisions are recorded.

### Wave 6: evidence compaction

After the new authorities are active, close the case-level ledgers and delete redundant dormant, command, typesetting, browser, PDF, and bibliography tests. Remove each predecessor in the same issue that establishes its replacement; do not accumulate permanent migration shims.

## Beads rollout

Create one top-level Beads epic for the combined portfolio and one child epic per master program selected for implementation. Each implementation issue must record:

- the surviving authority and the predecessor being deleted;
- the exact source snapshot used for its estimate;
- overlap exclusions with neighboring programs;
- compatibility or owner decision, if any;
- differential and performance gates;
- measured gross additions, deletions, and net result at closure.

Do not turn every subheading into an issue at once. Open the next dependency wave only after the preceding authority is merged or its interface is stable enough for parallel consumers.

## Coverage of all reviewed packages

| Package                  | Combined disposition                                                |
| ------------------------ | ------------------------------------------------------------------- |
| `bib-engine`             | Programs 14 and 15                                                  |
| `bib-input`              | Program 14                                                          |
| `bib-model`              | Program 14; immutable boundary retained                             |
| `bib-output`             | Program 14; full-golden deletion remains gated                      |
| `bib-unicode`            | Program 14; public compatibility API retained by default            |
| `corpus-manifest`        | Program 12; independent dependency-free package retained            |
| `fixturegen`             | Programs 12 and 18                                                  |
| `parity-harness`         | Programs 2, 12, and 18                                              |
| `png-import-prototype`   | Retired by `umber2-vgjr.18.4` after owner decision and caller audit |
| `profile-analyzer`       | No qualifying reduction; retain specialized reporting               |
| `refexec`                | Program 18; compatibility gate required                             |
| `test-support`           | Programs 12 and 16                                                  |
| `tex-arith`              | No qualifying reduction; retain shared exact arithmetic leaf        |
| `tex-command-benchmarks` | Program 18; active workload retained with its measured owner        |
| `tex-command-stream`     | Programs 2, 9, 12, and 18                                           |
| `tex-command`            | Programs 1, 9, and 15                                               |
| `tex-content`            | No qualifying reduction; retain stable identity leaf                |
| `tex-exec-benchmarks`    | Program 18; shipout retained, mixed-owner rows rehomed              |
| `tex-exec`               | Programs 1, 3-6, 9, and 15                                          |
| `tex-fonts`              | Programs 6, 7, and 11                                               |
| `tex-incr`               | Programs 3, 4, and 18                                               |
| `tex-observe`            | Programs 1 and 2                                                    |
| `tex-oracle`             | Programs 2 and 12                                                   |
| `tex-out`                | Programs 5-7 and 16                                                 |
| `tex-state-benchmarks`   | Program 18; active gates and diagnostics retained                   |
| `tex-state`              | Programs 3, 4, 8-11, and 15                                         |
| `tex-typeset`            | Programs 10 and 15                                                  |
| `texlive-wasm-publish`   | Program 13                                                          |
| `umber-distribution`     | Programs 3 and 13                                                   |
| `umber-fetch`            | Program 3                                                           |
| `umber-fuzz`             | No qualifying reduction; retain isolated TFM fuzz target and seed   |
| `umber-interrupt`        | No qualifying reduction; retain unsafe-FFI quarantine               |
| `umber-vfs`              | Programs 3 and 17                                                   |
| `umber-wasm`             | Programs 3, 7, and 13                                               |
| `umber`                  | Programs 3, 4, 6, 7, 13, and 18                                     |

## Explicit non-goals

- Do not merge crates only to reduce package count.
- Do not replace exact bounded codecs with serde, bincode, recursive trees, or generic parser frameworks when wire bytes, rejection order, memory bounds, or provenance are observable.
- Do not delete caches, secure workers, incremental convergence, DVI plans, HTML identity, profiling tools, fuzzing, or benchmarks without the named functional or performance decision.
- Do not count expected bytes moved into snapshots, generated Rust, root lockfiles, or manifests as authored-code deletion.
- Do not implement both sides of a resolved conflict: one browser receiver, resource lifecycle, event comparison owner, reference kernel, primitive catalogue, effect ledger, and fixture publication authority are the target.

## Completion criterion

The portfolio succeeds when each implemented program leaves one named authority, deletes its obsolete predecessor, preserves or explicitly revises every governed contract, passes its differential and performance gates, and records a measured net reduction. Architectural motion without predecessor deletion does not count as completion.
