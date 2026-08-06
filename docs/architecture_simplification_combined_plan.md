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

**Dependencies.** The contract precedes program 13's browser orchestration and program 17's VFS contraction. Do not wait for API retirement to begin the canonical internal lifecycle.

## 4. Retire paragraph replay, then establish one revision transaction and effect log

**Outcome.** Named restart checkpoints and suffix convergence become the only cross-revision reuse mechanism. After replay-only fields are removed, one immutable revision payload and transaction owns restart identity, resources, effect bundles, artifact rows, DVI plans, convergence, and publication. `tex-state` exposes an `EffectJournal`; executor commit closes a validated `RevisionOutputPatch`; `tex-incr` applies prefix, patch, and validated suffix.

**Combines.** Luna ranks 4 and 5; Codex rank 9.

**Counted reduction.** Approximately 1,400-2,100 scheduled authored LOC for effect/publication consolidation, plus 1,000-1,700 LOC from the separately approved paragraph-replay retirement.

**Proof.** Before replay deletion, record the replacement performance contract and fixed baseline. Preserve cold-equivalent artifacts, effects, resources, state hashes, rollback, pruning, accepted prefix/suffix boundaries, recursive output, terminal phases, OpenOut positions, suspension safety, and two-phase prepare/accept.

**Dependencies.** Replay retirement fixes the durable revision shape and therefore precedes final revision payload publication. State façade work in program 8 may be staged first.

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

## 10. Replace typesetting shadow arenas and repeated topology with native authorities

**Outcome.** A detached native-node transaction replaces the second math arena. `ParagraphTape` owns `NodeSequence`, analyzed break sites, prefix metrics, trace ranges, and materialization actions. A shared metrics cursor supplies packing, line breaking, vertical contributions, and math without erasing domain-specific policy.

**Source.** Codex rank 8. Luna treated its smaller constituent proposals as individually sub-threshold; the combined transaction/tape program clears that bar.

**Counted reduction.** Approximately 900-1,250 scheduled production LOC. Another 700-950 test LOC is conditional on a rule-by-rule assertion ledger.

**Proof.** Preserve 20,000-depth stack safety, occurrence-ordered observations, selected OpenType glyphs, source-box geometry, semantic versus physical paragraph channels, discretionary topology, overflow behavior, trace routes, glue policy, and hot-path performance.

## 11. Publish one canonical font runtime while preserving format-specific policy

**Outcome.** TFM parsing retains raw tables only through reference and error-precedence validation, then publishes canonical `FontMetrics` through one loaded-font constructor. OpenType MATH uses one strict eager validation walk and lazy borrowed queries through the existing scaled facade. A realized font identity feeds HTML, PDF, incrementality, and distribution boundaries without repeated decoding.

**Combines.** Luna rank 9; Codex rank 17.

**Counted reduction.** Approximately 800-1,200 scheduled production LOC. Raw public TFM/MATH model deletion adds 150-250 conditional LOC after compatibility handling. Binary fixture subsetting remains outside LOC accounting.

**Proof.** Preserve TFM error identity and precedence, lig/kern and absent-character rules, `font_info_words`, parameter padding, MATH strictness and budgets, variation/device policy, shaping, fallback precedence, glyph numbering, PDF subset identity, HTML paths, cache identity, and fuzz coverage.

## 12. Establish one fixture contract while compacting repeated catalogues

**Outcome.** A typed closed-case contract owns identity, tracked inputs, expected outputs, statuses, xfail reasons, profiles, and publication metadata. Command-semantic V2 infers conventional fields and embeds capture policy. The TeX82 catalogue uses an implicit typed default disposition plus explicit overrides. Fixturegen alone mutates and publishes; test-support validates and stages; `corpus-manifest` remains the external-corpus leaf.

**Combines.** Luna rank 15; Codex ranks 3 and 5, plus the contract portion of rank 18.

**Counted reduction.** Approximately 500-900 scheduled authored LOC after overlap, plus 13,600-14,200 repetitive declarative/generated lines. All meaningful expected values remain explicit and are not counted as deletion.

**Proof.** Preserve Git authority, normalized paths, exact case membership/order, source closure, xfail reasons, schema compatibility, missing/extra rejection, traversal protection, command capture selection, TeX82 module census and ownership, atomic publication, and local fixture-edit workflow.

## 13. Use one WASM wire schema, catalogue boundary, and session driver

**Outcome.** Explicit host-neutral DTOs own options, requests, responses, attempts, outputs, diagnostics, metrics, and stable error codes. TypeScript derives from those DTOs with explicit `Uint8Array` handling. One JS `SessionDriver` and `WorkerRpcClient` serve one-shot, editor, worker, and asynchronous resource sessions. `umber-distribution` validates raw catalogues and returns authenticated typed transport plans; JS performs fetch/cache/abort.

**Combines.** Luna ranks 17 and 18; Codex ranks 4 and 14.

**Counted reduction.** Approximately 2,600-3,600 scheduled Rust/JavaScript/test LOC across wire and catalogue publication after excluding acquisition in program 3 and HTML in program 7. Legacy public distribution API deletion adds 300-400 conditional LOC.

**Proof.** Preserve `Uint8Array`, safe integers, omitted fields, unknown-field policy, error codes/messages, worker containment, transfers, cancellation, request order, catalogue duplicate rejection, root/shard bytes and authentication, platform selection, HTML allowlists, offline use, and package API behavior.

## 14. Generate bibliography compatibility cases, then collapse production stages

**Outcome.** One compatibility-case manifest and immutable runner preserve separately named upstream assertions, inputs, outputs, order, and xfails. With that proof layer active, Biber uses one engine-owned editable draft and one freeze; classic retains its explicit-frame VM but removes duplicate lexer/compiler/callable/READ/report authorities; input and output stages lose intermediate models that are converted and discarded.

**Combines.** Luna rank 10; Codex rank 2 and secondary findings from `bib-input`, `bib-output`, and `bib-unicode`.

**Counted reduction.** Approximately 6,000-9,000 scheduled test Rust LOC plus 900-1,700 scheduled production LOC. Public legacy `Bib*` and Unicode APIs are excluded unless separately deprecated.

**Proof.** Preserve per-assertion selection and failure, exact Unicode and bytes, upstream order, xfails, field/source order, duplicate and inheritance policy, case-insensitive lookup, configuration precedence, XML/XInclude limits and errors, classic Web2C bounds and allocation traces, diagnostic order, BLG/BBL bytes, and generated filenames.

**Dependencies.** Generate the compatibility suite first. Never replace it with one mega-test or self-generated expected values.

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
