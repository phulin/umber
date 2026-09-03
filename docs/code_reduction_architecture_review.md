# Code-Reduction Architecture Review

## Scope and accounting

This review synthesizes 34 crate, tool, and benchmark reports for the Umber workspace. It retains only large, coherent initiatives that remove a duplicate authority, representation, execution path, or test expansion while preserving the behavior that the workspace currently promises.

The estimates are planning ranges, not commitments. They are separated into three categories:

- **Authored LOC**: Rust, JavaScript, tests, and shell/tooling maintained as executable source.
- **Declarative/generated lines**: repeated JSON, manifests, catalogues, or generated records whose deletion reduces repository size and authority count but is not a Rust implementation reduction.
- **Binary bytes**: assets such as font fixtures. These are not included in either LOC total.

Moving code between crates, moving expected bytes into a manifest, deleting a lockfile that reappears in the root lock, or hiding logic in generated Rust is not counted as a reduction. Public APIs, CLIs, benchmarks, ignored tests, and manual compatibility tiers are treated as functionality unless an explicit compatibility or retirement decision says otherwise.

## Aggregate result

### High-confidence, compatibility-preserving baseline

The baseline is **19,200-28,000 authored LOC**, plus **13,600-14,200 repetitive declarative/generated lines**, for **32,800-42,200 checked-in lines overall**.

The authored baseline is composed of:

- declarative Biber compatibility tests: 6,000-9,000;
- browser wire DTO/driver/RPC consolidation: 1,800-2,500;
- zero-sized `CommandRuntime` deletion: 450-600;
- typesetting production representations and topology: 900-1,250;
- effect journal and revision patch: 1,400-2,100;
- artifact codec/geometry consolidation: 1,450-1,900;
- command-semantic loader/capture implementation: 500-850;
- primitive catalogue: 900-1,400;
- oracle evidence/views/comparison: 900-1,350;
- executor assignment/operation pipeline: 900-1,250;
- executable node schema and production-decoder test migration: 1,000-1,500;
- compatibility-preserving distribution/publisher core: 800-1,100;
- internal canonical font representation: 650-850;
- PDF support infrastructure: 700-1,000;
- verified acquisition/store/test fixture: 550-800;
- single HTML producer model: 300-500.

The declarative/generated baseline is composed of:

- implicit TeX82 default dispositions: about 11,000 lines;
- command-semantic V2 fixture metadata and detached capture catalogue: about 2,600-3,200 lines.

### Conditional upside

If the project approves the named coverage, compatibility, CLI, and roadmap gates, the portfolio adds **22,400-27,800 authored LOC** of upside. The full conditional range is therefore **41,600-55,800 authored LOC**, plus the same **13,600-14,200 declarative/generated lines**, or **55,200-70,000 checked-in lines overall**.

The conditional increment is composed of:

- `tex-exec` dormant-test coverage recovery and compaction: 15,000-17,400;
- public `tex-state` façade removal: 2,100-2,700;
- command test compaction after a case-level assertion ledger: 1,250-2,400;
- typesetting test compaction after an assertion ledger: 700-950;
- legacy/public distribution API retirement: 300-400;
- unused exported Umber resource-plane retirement: 950-1,100;
- raw font-model API retirement: 150-250;
- unused Rust HTML receiver API retirement: 550-700;
- VFS single-stage/public-API roadmap decision: 750-1,000;
- `refexec`/parity CLI retirement or compatibility-command migration: 650-850.

No binary-byte saving is included. A separate font-fixture subsetting task could remove about 1.2-1.4 MiB, but it is an asset/provisioning project rather than code reduction.

## Resolved ownership choices

Four report conflicts have one selected direction in this portfolio:

1. **HTML receiver:** keep JavaScript as the actual public trust, DOM, and resource-lifetime boundary. Build one Rust HTML producer model, wire `PatchPlan` directly, retain JS pre-mutation validation, and retire the unused Rust receiver only behind a public-API gate. Do not add a main-realm WASM receiver.
2. **Composite resource resolver:** delete the two unused Rust resource control planes and keep the active JavaScript provider-composition behavior. Do not activate an unused Rust resolver merely to delete the active JS path.
3. **Observation schema:** keep engine observations and the immutable `tex-oracle` wire schema distinct. Add schema-owned exhaustive views/visitors and typed producer enums; do not move the physical wire model below `tex-oracle`.
4. **Reference runner:** fixturegen owns the minimal feature-gated reference-process kernel because it is the mutation/publication owner. Parity remains comparison/triage-only, and `test-support` remains the DVI equality owner. Preserve a compatibility command or approve CLI retirement before deleting `refexec`/live parity modes.

## Ranked initiatives

The ranking considers expected net reduction first, then architectural leverage, evidence quality, and compatibility risk. A conditional item is ranked by its upside but contributes nothing to the baseline until its gate is satisfied.

## 1. Recover and compact the unreachable `tex-exec` test island

**Status:** completed by `umber2-vgjr.15.2`; catalogue-link follow-up is
`umber2-vgjr.15.2.1`.

**Affected code.** `crates/tex-exec`, documentation, and TeX82 property shards that cite disabled tests.

**Evidence.** The original inventory is preserved in
`docs/tex_exec_dormant_test_ledger.md`. The library target now selects 445
crate-internal cases, all former `cfg(any())` sites are gone, and the routine
source audit accepts no exceptions. Compiler-backed caller mapping removed
callerless production scaffolding while retaining exact case operands.

**Result.** Every original case and helper row has an active or explicitly
retired disposition. Same-path cases were retained conservatively when exact
operand equivalence was not proved. The active source audit prevents
recurrence.

**Migration record.** The case-level ledger records test identity, semantic
assertions, fixture inputs, expected diagnostics/events, external citations,
and active replacements. The separate catalogue child owns its 35 properties
and 46 source paths.

**Estimate.** Gross dormant surface 17,500-18,200 authored LOC; replacement coverage 800-2,500; conditional net 15,000-17,400. The net must be recomputed from the completed ledger rather than assumed from this range.

**Invariants and risks.** Runtime behavior is unchanged, but latent specifications are functionality. Failure granularity, diagnostic bytes, rollback/resource suspension, alignment, insertion/page output, and normative evidence citations must survive. Moving all source under `tests/` would reactivate coverage but would not achieve the reduction.

**Dependencies/order.** Complete before executor initiatives 10 and 15 so dead helpers and tests do not distort those migrations.

## 2. Generate Biber compatibility tests from one manifest and runner

**Status:** baseline after a per-assertion equivalence ledger.

**Affected code.** `crates/bib-engine/tests/it/upstream`, `tests/it/scaffold.rs`, and shared bibliography fixture support.

**Evidence.** The upstream test tree is 27,996 lines. `uniqueness.rs` is 4,914 lines for 227 assertions and `labelalpha.rs` is 2,826 for 122. The fixture/session/override/BBL-slicing harness at `uniqueness.rs:9-159` is repeated in `labelalpha.rs:10-159`, `labelalphaname.rs:10-159`, `extratitle.rs:10-159`, `extratitleyear.rs:10-159`, and `extradate.rs:10-182`; `names.rs:9-212` and `names_x.rs:9-212` repeat another runner. The nine clearly regular cohorts total 12,799 lines. `scaffold.rs:610-689` lexically counts assertion-looking identifiers rather than binding assertions to inputs and expected values.

