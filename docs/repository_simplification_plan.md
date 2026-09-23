# Repository simplification and testing plan

Status: staged migration. The testing documentation and gate contracts below
describe the current repository; the production-owner and bibliography changes
remain proposals until their code and evidence land.
Review baseline: `11c7bf7cd8ec78b36337c3ee3a97e38d0dc099e9`.

The objective is to preserve Umber's current behavior while making both the
implementation and its evidence easier to understand. The first priority is
making the tests safe to refactor against; the second is reducing the number
of responsibilities and independently maintained representations in the core.
This document preserves the review rationale and proposed sequence. For the
current testing entry point, use [Testing Infrastructure](testing_infrastructure.md)
and [Testing Policy](testing_policy.md).

The testing pass established the seven-class front door, executable
script-suite inventory, and aggregate gate verdicts. The executor test split
and source-shape pilot are implemented; the bounded discretionary extraction
and the resource and root-source method grouping are recorded in
[Main-Control Responsibility Boundaries](main_control_responsibilities.md).
Browser/package coverage and artifact cleanup have separate implementation
owners and must be judged against their final merged gates. Bibliography
selection and status have not been changed by this pass; its migration is
deferred. The later storage, scanner, output, and bibliography sections below
remain design proposals. The session section records the implemented admission
and publication seams while retaining its broader ownership guidance.

## Assessment

The repository has substantial useful test infrastructure. Its difficulty is
not simply too few tests or too many crates. Behavioral evidence, reference
compatibility, implementation-shape checks, historical migration records, and
expensive diagnostics are interleaved. Important modules also combine enough
responsibilities that a local change requires understanding several subsystems.

The review inspected Cargo metadata, tracked source inventories, architecture
and testing documents, gate scripts, and representative implementations and
tests. Six Luna agents at xhigh effort covered testing, engine architecture,
gates/corpora, sessions/resources, outputs/fonts, and browser portability,
with follow-up bibliography, performance, and skeptical synthesis reviews.
The primary reviewer checked the central findings against source and
consolidated the recommendations. Counts below describe source at the baseline;
they are not measured
execution coverage or a fresh performance baseline.

| Observation                             | Baseline                                                          |
| --------------------------------------- | ----------------------------------------------------------------- |
| Root workspace packages                 | 32; 31 default members; WASM deliberately separate                |
| Tracked Rust                            | 944 files, 562,660 lines, including tools and benchmarks          |
| Rust in test paths                      | 177,022 lines; excludes inline tests and macro-generated cases    |
| Non-test-path Rust files over 600 lines | 173; some include inline tests                                    |
| Shared corpus                           | 18 areas, 1,521 tracked files                                     |
| TeX82 property catalogue                | 156 properties in eight shards: 133 marked covered, 23 marked gap |
| Documentation                           | 86 tracked files under `docs/`                                    |

Catalogue coverage is bookkeeping about specific claims, not a percentage of
TeX compatibility. Likewise, counts of `#[test]` do not account for generated
tests, ignored tests, feature selection, or multiple cases inside one test.

### What is already worth keeping

- The canonical command machine and exact TeX arithmetic; do not introduce
  another execution engine to make the current one appear smaller.
- The separation between stateful execution and state-free typesetting.
- Detached artifacts downstream of the commit boundary, with output drivers
  independent of mutable engine state.
- Cold execution as the correctness reference for incremental execution.
- Pinned external reference fixtures, explicit normalization, closed case
  directories, and the single fixture-regeneration entry point.
- Default-member coverage checks, separate shipping/test feature lint passes,
  and explicit optional checks that can report missing prerequisites.
- Small crates with real authority or portability boundaries. Crate count
  alone is not evidence that combining them will simplify the system.

These boundaries are visible in [architecture.md](architecture.md),
[tex-typeset](../crates/tex-typeset/src/lib.rs),
[tex-out](../crates/tex-out/src/lib.rs), and
[workspace selection](../crates/test-support/tests/workspace_selection.rs).

### Findings that should drive the work

**The tests sometimes preserved source spelling instead of a contract.**
At the review baseline, [executor integration tests](../crates/tex-exec/tests/it.rs)
inspected method bodies, field names, constructor counts, and source slices.
The executor dispositions below replace the remaining four such guards with
behavioral or compiler-backed evidence. Harmless renames and extractions now
leave those tests intact.

**A green result needs a clearer meaning.**
[check-and-test.sh](../scripts/check-and-test.sh) waits for both tests and lint,
but returns one status without a combined result summary. Its preflight also
runs script tests that bare Cargo does not run. The
[conformance asset helper](../crates/umber/tests/it/e2e_conformance/assets.rs)
permits an explicit `UMBER_CONFORMANCE_ORACLES=optional` downgrade when assets
are absent; Cargo can then succeed with reduced coverage. That exception must
be visible in the final result, not only stderr. Preserve the default failure
on missing required assets.

**Test status has several incompatible meanings.** Executed strict semantic
expected failures can detect an unexpected pass or a changed divergence.
Bibliography tests marked `#[ignore = "xfail: ..."]` are not executed by the
routine suite. Optional full-document tests and subprocess helper tests are
also ignored for entirely different reasons. Report these separately. An
ignored compatibility test remains useful inventory, but is not a passing or
verified expected-failure case.

