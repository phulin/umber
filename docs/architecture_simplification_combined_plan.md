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
The reduction shortfall is tracked by `umber2-vgjr.20`; no additional deletion
is silently credited to this program.

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
forecast shortfall is tracked by `umber2-vgjr.21`; no historical deletion or
future compatibility-gated retirement is silently credited to this program.

## 5. One artifact codec and geometry traversal authority

**Outcome.** One iterative validated node-event cursor/emitter owns the versioned artifact grammar. Owned decode, zero-copy DVI planning, scan, validation, and production adapt to it. One explicit-frame geometry walker owns boxes, glue, leaders, snapping, ordinals, and sibling lookahead; DVI and positioned sinks retain backend policy. Fresh and memo-hit DVI derive from canonical artifact bytes.

**Combines.** Luna rank 7; Codex rank 10.

**Counted reduction.** Approximately 1,450-1,900 scheduled production LOC after counting the `tex-exec` dual-emission materializer only once.

**Proof.** Preserve artifact v23 and legacy bytes, error precedence and limits, nonrecursive replay, Unicode/classic validation, ligature source units, DVI movement/font/leader bytes, positioned effects, throughput, and RSS. The extra fresh-page byte pass has an explicit performance stop gate.

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
detached input. Differential coverage freezes two independent legacy runs,
compares the complete inputs, and verifies the legacy finalizer still produces
identical bytes; the next migration step may therefore move lowering without
introducing `Universe`, `World`, or resolver callbacks into `tex-out`.

## 7. One canonical HTML producer and JavaScript receiver

**Outcome.** A keyed `RenderDocument` or `RenderRevision` resolves positioned events, fonts, specials, accessibility, and math once. Standalone HTML/assets and incremental patch plans derive from it. JavaScript remains the browser trust and DOM transaction boundary.

**Combines.** Luna rank 8; Codex rank 19 and the render portion of rank 4.

**Counted reduction.** Approximately 300-500 scheduled producer LOC. Retiring the unused exported Rust receiver adds 550-700 conditional LOC after compatibility review.

**Proof.** Preserve exact standalone bytes, event ordinals, resource order and identity, accessibility, math glyphs, stable DOM identity, focus/selection/scroll, atomic rollback, leases, validation limits, CSP, and large-patch performance.

## 8. Make state and node schemas executable and singular

**Outcome.** A `NodeRef`-centered exhaustive schema owns tags, semantic fields, handles, ordered children, remapping, equality, validation, hashing, copy, and format projection while compact storage remains specialized. Production frozen-format decode is the only restoration authority. Internally, `Universe` becomes the sole state façade and private `Stores` becomes field-oriented data.

**Combines.** Luna ranks 13 and 14; Codex ranks 6 and 12.

**Counted reduction.** Approximately 1,000-1,500 scheduled production/test LOC for the node schema and test-only format path. Removing public expansion/store forwarding adds 2,100-2,700 conditional LOC after API policy or a deprecation adapter.

**Proof.** Preserve schema tags and versions, origin exclusions, child order, allocation-free views, compact sidecars, survivor patching, malformed-reference rejection, lookup rebuilding, group invalidation, dependency observation, capability restrictions, state hashes, and nonrecursive behavior.

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
rationale are recorded in [the issue writeback](writeback/umber2-vgjr.11.4.md).
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

## 16. Consolidate PDF test support without weakening independent evidence

**Outcome.** Canonical structure projection walks Hayro's borrowed objects directly. Focused raw queries replace the copied `PdfProbe` graph. Ordinary valid synthetic inputs use `pdf-writer`; explicit raw-byte helpers retain malformed, classic-xref, cycle, depth, and independent-writer cases.

**Source.** Codex rank 18, modified by Luna's independent-oracle objection.

**Counted reduction.** Approximately 700-1,000 scheduled Rust/test LOC.

**Proof.** Preserve parser independence where it is the test's purpose, object/page order, xref and object streams, deterministic cycle labels, unresolved references, inherited resources, raw versus decoded streams, operation order, budgets, and intentionally malformed inputs.

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

| Package                  | Combined disposition                                              |
| ------------------------ | ----------------------------------------------------------------- |
| `bib-engine`             | Programs 14 and 15                                                |
| `bib-input`              | Program 14                                                        |
| `bib-model`              | Program 14; immutable boundary retained                           |
| `bib-output`             | Program 14; full-golden deletion remains gated                    |
| `bib-unicode`            | Program 14; public compatibility API retained by default          |
| `corpus-manifest`        | Program 12; independent dependency-free package retained          |
| `fixturegen`             | Programs 12 and 18                                                |
| `parity-harness`         | Programs 2, 12, and 18                                            |
| `png-import-prototype`   | Program 18; retirement decision required                          |
| `profile-analyzer`       | No qualifying reduction; retain specialized reporting             |
| `refexec`                | Program 18; compatibility gate required                           |
| `test-support`           | Programs 12 and 16                                                |
| `tex-arith`              | No qualifying reduction; retain shared exact arithmetic leaf      |
| `tex-command-benchmarks` | Program 18; active workload retained or rehomed                   |
| `tex-command-stream`     | Programs 2, 9, 12, and 18                                         |
| `tex-command`            | Programs 1, 9, and 15                                             |
| `tex-content`            | No qualifying reduction; retain stable identity leaf              |
| `tex-exec-benchmarks`    | Program 18; active workloads retained                             |
| `tex-exec`               | Programs 1, 3-6, 9, and 15                                        |
| `tex-fonts`              | Programs 6, 7, and 11                                             |
| `tex-incr`               | Programs 3, 4, and 18                                             |
| `tex-observe`            | Programs 1 and 2                                                  |
| `tex-oracle`             | Programs 2 and 12                                                 |
| `tex-out`                | Programs 5-7 and 16                                               |
| `tex-state-benchmarks`   | Program 18; maintainer retirement decision required               |
| `tex-state`              | Programs 3, 4, 8-11, and 15                                       |
| `tex-typeset`            | Programs 10 and 15                                                |
| `texlive-wasm-publish`   | Program 13                                                        |
| `umber-distribution`     | Programs 3 and 13                                                 |
| `umber-fetch`            | Program 3                                                         |
| `umber-fuzz`             | No qualifying reduction; retain isolated TFM fuzz target and seed |
| `umber-interrupt`        | No qualifying reduction; retain unsafe-FFI quarantine             |
| `umber-vfs`              | Programs 3 and 17                                                 |
| `umber-wasm`             | Programs 3, 7, and 13                                             |
| `umber`                  | Programs 3, 4, 6, 7, 13, and 18                                   |

## Explicit non-goals

- Do not merge crates only to reduce package count.
- Do not replace exact bounded codecs with serde, bincode, recursive trees, or generic parser frameworks when wire bytes, rejection order, memory bounds, or provenance are observable.
- Do not delete caches, secure workers, incremental convergence, DVI plans, HTML identity, profiling tools, fuzzing, or benchmarks without the named functional or performance decision.
- Do not count expected bytes moved into snapshots, generated Rust, root lockfiles, or manifests as authored-code deletion.
- Do not implement both sides of a resolved conflict: one browser receiver, resource lifecycle, event comparison owner, reference kernel, primitive catalogue, effect ledger, and fixture publication authority are the target.

## Completion criterion

The portfolio succeeds when each implemented program leaves one named authority, deletes its obsolete predecessor, preserves or explicitly revises every governed contract, passes its differential and performance gates, and records a measured net reduction. Architectural motion without predecessor deletion does not count as completion.