**Target.** One checked compatibility-case manifest owns pinned upstream identity, module, order, test name, xfail reason, effective control/input data, session options, output request, and expected values or fixture slices. One immutable `UpstreamFixture` runner and a closed macro/schema vocabulary expand each row to its own named `#[test]`.

**Migration.** Introduce the runner without deleting tests. Convert the most regular uniqueness/label/name/date cohorts. For every row, prove old/new name, module, order, ignored reason, inputs, output bytes, and independent failure. Keep bespoke assertions handwritten and referenced once by the manifest. Replace the lexical census with typed completeness validation.

**Estimate.** Gross expanded Rust/harness removal 9,500-14,000; replacement declarations, runner, macros, and validation 2,000-3,500; net **6,000-9,000 test Rust LOC**. Expected data moved into a manifest is not counted as deleted.

**Invariants and risks.** Preserve individual test selection, exact Unicode/bytes, module grouping, upstream order, xfail reasons, and pure cache identity. A giant table-driven test, opaque generated source, or self-generated expected values is not acceptable.

**Dependencies/order.** This is the bibliography migration safety net. Do not delete `bib-output` full-document goldens until currently ignored engine comparisons are active and byte-exact.

## 3. Make TeX82's default disposition implicit

**Status:** baseline declarative/generated reduction; not Rust LOC.

**Affected code.** `tests/tex82-properties/dispositions.json`, `tests/tex82_catalogue.rs`, `scripts/generate-tex82-property-inventory.py`, and catalogue documentation.

**Evidence.** `dispositions.json` is exactly 11,047 lines and repeats the same `deferred_review`, empty property IDs, null owner, gap bead, and rationale for 1,380 modules. `tests/tex82_catalogue.rs:69-89` loads all records only to seed defaults before shard overrides.

**Target.** After validating `modules.json`, the catalogue validator initializes `1..=1380` to one typed implicit deferred disposition and applies shard overrides. The explicit generated file and generation path disappear.

**Migration.** Assert that the old explicit and new implicit resolved maps are identical, including order and census. Convert staged negative tests to manipulate the resolved/default set. Remove the generator output and update documentation.

**Estimate.** About **11,000 declarative/generated lines** removed; authored Rust/Python/docs change is approximately neutral to tens of lines.

**Invariants and risks.** Preserve the pinned source identity, exact module range/order, gap bead/rationale, single shard ownership, property citations, and visible deferred count. The executable complete census replaces visual repetition as the proof of coverage.

**Dependencies/order.** Independent and suitable for the first implementation wave.

## 4. Establish one browser wire DTO layer and one JS session/RPC core

**Status:** baseline, with test deletion gated by a committed boundary-coverage matrix.

**Affected code.** `crates/umber-wasm/src/options.rs`, `src/result*`, `src/lib.rs` TypeScript declarations, `js/compile.js`, `js/worker-controller.js`, `js/worker-entry.js`, declarations, and their tests; a host-neutral protocol module in `umber` or a small neutral crate.

**Evidence.** Manual adapters occupy 646 lines in `options.rs`, 540 in `result.rs`, 125 in `result/resources.rs`, and 251 in `result/metrics.rs`. A separate 249-line TypeScript schema and declaration mirrors repeat field names and enum spellings. `compile.js`, `worker-controller.js`, and `worker-entry.js` total 1,345 lines and duplicate ordinary/editor driving, worker initialization, RPC listeners, cancellation, timeout, transfer, and error mapping.

**Target.** Explicit wire DTOs own options, requests/responses, attempts, outputs, diagnostics, metrics, observations, and stable error codes. TypeScript is generated from those DTOs with an explicit `Uint8Array` serializer. One authored-JS `SessionDriver` and `WorkerRpcClient` serve one-shot and retained sessions. JS continues to own asynchronous networking, workers, abort, timeout, and package ergonomics.

**Migration.** Freeze current JS shapes as golden fixtures, including omitted properties, safe-integer boundaries, error codes/messages, and every byte-bearing variant. Differential-test manual and DTO paths. Switch options/resources, then result families. Extract the driver and RPC client without changing messages. Commit a matrix mapping every removed test assertion to wire, worker, browser, or native-engine ownership.

**Estimate.** Gross replacement/deletion 3,200-4,100 authored Rust/JS/test lines; new DTOs, byte adapters, driver, RPC client, compatibility facades, and boundary tests 1,200-1,700; net **1,800-2,500 authored LOC**.

**Invariants and risks.** Preserve `Uint8Array`, no base64/JSON byte copies, integer validation, optional-property omission, current unknown-field tolerance, request/response order, transfer ownership, progress correlation, cancellation semantics, and one-shot worker containment. Do not put serde derives on allocation-rich engine internals.

**Dependencies/order.** Excludes catalog semantics (initiative 14), HTML render updates (initiative 19), and Rust provider composition (initiative 16).

## 5. Introduce command-semantic case contract V2 with embedded capture policy

**Status:** baseline; savings reported separately as authored versus fixture metadata.

**Affected code.** `tools/tex-command-stream/src/semantic.rs`, `src/bin/command-semantic-channels.rs`, 203 command-semantic manifests, the detached capture list, census tests, and later the reference-capture shell.

**Evidence.** The 203 manifests total 10,456 lines. Each has a singleton `cases` array even though `semantic.rs:942-1045` requires one case, matching directory ID/domain and conventional source. Normal dispositions dominate: terminal file in 199 cases, log file in 197, DVI present in 67 and empty in 136, effects empty in all 203, 201 clean statuses, and 202 pass expectations. A 173-line capture allowlist and the 467-line census at `tests/it/command_semantic.rs:303-770` duplicate typed `SessionProfile` selection, with one explicit exception. The handwritten schema has drifted from committed variants.

**Target.** `CaseManifestV2` infers domain, ID, and source from the closed directory; defaults pass/clean and ordinary file-or-empty channels; stores only exceptions, projections, provenance, and a typed capture-policy override. Structural schema is generated from the Rust manifest type. Validated case selection replaces the detached capture catalogue. A later typed capture-and-plan command may replace shell orchestration only after process equivalence is proven.

**Migration.** Add dual V1/V2 reading. Convert all 203 cases atomically. Compare loaded model, route, profile, capture set, projections, expected channels, status, and closed inventory old versus new. Retain all 1,233 meaningful expected projection strings. Remove the detached list and census only after exact selected-set equality.

**Estimate.** **500-850 authored Rust/shell LOC** plus **2,600-3,200 repetitive declarative fixture lines**. Replacing the live capture shell is excluded from the baseline estimate.

**Invariants and risks.** Missing or xfail files remain fatal/explicit; effects remain required empty; oracle eligibility never changes engine execution; every selected case must capture; schema and publication remain fail-closed and atomic.