The bibliography source contains 868 ignored declarations in its upstream
compatibility tree and two separate performance declarations. Some are macro
templates, so these are not counts of expanded cases or distinct product
defects. Its manifest/source audits validate declarations rather than execute
them. Derive the eventual census from expanded test discovery and typed case
metadata, not substring counts.

**At the review baseline, documentation had competing levels of authority.**
[Testing Infrastructure](testing_infrastructure.md) was over 2,000 lines and
mixed commands, detailed tracer operations, old measurements, and migration
history. It described three excluded tool workspaces in one table while
the manifest excluded two. [Architecture](architecture.md) stated
that production contained no unsafe code, while
[tex-dense-prefix](../crates/tex-dense-prefix/src/lib.rs) intentionally contains
the isolated allocator unsafe surface. Reconcile these statements with the
specific storage contract before using them to approve further changes.

**A few modules hold too many responsibilities.** The most conspicuous
examples are `main_control.rs` (9,833 lines), `fork_arena.rs` (7,883),
`world.rs` (7,164), structured scanners (5,888), `command_context.rs` (5,886),
PDF finalization (5,724), `tex-incr/src/lib.rs` (4,563), and the native/virtual
session modules (over 3,500 each). These are navigation indicators, not
automatic deletion targets. The refactors below require a responsibility or
ownership improvement in addition to smaller files.

**At the review baseline, execution tiers required reading many scripts.**
[check-tools.sh](../scripts/check-tools.sh) calls a mixture of contract tests,
feature builds, and excluded-workspace tests. Its `oracle-contract` step
validates contracts; it does not rebuild all reference engines. Several script
self-tests and benchmark commands had no aggregate caller in the reviewed
baseline. Their selection was an operator responsibility, not proof that they
were obsolete. Preserve the explicit dispositions in
[the tooling inventory](tooling_surface_inventory.md).

**Package selection does not prove target-specific test selection.**
`tex-dense-prefix` and `tex-dense-arena` each have WASM-only test modules.
The inspected `check-wasm.sh` invokes wasm-bindgen tests for `umber-wasm`;
building its dependencies does not run those dependencies' unit tests.
Give the allocator crates explicit WASM test steps and extend discovery to
target-specific suites. The host default-member guard remains valuable but
does not establish this separate coverage.

**The review baseline browser distribution integration gate validated an unavailable
placeholder.** [browser-tests/run.mjs](../crates/umber-wasm/browser-tests/run.mjs)
asserted schema 0 and an `unavailable` marker, printed an unavailable message,
and exited successfully. The separate `node-project.mjs` exercised generated
WASM with a custom resolver, and Rust wasm-bindgen tests remained independent
evidence. At that baseline, the aggregate package step could not establish
actual browser distribution/catalog/worker integration. The replacement must
report a blocked prerequisite or actual execution and prove a real package
resource round trip.

## The test model

Use two independent labels: **what a test proves** and **when it runs**.
Unit/integration describe Rust visibility boundaries; regression describes
why a case was added; property-based describes how inputs are generated.
None of those alone tells a reader what protection the test supplies.

### Seven classes, each answering a different question

| Class                   | Question and representative example                                                                                                                  | Why it exists                                                                                          | Primary evidence                                                                                                                         |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Local rules             | Does one algorithm implement its rule? Scaled arithmetic overflow, token recognition, line-break demerits, bibliography sorting.                     | Localizes failures and covers boundary cases cheaply.                                                  | Small explicit expected values or a simple independent model; private tests beside the owning implementation.                            |
| Reference compatibility | Does Umber reproduce the selected TeX/e-TeX/pdfTeX/BibTeX/Biber contract? A macro recovery case or diagnostic ordering fixture.                      | Umber agreeing with itself does not establish compatibility.                                           | Pinned reference output and cited behavior; compare the declared channels with only reviewed normalization.                              |
| State and lifecycle     | Does retry, rollback, editing, cancellation, or publication preserve the right state? Fail a resource request and retry.                             | Most serious integration defects occur across transitions, even when the local algorithms are correct. | Before/after state invariants and cold-versus-incremental/retry equivalence, including diagnostics and generated effects.                |
| Formats and outputs     | Are format images, page artifacts, DVI, PDF, and HTML valid and faithful? Reject a corrupt format or inspect a PDF destination.                      | A successful compilation can still produce corrupt or wrong output.                                    | Exact bytes where contractual, independent structural parsing, semantic assertions, and selected external rendering/validation.          |
| Product and platform    | Can a caller use the CLI, Rust session API, browser worker, cache, and package? A resource round trip through the actual worker.                     | Correct libraries do not prove that their adapters or shipped package work.                            | Public-boundary tests; a small real-browser/package suite in addition to Node mocks.                                                     |
| Limits and performance  | Does hostile input terminate, memory get released, and scaling remain acceptable? Repeated edits, deeply nested lists, or bounded allocation.        | Output equality does not detect hangs, leaks, stack overflow, or catastrophic cost.                    | Deterministic small limit tests routinely; seeded stress, allocation/scaling checks, and controlled performance measurements separately. |
| Test and tool integrity | Are tests selected, fixtures authoritative, and tools honest about failure? Missing oracle, failed fixture publication, or omitted workspace member. | Incorrect test machinery can certify incorrect production code.                                        | Negative tests of discovery, provenance, transaction recovery, selection, and verdict reporting.                                         |