**Dependencies/order.** Oracle event views in initiative 13 are separate and not counted here. Use shared fixture staging only if initiative 22's inventory design is later approved.

## 6. Remove obsolete `tex-state` façades

**Status:** implemented under the workspace-internal API policy; see Beads issue `umber2-vgjr.8.4`.

**Affected code.** `crates/tex-state/src/universe.rs`, `src/stores.rs`, exports/UI tests, and small command/executor call sites.

**Evidence.** `ExpansionState` begins at `universe.rs:105`, `ExpansionContext` at 435, and two largely forwarding implementations span `universe.rs:7799-8882`. Only `Universe` and `ExpansionContext` implement the trait, while canonical processing owns concrete `CommandContext`. Private `Stores` begins at `stores.rs:185`; 237 method names overlap with `Universe`, with large mirrored bands for code tables, immutable stores, fonts, environment, groups, and registers.

**Target.** Delete `ExpansionState`, `ExpansionContext`, and `MeaningCacheGuard`. Retain `CommandContext` and `InputOpenContext` as the real restricted capabilities. Replace method-rich private `Stores` with a field-only `StoreData`; domain-split inherent `Universe` impls become the sole state-facing API.

**Migration.** Concretize token-display/group/input reads and pivot capability tests. Migrate read-only content APIs, then typed mutations, then snapshots/groups. Retain private helpers for borrow splitting, format reconstruction, and hashing. If external Rust compatibility is promised, deprecate first or retain a thin adapter for one release.

**Estimate.** Gross 2,600-3,200; replacement/module plumbing 350-500; conditional net **2,100-2,700 authored LOC**.

**Invariants and risks.** Do not widen `CommandContext` or expose `StoreData` through `Deref`. Preserve dependency observation, owner nonces, survivor accounting, handle liveness, group-invalidated snapshots, transaction borrows, and token rendering.

**Dependencies/order.** Precedes initiatives 9, 11, 12, and 15.

**Outcome.** The workspace audit found no production generic consumer or
third-party implementation of the exported expansion trait, and no downstream
access to the private store aggregate. `ExpansionState`, `ExpansionContext`,
`MeaningCacheGuard`, both forwarding implementations, their dead measurement
plane, and obsolete capability fixtures were deleted. Token rendering now
accepts `Universe` directly; `CommandContext` and `InputOpenContext` remain the
restricted capabilities. The private store aggregate remains implementation
data, never a compatibility façade or public API.

## 7. Delete `CommandRuntime`, then compact scanner/delivery tests through a shallow rig

**Status:** runtime deletion is baseline; test compaction is conditional on a case-level assertion ledger.

**Affected code.** `tex-command`, construction sites in `tex-exec`, `tex-incr`, and Umber, and command scanner/delivery tests.

**Evidence.** `CommandRuntime` is zero-sized at `tex-command/src/state.rs:1685-1694`, is never read, and is threaded through every processor. There are 364 `CommandRuntime::default()` declarations in `tex-command` and 411 occurrences across `crates/`. The scanner/delivery suite repeats 441 universe constructions, 343 command-state setups, ten recorder definitions, and six processor helpers. `scalar/tests.rs`, `scan_toks/tests.rs`, and `structured/tests.rs` total 13,376 lines; systematic matrices overlap earlier narrative cases.

**Target.** Remove the empty type and constructor/session plumbing. Expand `test_harness.rs` into a small procedural `ProcessorScenario`/`ScannerRig` that owns fresh state, universe, host resources, source/token builders, recorder, and diagnostic queries. Compact only cases whose semantic assertions are fully mapped into retained matrices or bespoke tests.

**Migration.** Delete runtime plumbing mechanically and prove no exclusive-borrow behavior depended on it. Migrate setup with assertions unchanged. Build a ledger covering values, token/event order, recovery text, scanner lifecycle, rollback, source context, and identity-sensitive state. Remove duplicates only after ledger closure.

**Estimate.** Baseline runtime net **450-600 authored LOC**. Conditional test net **1,250-2,400**, for **1,700-3,000** total.

**Invariants and risks.** Every mutable-bank, glue-identity, or magnification case receives a fresh universe. Explicit recategorization and exact diagnostics remain visible. A giant test table, snapshot, or digest-only replacement is forbidden.

**Dependencies/order.** Runtime deletion should land before test-rig design and before broader command-state changes.

## 8. Replace the math arena and repeated paragraph topology with native authorities

**Status:** production reduction is baseline; test reduction requires an assertion ledger.

**Affected code.** `tex-typeset`, `tex-exec/src/math/lower.rs`, native node transaction support in `tex-state`, and typesetting tests.

**Evidence.** `math/model.rs` is 627 lines and defines a second node/box/list arena. `math/convert.rs` is 1,120 lines and copies native source lists into it; `tex-exec/src/math/lower.rs` is 449 lines and immediately translates it back. Break discovery, trace display, post-line-break, and expansion validation independently reinterpret paragraph topology. Packing and line breaking repeat horizontal metrics; vertical packing/breaking repeat contribution classification. Main math/linebreak/packing tests total 6,385 lines with overlapping rule groups.

**Target.** A detached native-node transaction uses canonical node vocabulary plus only narrow draft data for selected OpenType glyphs and direct glue. It commits atomically once at the executor boundary. `ParagraphTape` owns `NodeSequence`, analyzed break sites, prefix metrics, trace ranges, and materialization actions. One metrics IR/cursor supplies packing, line breaking, vertical contribution, and math measurement.

**Migration.** Build the detached transaction and project it against current `MathLayout`. Preserve source-box geometry and glyph IDs. Replace linebreak's parallel topology fields with `NodeSequence`; extract break sites; pair semantic/physical materialization cursors; consolidate metrics last. Inventory every test assertion before deleting overlapping rule cases.

**Estimate.** Baseline production net **900-1,250 authored LOC**. Conditional test net **700-950**, for **1,600-2,200** total.

**Invariants and risks.** Preserve 20,000-depth stack safety, shared-sublist occurrence-ordered observations, selected-glyph metrics, source-box authoritative geometry, discretionary physical topology, wide-prefix overflow behavior, trace-route distinctions, and compact character-run acceleration.

**Dependencies/order.** Benefits from initiatives 6 and 12. Detached transaction first, tape second, metrics third, test compaction last.

## 9. Create one effect journal and executor-closed revision patch

**Status:** baseline.

**Affected code.** `tex-state::World`, `tex-exec` page-output commit, `tex-incr`, and `umber::virtual_compile` caller state.

**Evidence.** `World` and `WorldSnapshot` repeat aligned effect roots, with about 213 direct sidecar references. `tex-incr` carries those sidecars through `PendingRevision`, `AdvanceSetup`, `Session`, and `RevisionRun`. Publication reconstruction spans `tex-incr/src/lib.rs:3739-5161`, including `assemble_effect_ledger` and `assemble_artifact_ledger`, after executor commit already knows publication dispositions and winners.

**Target.** `EffectJournal` owns retained/live segments and atomic metadata records. Boundaries exchange opaque `EffectBundle`s. Page-output commit closes episode/winner/disposition knowledge into a validated `RevisionOutputPatch` with effects, artifacts, and DVI plans. Incremental code retains prefix, applies patch, appends convergence-validated suffix, and uses one revision transaction lifecycle.

**Migration.** Introduce the journal behind existing getters. Close executor receipts at commit. Differentially compare current assembler output with patch application across recursive output, paragraph replay, terminals, DVI-disabled sessions, and finalization. Collapse revision representations only after patch parity.

**Estimate.** Gross overlapping sidecar/assembler/plumbing 3,000-4,200; journal/patch/revision replacement 1,200-1,800; net **1,400-2,100 production LOC**.

**Invariants and risks.** Preserve COW roots, absolute effect bases, episode/publication/semantic/placement order, recursive shipout, terminal phases, OpenOut positions, accepted prefix/suffix boundaries, provenance adoption, suspension safety, and two-phase prepare/accept.

**Dependencies/order.** Requires initiative 6. Moving the existing assembler unchanged does not qualify.

## 10. Establish one artifact node codec and one geometry walker

**Status:** baseline with a mandatory performance stop gate.

**Affected code.** `tex-out/src/binary.rs`, DVI/positioned traversal, coordinate oracle, and `tex-exec::shipout`.

**Evidence.** `binary.rs` expresses the node grammar in streaming writer (`537-954`), owned writer (`1881-2125`), streaming reader (`1029-1367`), and skip/owned decoder (`2984-3607`), while `model.rs:796-887` validates it again. Streaming and owned character/margin-kern checks already differ. DVI traversal/leaders total 1,122 lines, positioned traversal 1,006, and the coordinate oracle 279. Fresh shipout feeds both artifact and DVI builders and owns a 336-line DVI-only materializer, while memo hits already compile DVI from V10 bytes.

**Target.** One iterative validated node-event cursor/emitter is the artifact codec authority. Owned decode, zero-copy DVI planning, preliminary scans, validation, and direct production adapt to it. One explicit-frame geometry walker owns box coordinates, glue, leaders, snapping, ordinals, and sibling lookahead; DVI and positioned sinks retain backend policy. Fresh and memo-hit DVI derive from canonical artifact bytes.

**Migration.** Build cursor and differential accepted/error tests. Convert readers and owned collection, then writers and validation. Add the common geometry walker and compare DVI bytes/positioned events. Switch fresh shipout last. Remove the coordinate oracle only after parity and benchmark gates.

**Estimate.** Gross 4,400-4,900 overlapping production lines; replacement cursor/walker/sinks 2,700-3,100; net **1,450-1,900 production LOC**.

**Invariants and risks.** Preserve artifact v23 and legacy bytes, parse-error precedence, all resource/depth/collection limits, nonrecursive replay, Unicode/classic policy, ligature source units, DVI movement/font/leader bytes, positioned text/effects, and throughput/RSS. The extra fresh-page byte pass is a stop gate.

**Dependencies/order.** Node cursor first, geometry walker second, executor dual-emission deletion last.

## 11. Define one authoritative primitive/profile/WEB descriptor catalogue

**Status:** baseline.

**Affected code.** `tex-state/src/meaning.rs`, `tex-command` registry/observation/prefix policy, `tex-exec` primitive installation/admissibility, `umber::pdftex`, tests, and primitive documentation.

**Evidence.** Stable operands/variants live at `tex-state/src/meaning.rs:211-1313`; expandable names/profiles at `tex-command/src/primitives/registry.rs:14-500`; WEB identities at `observation/primitive_identity.rs:127-654`; unexpandable/parameter installation in the 569-line `tex-exec/src/assignments/primitives.rs`; pdfTeX inventories/defaults at `umber/src/pdftex.rs:10-439`. These are repeated views of one registry, but much of the surrounding files is real behavior and must remain.

**Target.** One declarative Rust-owned catalogue adjacent to stable meaning operands, or a small neutral descriptor owner if command spelling cannot live in `tex-state`. Rows own stable operand, canonical spelling/aliases, profile membership, expandable class, WEB formula reference, prefix/admissibility flags, installation policy, parameter cell/default, and documentation family. Execution bodies remain explicit Rust.

**Migration.** Generate parallel tables and equality tests. Convert enum operand maps and profile descriptors, then registration/installation, then observation/prefix/admissibility, then pdfTeX defaults/docs. Keep exceptions such as frozen names, `nullfont`, `endwrite`, control space, and aliases explicit.

**Estimate.** Gross duplicated metadata/tests/docs 2,000-2,800; catalogue/formulas/exceptions 900-1,400; net **900-1,400 authored LOC**. This is the sole primitive-catalogue estimate across all four reports.

**Invariants and risks.** Numeric operands, profile layouts, install order, register-after-format behavior, aliases, private/frozen meanings, and parameter slots are compatibility data.

**Dependencies/order.** Land after initiative 6 and before initiative 15.

## 12. Make the executable node schema authoritative

**Status:** baseline after production-decoder test migration.

**Affected code.** `tex-state` node/view/storage/copy/format/hash paths and tests.

**Evidence.** `NODE_SCHEMA` at `node.rs:41-88` is descriptive only. The same roughly 24 variants are repeated in owned equality (`node.rs:179-350`), `NodeRef` conversion/equality (`node_arena/view.rs:124-269`), compact encoding/decoding, child copy/patch, handle validation, semantic identity, and format capture/remap. Test-only `StoreFormat::restore` at `stores/format.rs:1453-1654` reconstructs an alternate format path, while `state_hash.rs:904-1375` contains a roughly 470-line alternate recursive node hasher.

**Target.** A typed exhaustive `NodeRef`-centered visitor/schema declares tags, semantic and nonsemantic fields, handle kinds, ordered child lists, and portable remapping. Mechanical walks delegate to it; compact SoA storage and the handle-free validated `FormatNode` boundary remain specialized. Production frozen decode is the only restoration authority.

**Migration.** Add parallel schema-generated operations and all-variant equivalence tests. Switch equality, child enumeration, validation, identity, and format mapping one by one. Port malformed-reference cases to frozen codecs. Delete alternate restoration and recursive hash only after coverage moves.

**Estimate.** Gross 1,900-2,500; visitor/schema/migrated tests 800-1,000; net **1,000-1,500 production/test LOC**.

**Invariants and risks.** Preserve semantic tags/version, origin exclusions, child order, allocation-free borrowed views, compact sidecars, survivor patching, schema-11 tags, malformed reference rejection, and nonrecursive behavior.

**Dependencies/order.** Requires initiative 6 and can support initiative 8's detached node transaction.

## 13. Put oracle event views, finalization, and comparison in one typed pipeline

**Status:** baseline; engine observations and oracle wire types remain distinct.

**Affected code.** `tex-oracle`, `tex-observe`, `tex-command-stream`, `parity-harness`, and Umber evidence callers.