One case may contribute to several contracts, but it needs one primary owner.
A reference fixture and a lifecycle test using the same TeX source are not
duplicates when they observe different behavior. Shared input is not a reason
to delete either assertion.

### The regression backbone to retain

This is the minimum evidence to keep visible during simplification, not a
replacement for all existing cases:

| Contract                    | Existing owners to organize around                                                                      | Required observations                                                                                                                                  |
| --------------------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Language rules and recovery | `tex-command` scanner/expansion tests; command-semantic fixtures; `tex-exec` assignments and mode tests | Consumed tokens, resulting values, expansion/recovery order, terminal/log channels, and effects where declared.                                        |
| Layout and shipout          | `tex-typeset` packing/linebreak/math tests; `tex-exec` page/output tests; canonical DVI corpus          | Chosen breaks, dimensions, page order, exact DVI, and independent diagnostic topology.                                                                 |
| Save, restore, and editing  | `tex-state` rollback tests; `tex-incr` revision tests; Umber effectful replay tests                     | Rejected work changes no accepted output; successful incremental/retried work matches cold execution; effects publish exactly once.                    |
| Startup and generated files | Umber format fixture/cache tests, virtual compilation, project/stabilization fixtures                   | Fresh versus loaded format behavior, corrupt-image rejection, generated-input invalidation, fixed-point convergence/oscillation, complete-job cleanup. |
| Output formats and fonts    | `tex-out`, `tex-fonts`, Umber PDF parity tests                                                          | Structural meaning plus exact contractual encoding, font identity/layout authority, corrupt input and missing-resource handling.                       |
| Bibliography                | Active classic and Biber-compatible `bib-engine` tests and pinned fixture inventories                   | Backend-specific entries, ordering, labels, diagnostics, generated bibliography bytes, and clearly separated unsupported cases.                        |
| Product adapters            | Umber CLI integration, WASM Rust tests, Node tests, real browser/package tests                          | Options and errors, resource batches, binary transfer, cancellation, worker disposal, rendered patches, and usable packaged exports.                   |

Use a small selected cross-cutting matrix for changes to shared owners:
fresh/loaded startup; cold/incremental/retried execution; observation off/on;
successful/rejected/cancelled work; DVI/PDF/HTML and classic/OpenType font
policy where applicable; native/WASM where portable. Every dimension
needs representative evidence, but every fixture need not run the full
Cartesian product. Broad corpus runs complement this diagnostic backbone.

### Execution lanes

Keep the existing commands as the public interface while improving their
selection and reporting. Do not add competing `fast`, `native`, and `all`
wrappers with subtly different definitions of correctness.

| Lane                                            | Existing entry points                                                                              | Required meaning                                                                                                                                                                |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local iteration                                 | `cargo test -q --tests -p <crate>` with a focused filter when useful                               | Selected tests only; report scope.                                                                                                                                              |
| Routine native correctness and quality          | `cargo test -q --tests`; `scripts/check.sh`; `scripts/check-and-test.sh` for the combined workflow | Every host-testable default member, bounded fixtures, and the declared quality feature resolutions. Provisioned assets required; no live reference regeneration during testing. |
| Subsystem checks                                | `scripts/check-wasm.sh`, `scripts/check-tools.sh`, LaTeX and external PDF checks                   | Required for changes to the corresponding product/tool surface; prerequisites explicit and partial selection visible.                                                           |
| Extended compatibility, stress, and performance | Existing full-document, corpus, fuzz, snapshot, node-width, and BibTeX budget commands             | Scheduled or release/subsystem validation with named datasets, seeds, limits, and comparable measurements.                                                                      |

Provisioning and fixture regeneration are **maintenance operations**, not a
fifth correctness lane. Running `scripts/regen-fixtures.sh` produces evidence
to review; a generator exit code is not evidence that Umber matches it.
Document the difference between copying pinned assets into a linked checkout
and the primary-checkout command that may acquire sources and build oracles.

For the routine lane, distinguish cold compile/link time, warmed execution,
and quality-gate time. Establish a fresh quiet-machine baseline before setting
feedback budgets. The existing 30-minute/6-GiB test-process guard is an
operational ceiling, not a satisfactory everyday feedback target. Do not use
the historical approximately 19-second aggregate test time as a fresh result.

Keep performance evidence interpretable:

- `check-snapshot-budgets.sh` enforces warmed instrumentation-based allocation
  and lifecycle budgets; its correctness signal is distinct from Criterion
  timing diagnostics.
- `check.sh node-width-budget` measures Criterion wall time against a matching
  host/toolchain baseline. Preserve its unsupported/blocked result when the
  baseline does not apply.
- `paired-measure.py` already has a documented owner and uses alternating
  measurements, semantic validation, and identity-bearing receipts. Reuse it
  for acceptance measurements; it is not an unowned ordinary test.