**Evidence.** `tex-oracle/src/normalize.rs:48-151` and `fixture.rs:370-458` independently walk the event/value/token graph. `tex-command-stream/src/group.rs:228-397` reconstructs it again to erase positions, while comparator alignment/anchor views repeat more event matches. `LiveSessionTranslator::finish` separately defines stable TRIP membership already validated in `tex-oracle::Tex82ObserverProfile`. Parity's observer fanout clones observations into multiple translators, and `trip_triage.rs` parses streams separately for divergence and accounting.

**Target.** `tex-oracle` owns exhaustive borrowed/mutable views for normalization, locations, location erasure, class, alignment/anchor identities, and concise rendering, plus typed profile projection. `tex-observe` retains the detached enrichment boundary but finalizes once into semantic, geometry, and stable views. `tex-command-stream` owns named strict/ordinary host-side comparison policies and returns one parsed comparison/accounting result. Parity consumes it once.

**Migration.** Add schema traversal and exhaustive all-carrier tests. Replace normalization/location/group walks byte-for-byte. Move stable profile projection to oracle. Return one typed evidence bundle and compare old/new JSONL. Centralize strict TRIP projection last, preserving exact ordered mismatch behavior.

**Estimate.** Gross repeated views/finalizers/comparators/tests 1,800-2,400; shared traversal/profile/result code 800-1,050; net **900-1,350 Rust/test LOC**.

**Invariants and risks.** Preserve schema-v1/v2/v3 JSON, control-sequence atom normalization, independent sequence spaces, source/geometry locations, strict TRIP index order, macro/group proof, report precedence, hashes over original bytes, and bounded million-event behavior.

**Dependencies/order.** Schema traversal, then profile projection, then typed capture, then comparison. Do not merge the physical observation and oracle wire models.

## 14. Make `umber-distribution` the sole catalogue authority and use one prepared publisher

**Status:** compatibility-preserving core is baseline; public legacy API removal is conditional.

**Affected code.** `umber-distribution`, `texlive-wasm-publish`, `umber-wasm` manifest resolver, and catalogue tests.

**Evidence.** `umber-distribution` carries live sharded roots/shards alongside old monolithic `Manifest`/`ManifestFont`, pretty writer, and a dead monolithic planner. The publisher defines shadow format metadata at `src/lib.rs:108-130`, repeats validation at `:499-611`, and maintains a 228-line programmatic MVP catalogue duplicating committed JSON. Full and HTML publication repeat scanning, object staging, sharding, pruning, inventory, and verification. `umber-wasm/js/manifest-schema.js` is 487 lines recreating file/font keys, strict JSON, canonicalization, and partition semantics even though Rust already validates catalogues.

**Target.** The shared I/O-free crate owns strict sharded root/shard/format/catalogue contracts and returns complete typed browser transport plans from raw manifest text. Legacy monolithic schema 1 is quarantined in publisher migration code. The publisher deletes shadow metadata and the executable MVP catalogue and feeds one prepared object set into full/HTML policy. JS retains fetch/cache/abort and object download policy, not catalogue semantics.

**Migration.** Add the thin named-format envelope and batch browser adapter. Differential-test root/shard bytes and old/new plans. Move publisher format producers to canonical metadata. Delete the MVP generator after shared/native audit. Merge publication pipelines. Retain a strict legacy reader or deprecate public monolithic APIs before removal.

**Estimate.** Baseline net **800-1,100 authored Rust/JS/test LOC**. Conditional legacy/public API retirement adds **300-400**, for **1,100-1,500** total.

**Invariants and risks.** Preserve root/shard bytes, duplicate-key rejection, canonical order, shard authentication/partitioning, format closures, required-before-hint order, HTML allowlists/inventory, complete-root authentication, filesystem read-after-write verification, and JS ownership of transport.

**Dependencies/order.** Browser DTOs in initiative 4 precede the typed transport plan. Do not add serde to the dependency-free shared crate.

## 15. Unify executor assignment commits and operation flow

**Status:** baseline.

**Affected code.** `tex-exec/src/assignments/tracing.rs`, mutation classifiers, `step_once`/observed/alignment/nested execution paths, and typed state/observation receipts.

**Evidence.** `assignments/tracing.rs` is 619 lines of family-specific old/new/global/local wrappers. `main_control.rs:12370-13216` has an approximately 680-line mutation classifier parallel to application arms at `14602-17960`; another classifier handles identical local assignments at `18089-18240`. `step_once` (`3895-4120`) and `step_with_observer_once` (`5752-6142`) repeat delivery, three resource passes, application, output, paragraph completion, cleanup, snapshot, and failure control; alignment and nested paths repeat parts again.

**Target.** An `AssignmentCommitter` performs each TeX write exactly once and returns typed mutation/trace receipts. One `execute_operation` owns snapshot, a small delivery strategy, scanning/application, resource suspension, commit/rollback/fatal behavior, and an optional observer buffer. Compatibility wrappers do not select different semantic engines.

**Migration.** Stabilize primitive descriptors first. Move simple integer/dimension registers and parameters behind the gateway; handle glue pointer identity, meanings, boxes, and arithmetic later. Compare observed/unobserved state and event streams. Merge operation paths only after all writes return authoritative outcomes.

**Estimate.** Gross duplicate classifiers/wrappers/paths 1,700-2,200; gateway/outcome/strategy code 700-950; net **900-1,250 production LOC**.

**Invariants and risks.** Preserve tracing before/after timing, identical local/global rules, glue pointer semantics, provisional meanings, overflow suppression, event/effect/afterassignment ordering, resource rollback, fatal partial commits, nested host operations, and root/alignment end policy.

**Dependencies/order.** Initiatives 6 and 11 precede this work. Keep it separate from primitive metadata generation for diagnosable migrations.

## 16. Delete the two non-driving Umber resource control planes

**Status:** conditional on exported Rust API compatibility; selected instead of activating the Rust composite resolver.

**Affected code.** `umber/src/virtual_compile/output_resources.rs`, `resource_resolver.rs`, their tests, state, and reexports.

**Evidence.** `output_resources.rs` and tests total 572 lines. The live session already constructs authoritative required/probe/prefetch vectors, registers file expectations, and returns `NeedResources`; `last_resource_plan` is built only afterward and has no production consumer. `resource_resolver.rs` and tests total 405 lines; its `TypedResourceProvider`/`CompositeResourceResolver` path has no in-tree driver beyond its own tests. Active WASM provider composition is JavaScript.

**Target.** `NeedResources`/`ResourceResponse` remains the sole engine resource protocol, with a small checked/deduplicated batch-size guard. Delete both unused Rust planes after an external API decision. Keep active JS provider composition and native distribution resolution unchanged.

**Migration.** Confirm whether exported 0.1 APIs are workspace-internal. If not, deprecate or move compatibility facades to an optional compatibility crate/release. Remove planner construction/state/reexports and retain ordering/ceiling assertions at the live protocol seam.

**Estimate.** Gross about 1,000-1,050; replacement 15-40; conditional net **950-1,100 authored Rust/test LOC**.

**Invariants and risks.** Preserve required-over-probe promotion, earliest probe frontier, prefetch filtering, request ordering, and ceilings. Do not introduce a third provider abstraction.

**Dependencies/order.** The ownership decision is independent but should precede further browser orchestration work.

## 17. Publish canonical TFM and OpenType MATH models directly

**Status:** internal portion is baseline; raw public API retirement is conditional.

**Affected code.** `tex-fonts` TFM/MATH models, `tex-exec` font loading, `tex-typeset` MATH facade consumers, and Umber VF/PDF font paths.

**Evidence.** `opentype/math.rs:10-817` defines an owned public MATH graph and custom parser after `OpenTypeFont::parse` already owns a validated `ttf_parser::Face`; tests add 228 lines. Production uses projected `OpenTypeMathMetrics`. `tfm/types.rs:11-229` retains a second full metric model; `TfmFont::font_metrics` converts it into canonical runtime metrics, while parser and `FontMetrics` validation overlap. Parameter padding repeats across parser, executor, and `LoadedFont`.

**Target.** Perform one strict eager validation walk of the dependency's lazy MATH table, then query it through the current scaled facade. TFM parsing keeps raw tables only until reference/error-precedence validation and publishes canonical `FontMetrics` directly with one loaded-font constructor.

**Migration.** Canonicalize TFM first and compare complete metrics/errors. Preserve temporary raw tags through validation. Switch executor/VF loaders. Add strict lazy MATH validation/query parity, including malformed limits and native/WOFF2 equivalence. Deprecate raw exported models before deletion if necessary.

**Estimate.** Baseline internal net **650-850 authored LOC**. Conditional raw API/test retirement adds **150-250**, for **800-1,100** total.

**Invariants and risks.** Preserve MATH stricter checks and budgets, self-cell borrow safety, device/variation policy, selected glyph IDs/metrics, TFM error precedence, lig/kern limits, absent-character tags, boundary records, parameter padding, and `font_info_words`.

**Dependencies/order.** TFM first, then MATH. Initiative 8 continues to consume only the projected facade.

## 18. Consolidate PDF support infrastructure around Hayro and `pdf-writer`

**Status:** baseline.

**Affected code.** `test-support/src/pdf_probe.rs`, `src/pdf.rs`, `src/pdf_fixture.rs`, their tests, and PDF test consumers in `tex-out`/Umber.

**Evidence.** `pdf_probe.rs` is 809 lines including tests and reproduces Hayro's object model, then `pdf.rs:15-477` walks the copied model again to create stable structure and inherited resources. Only three external test files use the generic graph. `pdf_fixture.rs` and tests total 651 lines and implement a custom valid PDF writer, xref, trailer, dictionaries, streams, verifier, and self-tests despite the already-pinned `pdf-writer` dependency.

**Target.** The stable canonical projection walks Hayro's borrowed objects directly, with only focused queries for raw streams, object dictionaries, ordered pages, and decoded operations. Valid synthetic PDFs use `pdf-writer` directly or a tiny adapter. Intentionally malformed/deep/xref inputs use a visibly separate byte-level helper.

**Migration.** Replace the valid fixture writer first while Hayro remains the independent parser. Delete writer self-tests, retaining semantic consumer checks. Classify probe consumers into canonical projection versus focused raw queries. Port object-stream/xref/cycle/budget coverage before deleting the owned DOM.

**Estimate.** Gross 1,500-1,600; focused query/adapter/malformed helper and consumer changes 500-800; net **700-1,000 authored Rust/test LOC**.

**Invariants and risks.** Preserve parser independence, page/object order, xref/object streams, deterministic cycle labels, unresolved references, inherited resources, raw versus decoded streams, operation order, budgets, caller object numbers/gaps, and truly malformed inputs.

**Dependencies/order.** Writer replacement before probe removal. A future PDF closed-case runner is separate and not counted.

## 19. Use one HTML producer model and keep JavaScript as the receiver

**Status:** implemented; producer consolidation and Rust receiver API retirement are complete.

**Affected code.** `tex-out/src/html.rs`, `src/html/incremental*`, `umber::virtual_compile`, `umber-wasm/src/result/render.rs`, and authored JS validation/DOM code.

**Evidence.** Standalone and incremental HTML independently resolve fonts and translate positioned/math events. Incremental construction calls and discards a complete standalone `write_positioned_html` result at `html/incremental.rs:367-388`, then resolves fonts again at `389-429` and maps events again at `470-698`. `tex-out` also has a roughly 659-line Rust receiver/protocol/applier, but WASM serializes only `envelope.patch`, dropping magic, capabilities, counts, and fingerprint. The browser JS mount is the actual public raw-data, DOM, focus/scroll, and resource-lifetime boundary.

**Target.** One keyed `RenderDocument`/`RenderRevision` resolves fonts, events, specials, accessibility, and math once. Standalone HTML/assets and incremental keys/digests/plans derive from it. Wire `PatchPlan` directly. Keep JS pre-mutation validation and DOM/resource transaction logic. Do not create a main-realm WASM receiver.

**Migration.** Freeze standalone HTML and patch-wire goldens. Build the render document in parallel and compare exact bytes/assets/operations. Switch both outputs. Simplify render DTO serialization. If Rust receiver APIs are public, deprecate before deleting; retain browser hostile-patch, rollback, focus/scroll, and resource-lifetime tests.

**Estimate.** Baseline producer net **300-500 authored LOC**. Conditional unused Rust receiver retirement adds **550-700**, for **850-1,200** total.

**Invariants and risks.** Preserve exact standalone bytes, event ordinals, resource names/order/identity, accessibility, specials, math glyph selection, DOM identity, focus/selection/scroll, atomic rollback, resource leases, validation limits, CSP, and large-patch performance.

**Dependencies/order.** Browser DTOs in initiative 4 precede render serialization. This choice is incompatible with a Rust/WASM receiver and intentionally rejects it.

**Outcome.** The external-use gate closed on 2026-08-06: the repository had no
tags, releases, packages, or forks; neither Rust crate was published; the npm
package was private and unpublished; and exact-symbol plus workspace searches
found only receiver tests and re-exports. Production had always projected
`PatchPlan` directly. The Rust envelope, protocol validator, abstract applier,
re-exports, and receiver-only tests were deleted without a compatibility shim.
`HtmlPatchMount` now exclusively owns hostile structured input, bounded model
simulation, detached DOM construction, resource verification and lifetime,
atomic publication, and resynchronization.

## 20. Collapse VFS to one generated transaction and private shape-safe maps

**Status:** conditional on an explicit public API and future multipass roadmap decision.

**Affected code.** `umber-vfs` transaction/storage/snapshot APIs and tests, plus direct Umber/bibliography fixtures.

**Evidence.** `BuildPlan::invalidate_accepted`, `declare_replacement`, producer collisions, multi-stage commits, and standalone `VirtualFs` are test-only. All production callers create exactly one stage per build. Yet pending layers and stage metadata affect durable storage and lookup. Generic public `LayerKind`, `FileLayer`, and `LayeredFileStorage` model a fixed schema and revalidate root/origin combinations; external raw construction is confined to tests.