- `effectful-rollback-fuzz.sh` defaults to 10,000 randomized cases. Capture a
  replay seed for failures; distinguish it from the scripted deterministic
  thousand-edit test. Small bounded lifecycle cases stay routine.
- Allocator counter assertions must respect their scope. Process-wide counters
  affected by concurrent tests cannot safely assert exact per-test deltas;
  preserve isolated counters or the existing lower-bound checks.

### Make the suite understandable without building another framework

[Testing Infrastructure](testing_infrastructure.md) now starts with the class
table, command/prerequisite table, and result meanings. Detailed tracer
operation moved to [Command-core diagnostic tools](command_core_diagnostics.md).
Keep [Testing Policy](testing_policy.md) focused on authoring rules as later
work changes the suite.

Within existing test modules, use descriptive groups such as `recovery`,
`rollback`, `format_validation`, and `cli`. Put a short module-level statement
of the contract, its oracle, and exclusions where it helps. Keep case identity
in failure messages. Use Rust filters for focused debugging without creating
one Cargo integration binary per class.

The original `tex-exec/src/main_control/tests.rs` was approximately 17,800
lines. Its cases now live in named behavior modules under
`tex-exec/src/main_control/tests/` within the same test target. Apply the same
approach to the large CLI, conformance, expansion,
and line-breaking suites. Preserve test names or update their catalogue
links and scripts atomically.

The CLI, conformance, expansion, and line-breaking suites now have respectively
10, 6, 10, and 8 named behavior modules under their original test targets.
Their shared helpers stay in the same test target; conformance has four
purpose-specific support modules. Full Story, Gentle, TRIP, and e-TRIP gate
functions remain at the original conformance namespace,
so the documented exact manual filters still select those gates. The CLI's
profiling-only case remains feature-gated, as do expansion's two profiling
cases; moving a case into a module did not change its selection tier.

`test-support` currently has seven integration-test targets. Measure whether
one target with modules improves compile/link time before consolidating them.
Likewise, merging executor fixture parity into its integration target must
update the explicit regeneration invocation that names `--test fixture_parity`.
Test-target count is not itself an optimization result.

Use the existing command-semantic runner for new small oracle-backed command
cases. Preserve its typed projections, per-channel comparisons, and strict
expected-failure evaluation. Consolidate fixture I/O and comparison helpers
through `test-support`; retain private engine setup helpers where Rust
visibility or distinct complete-job/fragment semantics justify them.

Extend existing case manifests and command inventories only where selection
or reporting needs the information. Do not require a new manifest row for
every arithmetic unit test. A suite-level index should name its owner, class,
lane, command, prerequisites, and status. Validate actual discovery against
that index for scripts and separately selected suites, using the existing
workspace-selection guard as the model.

Report `PASS`, `FAIL`, `BLOCKED`, and `PARTIAL` at the run level. Within a
selected compatibility suite, count matched cases, executed known failures,
unexpected passes, dormant cases, and unselected cases separately. A timeout,
missing asset, or arbitrary panic must not satisfy an expected divergence.

### Replace brittle tests before reorganizing their implementation

Classify each source-inspection assertion by intended contract:

- For non-forgeable handles and forbidden dependencies, prefer Rust privacy,
  compile-fail tests, or Cargo dependency checks.
- For exactly-once mutation and publication, use observation counts and
  fault-injected lifecycle tests.
- For bounded evidence and allocation, test the bound through a narrow
  test-only counter or explicit performance gate.
- For historical names, comment boundaries, and exact module placement,
  remove the assertion when it has no independent lasting contract.
- Retain small structural audits for constraints behavior cannot establish,
  such as disabled test targets or the approved unsafe boundary. Explain what
  they prove and test their rejection behavior.

Map every removed behavioral assertion to surviving evidence in the review of
that change. Reuse the existing assertion ledgers where appropriate; do not
create another permanently maintained ledger for the same property. Do not
delete reference channels, whole-document cases, or manual tools merely
because a smaller test shares their input.

### Executor source-shape assertion dispositions

The executor pilot retired checks of private Rust spelling while preserving
their observable or compiler-enforced contracts:

| Removed assertion                                                                                                             | Active evidence and invariant                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `live_shipout_has_no_second_dvi_emitter` counted compiler calls in one file.                                                  | `fresh_and_memo_shipouts_share_canonical_artifact_dvi` compares emitted DVI bytes for fresh and memo paths; `dvi_disabled_fresh_and_memo_shipouts_both_omit_plans` checks both disabled paths. The single canonical artifact-to-DVI result is the observable contract.                                                                                                                                                                          |
| `receipt_categories_are_append_bounded_consumed_and_closed_before_commit` scanned method bodies and comment-delimited slices. | `every_receipt_category_is_bounded_and_consumed` exercises all six append categories at the exact record limit, confirms rejection without growth, and compares terminal consumption with reset. `unified_operation_preserves_state_output_and_typed_evidence`, `observed_pdf_fatal_error_publishes_its_committed_receipt`, and `observed_pdf_dvi_preflight_error_discards_its_uncommitted_receipt` exercise publication and rollback ordering. |
| `command_host_facts_are_sampled_only_by_the_consuming_query` counted provider calls and field spellings.                      | The test now compares host-query and effective-tail telemetry for ordinary delivery, a mode enquiry, and `\lastnodetype`; `etex_lastnodetype_reads_each_live_mode_tail_without_mutation` checks the returned values in multiple modes. The telemetry verifies that ordinary delivery does not sample executor facts and mode queries do not traverse the tail.                                                                                  |
| `every_processor_borrows_the_singular_fuel_ledger` counted constructor names and inspected field spelling.                    | `command_fuel_can_only_be_owned_by_a_session_ledger` compiles forbidden construction and field-access probes and requires rejection; `session_ledger_lends_typed_fuel_without_transferring_ownership` checks monotonic borrowing. Fuel abort tests in `resource_replay.rs` exercise cleanup of scanner and operation children.                                                                                                                  |