**Target.** `GeneratedTransaction` owns a private write set, validated snapshot, limits, drop rollback, and atomic replacement of accepted-generated output. Durable storage is a private COW generation of three typed maps: user, resolved, and accepted-generated. Narrow constructors make wrong root/origin states unrepresentable.

**Migration.** First decide that supported TeX -> bibliography -> TeX orchestration continues to publish one orchestrator-owned set rather than use VFS multi-stage semantics. Collapse real call sites while keeping old storage. Delete pending/collision APIs/tests. Privatize maps and migrate raw fixtures. Deprecate public multi-stage/layer APIs if needed.

**Estimate.** Gross 1,350-1,550; replacement 550-650; conditional net **750-1,000 authored production/test LOC**.

**Invariants and risks.** Preserve invisible writes until accept, drop rollback, whole-set replacement, count/byte limits, stale transaction snapshots, COW generations, lookup precedence, ordering, retained-byte accounting, storage identity, immutable conflicts, and shared bytes.

**Dependencies/order.** One transaction first, private maps second. Do not merge request ledgers into path maps.

## 21. Retire standalone `refexec` and parity live execution with a compatibility gate

**Status:** conditional on CLI retirement or a behaviorally equivalent compatibility command.

**Affected code.** `tools/refexec`, `parity-harness`, `fixturegen`, DVI scripts, and `test-support` DVI setup/equality.

**Evidence.** Parity's only tracked binary invocation is comparison-only. Its large live reference/Umber runner, tracing reruns, and `run_named_external_document` have no repository caller. `refexec` is a 439-line package whose reference-process kernel is used by fixture publication, while its comparison CLI is weaker than parity's active triage. Fixturegen is already the publication owner and direct consumer of reference TeX/tftopl behavior.

**Target.** Fixturegen owns the minimal feature-gated deterministic reference-process/tftopl kernel and all reference publication. Parity is a feature-free existing-DVI/TRIP comparison and triage tool. `test-support` owns DVI equality/staging. The standalone CLI and live parity mode are removed only after a compatibility command or explicit retirement decision.

**Migration.** Add hermetic fake-executable tests for lookup, flags, environment, staging, log/DVI capture, failure, and tftopl. Move ordinary math/align publication into fixturegen using one reference run. Migrate scripts to parity comparison. Provide a thin composition command if external operators need old live behavior, then remove package/mode after a deprecation window.

**Estimate.** Gross 1,100-1,350 Rust/shell/config lines; moved/replacement kernel and commands 400-550; conditional net **650-850 authored LOC**.

**Invariants and risks.** Preserve executable override/PATH policy, pdfTeX-versus-TeX flags, INITEX/e-TeX/DVI behavior, deterministic environment, isolated staging, one-run raw publication, TFM/tftopl behavior, DVI normalization, triage artifacts, and exit status.

**Dependencies/order.** Initiative 13 should centralize evidence/comparison first. This resolves ownership in favor of fixturegen, not parity.

## 22. Use one source-aware verified acquisition engine

**Status:** baseline.

**Affected code.** `umber-fetch`, `umber::cli_resource`, format-cache accessors, and fetch tests.

**Evidence.** Standalone `FetchClient`, manifest functions, and `DistributionClient` overlap; the latter owns separate object and manifest agents. Object and manifest downloads separately implement URL validation, status/content-length checks, cancellable 64 KiB reads, bounds, SHA-256, and errors. `umber::DistributionResolver` reaches through `client.store()` and repeats cache/local/offline/remote ladders for objects, shards, and roots. Four `BlobStore` workflows repeat authority, locking, current validation, legacy migration, quarantine, and publication. The 313-line test fixture implements ureq's unversioned connector/transport despite middleware response support.

**Target.** One private `VerifiedDownloadSpec` core parameterizes exact-versus-maximum length and retry policy. `DistributionClient` owns one store, one agent, source selection, ordered batches, and source telemetry. One locked-entry state machine serves validated lookup/get-or-initialize and content-addressed object/manifest workflows. Tests use scripted middleware responses.

**Migration.** Replace the bespoke transport first. Introduce shared download policy under existing APIs. Move CLI source ladders behind `DistributionClient`. Consolidate store entry flow and migrate generic format-cache calls. Delete old test-only/public wrappers only if no compatibility obligation remains; baseline savings assume thin compatibility forwarding where necessary.

**Estimate.** Gross 1,000-1,350; shared downloader/store/middleware 450-650; net **550-800 authored Rust/test LOC**.

**Invariants and risks.** Preserve HTTPS/loopback policy, object exact versus manifest bounded length, current retry differences, ordered bounded workers, full join/all-or-nothing batch return, cancellation publication mutex, local/cache/remote telemetry, anchored no-follow store, per-key locking, quarantine, durability, legacy/offline behavior, and one constructor per key.

**Dependencies/order.** Can proceed independently after distribution wire ownership is stable.

## Recommended execution order

### Wave 0: decisions and ledgers

Decide public 0.1 API policy, CLI retirement policy, VFS single-stage roadmap, and generated-fixture review policy. Commit the case-level ledgers required by initiatives 1, 2, 7, and 8. Freeze JS, artifact, oracle, DVI, and catalogue golden contracts.

### Wave 1: isolated authorities

Land initiatives 3, 5, 7's runtime deletion, 11, 13's schema traversal portion, 14's shared catalogue contract, 18's PDF writer replacement, and 22's middleware/shared downloader. These create authoritative seams without broad execution changes.

### Wave 2: state foundations

If compatibility policy permits, land initiative 6. Then land initiatives 12 and 17's TFM half. Introduce `EffectJournal` from initiative 9 behind existing getters. Complete primitive catalogue consumers.

### Wave 3: execution and output

Land initiative 15 assignment families, then unify operation flow. Land initiative 10's node cursor, then geometry walker, then fresh artifact-to-DVI. Land initiative 8's detached math transaction, tape, and metrics. Close initiative 9's executor revision patch last.

### Wave 4: browser, publisher, and tools

Land initiative 4 DTOs/driver, initiative 14 browser transport plan and prepared publisher, initiative 19's single HTML producer, and initiative 22's completed source-aware client. Apply conditional initiatives 16, 20, and 21 only after their compatibility/roadmap gates.

### Wave 5: evidence compaction

Only after the new seams are active, complete Biber generation, command/typeset test compaction, and `tex-exec` coverage recovery. Remove old implementations immediately after equivalence rather than keeping permanent dual authorities.

## Coverage appendix

Exactly 34 reviewed targets are covered below.

| Target/report                                          | Portfolio disposition                                                                                                                                |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bib-engine` / `bib-engine.txt`                        | Initiative 2 retained. Broader dormant Biber/public result pruning excluded; mutable `EntryBuilder` fusion rejected.                                 |
| `bib-input` / `bib-input.txt`                          | No ranked item. Streaming XML is concrete but only 300-450 net for high error/XInclude risk; keep as secondary.                                      |
| `bib-model` / `bib-model.txt`                          | No qualifying independent item; its report correctly constrains mutable-entry fusion.                                                                |
| `bib-output` / `bib-output.txt`                        | No independent ranked item. Router collapse is sub-major; full-golden deletion remains blocked on ignored engine tests.                              |
| `bib-unicode` / `bib-unicode.txt`                      | Excluded: major proposal deletes public tested compatibility APIs without an approved scope break.                                                   |
| `corpus-manifest` / `corpus-manifest.txt`              | No qualifying item; already the shared dependency-free parser.                                                                                       |
| `fixturegen` / `fixturegen.txt`                        | Contributes to initiative 21. Closed-tree consolidation remains a secondary conditional design; root-workspace lockfile churn is not code reduction. |
| `parity-harness` / `parity-harness.txt`                | Contributes to initiatives 13 and 21. Live CLI deletion is compatibility-gated.                                                                      |
| `png-import-prototype` / `png-import-prototype.txt`    | Excluded: deletion retires a unique comparative benchmark and needs an explicit maintainer decision.                                                 |
| `profile-analyzer` / `profile-analyzer.txt`            | No qualifying item; deterministic specialized reports are unique functionality.                                                                      |
| `refexec` / `refexec.txt`                              | Contributes conditionally to initiative 21; CLI compatibility is explicit.                                                                           |
| `test-support` / `test-support.txt`                    | Contributes initiatives 3 and 18. Git-only fixture inventory is secondary/conditional.                                                               |
| `tex-arith` / `tex-arith.txt`                          | No qualifying item; similar code encodes different rounding, overflow, and wire contracts.                                                           |
| `tex-command` benchmark / `tex-command-benchmarks.txt` | No qualifying item; workloads measure unique active questions.                                                                                       |
| `tex-command-stream` / `tex-command-stream.txt`        | Contributes initiatives 5 and 13.                                                                                                                    |
| `tex-command` / `tex-command.txt`                      | Contributes initiatives 7 and 11. Macro-frame/portable closure is a secondary design prototype.                                                      |
| `tex-content` / `tex-content.txt`                      | No qualifying item; small stable identity boundary is already shared.                                                                                |
| `tex-exec` benchmark / `tex-exec-benchmarks.txt`       | No qualifying item; possible harness sharing is sub-threshold.                                                                                       |
| `tex-exec` / `tex-exec.txt`                            | Contributes initiatives 1, 10, 11, and 15. Dormant-test estimate is conditional on the case ledger.                                                  |
| `tex-fonts` / `tex-fonts.txt`                          | Contributes initiative 17. Binary fixture subsetting is reported outside LOC and not ranked.                                                         |
| `tex-incr` / `tex-incr.txt`                            | Contributes initiative 9. Generic trace/public API deletion excluded.                                                                                |
| `tex-observe` / `tex-observe.txt`                      | Contributes initiative 13 only through detached finalization; shared physical wire model rejected.                                                   |
| `tex-oracle` / `tex-oracle.txt`                        | Contributes initiative 13. Fixture contract v3 remains a secondary opportunity.                                                                      |
| `tex-out` / `tex-out.txt`                              | Contributes initiatives 10 and 19. Detached PDF semantic-hash deletion excluded.                                                                     |
| `tex-state` benchmark / `tex-state-benchmarks.txt`     | Excluded from ranking: retirement removes developer profiling capability without explicit approval.                                                  |
| `tex-state` / `tex-state.txt`                          | Contributes initiatives 6, 9, 11, and 12. Public compatibility gates are explicit.                                                                   |
| `tex-typeset` / `tex-typeset.txt`                      | Contributes initiative 8. Test reduction requires assertion mapping.                                                                                 |
| `texlive-wasm-publish` / `texlive-wasm-publish.txt`    | Contributes initiative 14.                                                                                                                           |
| `umber-distribution` / `umber-distribution.txt`        | Contributes initiative 14. Legacy public model deletion is conditional.                                                                              |
| `umber-fetch` / `umber-fetch.txt`                      | Contributes initiative 22.                                                                                                                           |
| `umber-interrupt` / `umber-interrupt.txt`              | No qualifying item; retain the unsafe-FFI quarantine and exact signal policy.                                                                        |
| `umber-vfs` / `umber-vfs.txt`                          | Contributes conditional initiative 20.                                                                                                               |
| `umber-wasm` / `umber-wasm.txt`                        | Contributes initiatives 4, 14, and 19. Rust receiver and Rust composite resolver activation rejected.                                                |
| `umber` / `umber.txt`                                  | Contributes initiatives 16 and 22. Multipass/format/PDF redesigns remain secondary until replacement estimates are proven.                           |

## Explicitly rejected rewrites and deletions

- Do not call benchmark or profiling-tool deletion behavior-preserving merely because CI does not invoke it. The PNG prototype and `tex-state` synthetic benchmarks require explicit retirement decisions and are not ranked.
- Do not delete `tex-exec` dormant tests without the case-level coverage/citation ledger described in initiative 1.
- Do not delete public compatibility APIs solely because workspace production has no caller. Use an internal-API declaration, deprecation release, or compatibility adapter for `tex-state`, `tex-fonts`, Umber resource planes, distribution legacy models, VFS stages, HTML receiver APIs, and tool CLIs.
- Do not activate the unused Rust composite resolver to replace active JS composition while also counting deletion of that Rust surface.
- Do not move the oracle wire model below `tex-oracle`; shared typed views remove repeated walks without weakening the immutable schema boundary.
- Do not implement a Rust/WASM HTML receiver and also delete the JS receiver. The selected target retains JS as the real trust/DOM boundary.
- Do not move the reference kernel into parity. The selected target keeps parity comparison-only and fixturegen as the publication/process owner.
- Do not add primitive-catalogue estimates from state, command, executor, and pdfTeX reports; initiative 11 counts the project once.
- Do not add effect-journal and incremental-ledger estimates; initiative 9 counts positional sidecars and publication plumbing once.
- Do not add `tex-exec` fresh-DVI savings to the full `tex-out` walker estimate; initiative 10 includes producer cleanup.
- Do not add event-view, finalization, parity-observer, and comparison estimates; initiative 13 is the single total.
- Do not generalize `bib-model::EntryBuilder` into the engine's mutable draft for LOC. Its duplicate policy and freeze-only role are different, and the independent report finds only 35-60 net lines.
- Do not delete `bib-output` reconstructed goldens while their end-to-end replacements remain ignored.
- Do not count expected fixture bytes moved into manifests, generated Rust, root/private lockfile churn, or font binary subsetting as authored-code savings.
- Do not replace bounded exact codecs/parsers with serde/bincode, parser combinators, generic serializers, recursive trees, or JSON rewriting when wire bytes, rejection order, memory bounds, or provenance are observable.
- Do not delete optional caches, incremental replay/convergence, secure format workers, browser workers, DVI plans, or HTML DOM identity semantics merely for source reduction.

## Decision rule

The portfolio should be executed only when each initiative leaves one clearly named authority and deletes the predecessor in the same migration. Temporary differential implementations are useful; permanent parallel implementations are the failure mode this review is intended to remove.