Private owner placement and the count of constructor names are not public
contracts. The compile-fail fuel tests remain because they enforce a real
capability boundary, and the `workspace_selection` source audit remains because
it detects disabled production or test code that runtime fixtures cannot see.

### Non-executor source-shape assertion dispositions

The September 2026 test cleanup removed private spelling checks with these
active owners:

| Removed assertion                                                                              | Disposition and surviving evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tex-state` universe field names, obsolete command-state names, and `CommandContext` byte size | Retired representation and size constraints. `command_episode_admits_session_and_generation_once`, `runtime_checkpoint_fork_moves_the_checkpoint_bank_without_new_payload_owners`, and the page/scratch fork tests in `universe/tests.rs` exercise admission and lifetime behavior. The runtime-storage contract ledger excludes Rust type size as a compatibility contract.                                                                                                                                                                                                                                                                          |
| `tex-state` shared-copy function body and helper call names                                    | Replaced by existing `explicit_shared_copy_scaling_counts_each_node_once_at_required_sizes`: it checks one semantic clone per node, zero measured allocations after warmup, stable source values, and sizes through 4,096. The range and rollback tests cover cross-chunk copies and rejection.                                                                                                                                                                                                                                                                                                                                                       |
| `tex-incr` DTO field-name exclusions and prior-generation spelling                             | Retired private representation constraints. The active compile-fail tests in `tests/it.rs` reject cross-generation ids, escaping brands, an outlived reachability store, and a generation owner in detached history. Revision rejection and repeated cold-equivalence tests protect publication behavior.                                                                                                                                                                                                                                                                                                                                             |
| `umber` runtime source paths and exact entry names                                             | Retired routing layout constraint. CLI, direct, format, virtual compile, editor, and incremental tests execute those routes; `tex_etex_pdftex_fresh_and_twice_loaded_format_matrix` checks fresh and loaded modes. The browser route remains owned by its separate WASM gate.                                                                                                                                                                                                                                                                                                                                                                         |
| `tex-command-stream` loaded-route function bodies and generic-provider call spelling           | Retired migration-only delegation constraint. The provider boundary is exercised by `every_loaded_job_has_fresh_clock_terminal_and_mutable_state`, independent cache reuse in `declared_command_semantic_cases_match` (explicit parity tier), and focused loaded command cases in `command_semantic.rs`.                                                                                                                                                                                                                                                                                                                                              |
| `umber` retained pdfTeX fixture count of eight                                                 | Replaced with a nonempty runner assertion. `retained_pdftex_extension_fixtures_compare_oracle_projections` executes every catalogue-owned case, while `retained_pdftex_extension_properties_have_complete_unique_active_ownership` derives the complete case inventory from the closed fixture tiers and rejects missing or overlapping ownership.                                                                                                                                                                                                                                                                                                    |
| `umber` conformance helper source spelling and route-body scans                                | Retired private placement and call-spelling constraints. The pinned Plain recipe and typed job-resource tests check construction inputs and identity; `trip_profiles_reuse_verified_provider_entries_and_fresh_jobs` and `plain_provider_reuses_one_verified_construction_with_fresh_jobs` check authenticated cache reuse and fresh mutable loaded jobs. The Story/Gentle and focused canonical DVI gates execute the selected routes, while phase-channel and format-image controls check their output and detachment. The separate gate-registry source audit remains because an unused registered oracle cannot be detected by output comparison. |

The `workspace_selection` source audit stays: it checks whether production
branches or library tests are silently disabled, which runtime behavior cannot
prove, and its positive and negative fixtures verify rejection. Compiler privacy
and the compile-fail tests above also stay because they prove that forbidden
handles cannot be forged or moved across lifetime boundaries. These are narrow
capability and test-activity policies, not requirements on private naming or
file layout.

The line-breaking test families still contain three exact Rust-size checks for
`Candidate`, `BreakSite`, and `MaterializationAction`. They constrain memory
used per active route or paragraph node, and no behavioral test proves the
same byte budget. Their replacement belongs with measured storage evidence;
this test organization pass leaves them active.

## Code simplification sequence

The following sections propose responsibility changes within the existing
architecture. Preserving public interfaces and actually supported wire formats
is a requirement; documented promises that conflict with implementation need
an explicit disposition first. Compatibility adapters can remain at external
boundaries; superseded internal owners should disappear when a migration lands.
Intentional representations such as owned/streaming codecs and semantic/
diagnostic node channels are not superseded owners merely because they coexist.
The evidence work comes first. The remaining sections group work by owner;
the delivery order below starts with bounded pilots before broad core changes.

### 1. Make the evidence and current contract trustworthy

Reconcile the architecture and testing entry documents with current source.
Produce suite discovery and status summaries, aggregate combined-gate results,
and classify the expensive/manual tools. Fix prerequisite declarations and
distinguish oracle-contract validation from live oracle construction.

Use the executor source-shape tests as the first pilot for replacing layout
constraints with behavioral or capability evidence. This unlocks later
refactors and provides a concrete test of whether the new organization helps.

### 2. Reduce executor coordination and scanner coupling

The first mechanical extraction should be
[structured scanners](../crates/tex-command/src/scanners/structured.rs): PDF
actions/resources, box requests, math requests, and input scanning have
distinct responsibilities but currently share one large file. Keep the
existing reexports and scanner episode ownership. Follow with semantic
deduplication only after the move passes its original fixtures.

The extraction now places those families under `scanners/structured/` while
the existing `CommandProcessor` remains the sole scanning and recovery owner.
[Structured scanner ownership](structured_scanner_ownership.md) records the
module boundaries. The old fixture and source-boundary gates remain the
acceptance evidence for this mechanical step.

Keep one operation authority. Separate command delivery, scanned command
payloads, semantic application, diagnostic rendering, and commit/rollback
settlement by their existing ownership seams. A split is successful when a
command family can be understood without reading all other families and when
one transition still has one mutation/settlement owner.

Preserve TeX token-consumption and error ordering. Scanning is observable:
changing when a token is read can change recovery, expansion, grouping, and
resource replay. Begin with a bounded assignment family and compare exact
command/terminal/log/effect channels before generalizing the pattern.

Audit module-wide `allow(dead_code)` in `macro_call.rs`, `scan_toks.rs`,
`continuation.rs`, and input modules against actual callers. Comments naming
the "next integration slice" can be stale. Remove an allowance where the
implementation is now live; migrate or retire an unused private representation
only after checking feature and diagnostic consumers. Expansion already lives
inside `tex-command`; a new expansion crate would reverse that consolidation.

That audit found the detached-continuation implementation unreachable from
production in both default and all-feature builds. Its module was private, its
types had no external API, and only its own tests constructed the recipe graph.
The unused implementation and its seven prototype-only direct tests were
retired; active structured-scanner tests retain their original cases and names.
The macro-call, token-list,
and input modules now expose their live paths without module-wide dead-code
allowances; direct-test adapters are test-only, and unused private helpers were
removed. Any future detached transport must enter through a real production
caller and receive evidence for that boundary.

### 3. Separate state storage, semantic state, and host effects

The highest-value ownership audit candidate is the legacy node adapter surface.
[node_arena.rs](../crates/tex-state/src/node_arena.rs) explicitly allows dead
code as retained compatibility substrate, while also defining the modern
borrowed `NodeCursor`. [page_node_arena.rs](../crates/tex-state/src/page_node_arena.rs)
offers both cold materializing views and compact cursors; `Universe` still
maps some compact errors into the legacy error type. Establish callers for
each surface, move modern views away from legacy ownership, migrate the
remaining internal users, and delete the old owner and conversions together.
Preserve public compatibility APIs unless separately approved for retirement.
Do not assume a commented prototype is the production storage representation.
The compatibility module still has production and public consumers; its
comment and dead-code allowance are insufficient deletion evidence.

The bounded node-view pass keeps the public `NodeArena` and cold
`list`/`span_list` APIs. It removes unused private page aliases and moves the
borrowed `NodeView` and `NodeCursor` projections behind public reexports in
`node_arena`. Output-box kind and dimension queries now use a borrowed cursor.
Page-list membership checks admit the compact root and validate each record
without collecting an owned list. The existing decoder reads fixed-size annex
payloads into stack arrays, so ordinary box, penalty, and math records avoid
allocation during this check. Ligature source and variable-byte whatsit
decoding still allocate; the membership check shares their cold decoder so
malformed-record rejection follows the same rules.

Decompose `world.rs` around resource/input records, effect publication,
artifact/provenance values, and host services. Keep its transactional boundary
explicit; do not turn every piece into a separately checkpointed service.

Decompose `fork_arena.rs` around physical storage, logical coordinates, list
topology, and ownership transfer. First audit the responsibilities already in
`tex-dense-prefix` and `tex-dense-arena`; avoid inventing another arena
abstraction over the same storage. Keep the isolated unsafe boundary small.
The acceptance criterion is fewer independently updated ownership records,
with the same accepted/candidate lifetime and exact rollback guarantees.

[ParagraphTape](../crates/tex-typeset/src/linebreak/mod.rs) currently accepts
owned, borrowed mirrored, borrowed arena, and arena-ID sources. Audit which
are production inputs and which support tests or public compatibility. Prefer
one production traversal/materialization path, retaining explicit adapters
where callers require them, and only where lifetime and performance evidence
show that the inputs can share it. These forms are not automatically duplicate
storage. The semantic and physical diagnostic node
channels carry different evidence and must remain distinct.

### 4. Clarify session and output ownership

Mixed resource admission is implemented. `VirtualCompileSession` stages its
workspace, font map, non-file admission state, and PK-font map in one rollback
guard; registration into incremental input state occurs before the guard
commits. `LatexProjectSession` separately stages project resources and commits
them only after child TeX admission succeeds. Authorization and font policy
remain explicit at their respective layers.

Session configuration, resource admission, execution candidates, accepted
revisions, and output finalization remain in `VirtualCompileSession` because
its public API coordinates their shared lifecycle. Private resource admission
and publication methods isolate the two transaction boundaries. Further
bundling is justified only when fields share an invariant.

One candidate publication transaction now protects the accepted output and
workspace through resource arrival, failure, cancellation, and patch rejection.
`TexFixedPointSession` already delegates to `LatexProjectSession`; preserve
that reuse. The editor's provisional/stable distinction and bibliography
fixed-point passes represent different product behavior and should not be
collapsed into one generic session state machine without proof of equivalence.

The retained session publication seam prepares memory effects, auxiliary and
rendered outputs, output limits, and the HTML update while the candidate and
generated transaction are still private. It accepts generated files into a
private workspace, computes generated-file fingerprints, accepts the
incremental revision, and installs the accepted output, workspace, and render
state together at the host session boundary. The incremental layer retains its
own candidate acceptance validation and error contract. Loaded format bytes
remain available until the first revision is accepted, so a failed initial
output preparation can retry with the same format. An HTML update for a valid
nonconsecutive revision is a full snapshot, since patches require adjacent
revision numbers. `EngineSession` keeps its bounded execution and resource
resume role; `tex-incr` keeps candidate validation and revision acceptance.
Neither owns the host-facing output bundle or HTML delivery state.

For native/browser resource loops, share a transition specification and
cross-platform fixtures for attempt, need, admission, speculative drain, retry,
and acceptance. Native filesystem policy and browser asynchronous fetch/cache
policy should remain separate. An empty speculative response, cancellation,
and an unavailable required resource need distinct transitions.

### 5. Simplify downstream families using the same method

At review time, the artifact codec had a concrete cleanup opportunity:
[binary.rs](../crates/tex-out/src/binary.rs) accepted only version 24 at both
its owned and streaming header boundaries, but retained branches for versions
13 through 23. Its facade still described append-only compatibility. Confirm
all decoder entry points, preserve the version-24 acceptance and older-version
rejection behavior, and remove unreachable migration branches.
If historical decoding is a required product promise, treat implementing it
as separate compatibility work rather than silently broadening this cleanup.

Then separate owned encode/decode, streaming emission, and borrowed scanning,
sharing wire tags, headers, and limits. Keep owned-versus-streaming byte
identity and malformed-input error tests. Versioned legacy type names such as
`V10ArtifactBuilder` can be hidden behind the existing version-independent
aliases without breaking external callers.

The codec cleanup now keeps the exact version-24 header boundary in both
decoders, with explicit rejection checks for every older version across all
decode entry points. The facade retains the public versioned names and aliases;
the implementation separates bounded primitive I/O, shared wire tags, owned
encoding and decoding, iterative node traversal, streaming emission, and
borrowed scanning. Extraction does not change the artifact representation or
introduce historical decoding.

Split oversized PDF lowering helpers within the existing boundaries:
`finalization.rs` owns detached input, `finalize.rs` lowers it, `pdf.rs`
validates and hashes the graph, and `serialize.rs` writes bytes. Keep one
object-numbering and final publication authority. Use
semantic PDF queries for meaning, exact bytes for deterministic serialization,
and external validators/rendering for independent artifact acceptance.

In `pdf/finalize.rs`, navigation, font assembly, raster/image helpers, and
page/form content lowering are distinct extraction candidates. Page and form
loops share traversal mechanics but have different resource/geometry policies;
preserve those policies explicitly when consolidating the mechanics. Exercise
resident, Type1, TrueType, and PK font branches, including subsets, shared
encodings, ToUnicode, and virtual fonts. Their resource identities and
embedding rules are not interchangeable.

The finalization extraction now keeps the allocation cursor, indirect-object
collection, graph validation, serialization, and diagnostic publication in
`finalize_pdf`. Private navigation, content, font, image, and numeric modules
borrow that operation's state. Page and form assembly retain their separate
geometry and resource policies. See [PDF Finalization Module Boundaries](pdf_finalization_modules.md) for the current file map and ownership rule.

Treat classic BibTeX and the Biber-compatible pipeline as separate semantic
backends. Share source acquisition, resource identity, and orchestration where
already common; do not force their different algorithms into a universal VM
or one option model. Preserve both active conformance evidence and the honest
inventory of unsupported upstream cases.

The Biber worker currently does not invoke every independently tested graph
and sorting capability. For example,
[BiberWorker](../crates/bib-engine/src/biber/mod.rs) uses default graph options
and does not run `DataListBuilder`. A sorting unit test is therefore not
evidence that the complete Biber pipeline supports the corresponding upstream
case. Preserve that distinction; connecting unfinished functionality is
separate feature work, not a prerequisite for code simplification.

In bibliography test scaffolding, use typed access to the existing manifests
and shared file/hash validation instead of repeating their names, counts, and
roles throughout Rust. Retain explicit upstream identity and completeness
assertions. Split broad serializer and foundation test files by format/backend
within their existing targets. An explicit strict compatibility lane can be
added incrementally for cases with structured failures; do not blanket-run
all dormant cases and treat any failure as the expected one.

Keep browser adapters thin around the shared Rust session contract, with
browser-specific cancellation, transfer, cache, and DOM rules tested at their
actual boundary. Cross-language validation is not automatically duplicate
code: independent decoders must reject malformed inputs on both sides.

The browser prefetch policy now has one Rust owner. `manifest-resolver.js` uses
the generated WASM policy when complete bindings are available, while Node
transport tests inject an explicit fake. A catalog-only standalone resolver
disables speculation and preserves required resource acquisition; binding Rust
before `beginRun` enables prediction. The JavaScript scanner and queue/replay
fallback were retired after generated WASM catalog/prefetch coverage landed.
Disabling speculation can change performance and telemetry even when
compilation output is unchanged. The exact behavior is recorded in
[Browser Prefetch Policy Ownership](browser_prefetch_policy.md).

Keep JSON/fake-binding resolver tests for transport and cache behavior. Add
real packed-catalog batch admission, tampering/mispartition rejection, and
one packaged worker resource round trip. Share normalization, typed identity,
font-declaration, replay, and dependency vectors across policy boundaries
rather than implementing another packed decoder in JavaScript.

## Delivery and acceptance

| Stage                          | Concrete deliverable                                                                                                       | Exit condition                                                                                                                       |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Establish baseline             | Current class/command index, missing-prerequisite and ignored-case inventory, representative behavior/performance captures | A reader can identify which command protects each supported surface and what a green result means.                                   |
| Make refactoring safe          | Executor source-shape pilot and honest combined/subsystem result summaries                                                 | Selected harmless renames/extractions no longer break layout assertions; induced behavior and reporting failures are still caught.   |
| Prove one simplification       | One bounded command family and one state/session ownership seam                                                            | Fewer independent owners/conversions, unchanged contractual outputs, relevant lifecycle failures covered, measured costs acceptable. |
| Expand successful patterns     | Executor, storage, sessions, PDF, and bibliography work separated by owner                                                 | Each change deletes its internal predecessor and passes the applicable lanes; no second architecture accumulates.                    |
| Stabilize the new organization | Reconciled docs, retired migration scaffolding, repeated baseline measurement                                              | Remaining exceptions are understandable and owned; the routine and subsystem commands give trustworthy scoped verdicts.              |

Start with the baseline and source-shape pilot. Do not simultaneously rewrite
the scanner, arena, and incremental/session transaction: a failure would be
too difficult to localize. Once the pilot is proven, work on independent
owners can proceed concurrently; changes sharing a mutation boundary should
remain serialized.

The first reviewable changes should be:

1. Make combined-gate, missing-oracle, and unavailable browser-integration
   verdicts honest, inventory separately selected suites, and reconcile the
   short testing entry documents. Preserve all existing selection until the
   inventory establishes an explicit owner.
2. Split the executor test file by behavior and replace a bounded cluster of
   source-shape assertions. Keep fixture channels and catalogue links intact.
3. Remove verified unreachable artifact-version branches, retaining byte
   identity, old-version rejection, and existing public aliases. This is the
   first small production simplification to validate the process.
4. Extract mixed resource admission behind the existing session API, proving
   failure atomicity at each stage. Then expand the ownership refactor into
   executor/scanner and legacy-node work using the same evidence discipline.

During those pilots, perform the legacy-node caller inventory and identify
the exact internal owner/conversions that can be removed together. Do not
defer that larger simplification indefinitely in favor of cosmetic file moves.

For each production refactor, preserve current CLI/API behavior, complete-job
versus fragment semantics, diagnostics and effects, accepted/rejected revision
behavior, supported output/format versions, and bounded execution. Record
existing known failures separately so cleanup neither claims to fix them nor
silently broadens them. Do not regenerate expected outputs from the modified
Umber implementation to make a refactor pass.

Before deleting an adapter or representation, audit repository callers,
feature/target configurations, public types, format/error contracts, and
diagnostic consumers. Establish the replacement evidence first. If a selector
or fallback remains, test its invalid/rejected path so accidental re-entry
cannot masquerade as successful migration. An ownership map must precede
extraction from `World` or the arena; do not create new services, checkpoint
owners, or conversion layers simply to shorten those files.

Measure progress by the number of independent owners and conversion paths
removed, tests that survive an implementation-only change, time to identify
the failing contract, and comparable build/runtime/retention costs. Reduced
line count is useful supporting evidence, not the objective. No percentage
code-deletion target is justified by this review.

## Review validation and limits

The baseline was a source and test-design review, not a claim that the full current
native, browser, corpus, or performance suites pass. Cargo metadata was
inspected without a build. The existing `scripts/check-wasm.sh node-unit`
step passed 105 tests with no skips; its verdict was `PARTIAL`, one of six
steps selected. No reference fixtures were regenerated or production code
changed for this review.
The authoritative `scripts/check.sh` reported all four gates passed, including
both clippy feature resolutions over 32 workspace members. The Markdown gate
was rerun after subsequent documentation edits.
