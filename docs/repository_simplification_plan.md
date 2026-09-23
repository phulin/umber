# Repository simplification and testing plan

Status: this is the first-wave bounded nonbibliography review and its
revision-specific validation receipt. A later authorized breaking cleanup
removed implementation surfaces that this review had retained, including the
generic `NodeArena<L>`, owned paragraph traversal, obsolete World artifact-hash
storage, and compatibility aliases. Read [Compatibility Surface Removal](compatibility_surface_removal.md)
for the current owners and the later corpus criteria; the retained-API
dispositions below are historical, not current API guidance. The local Plain
format was regenerated for schema 12; default browser distribution deployment
remains unpublished. Bibliography migration is deferred.
Review baseline: `11c7bf7cd8ec78b36337c3ee3a97e38d0dc099e9`.

The objective is to preserve Umber's current behavior while making both the
implementation and its evidence easier to understand. The first priority is
making the tests safe to refactor against; the second is reducing the number
of responsibilities and independently maintained representations in the core.
This document preserves the review rationale and proposed sequence. For the
current testing entry point, use [Testing Infrastructure](testing_infrastructure.md)
and [Testing Policy](testing_policy.md).

The testing pass established the seven-class front door, executable
script-suite inventory, and aggregate gate verdicts. Its test-family splits
and private source-guard dispositions make implementation-only changes easier
to review. The bounded implementation passes are recorded under their owners
below. Shared resource-transition vectors now run against both the native
session and generated WASM package. The acceptance results below identify the
selected revision and steps; they do not imply that every optional tier ran.

### First-wave implemented scope and then-retained boundaries

| Owner                        | First-wave disposition                                                                                                                                                                                                                                                                                                          | Evidence to keep visible                                                                                                                                                                                                |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Test selection and reporting | The script-suite inventory, seven behavior classes, combined verdict, and explicit optional-step verdicts are live. Executor, CLI, conformance, expansion, and line-breaking tests are grouped within their original targets. Private spelling guards were replaced with behavior, compile-fail, or narrow structural evidence. | `scripts/check-and-test.sh`, `scripts/check.sh`, `scripts/check-wasm.sh`, `test-support` workspace selection, and the assertion dispositions below.                                                                     |
| Command and state            | Structured scanner families and their ownership are explicit. Unreachable detached-continuation code is removed. `MainControl`, `World`, `ForkArena`, and `CommandContext` methods are grouped by responsibility without adding a second state or checkpoint owner.                                                             | Existing command fixtures, scanner recovery and resource replay tests, state rollback tests, and [responsibility boundaries](state_responsibility_boundaries.md).                                                       |
| Resources and sessions       | Mixed file/font/project admission is staged and rejected atomically. A private publication seam prepares output before accepting the revision and installs accepted output, workspace, and render state together. Valid nonconsecutive HTML revisions publish a full snapshot.                                                  | Virtual/project admission rejection tests, accepted-revision and render-update tests, and four shared native/generated-WASM transition cases.                                                                           |
| Nodes and output             | Borrowed node views and compact page reads are separated from the generic legacy `NodeArena<L>`. Artifact codec accepts only version 24 while preserving public aliases and owned/streaming byte identity. PDF lowering has private content, navigation, font, image, and numeric modules under one finalization authority.     | Node checkpoint/borrow tests; old-version rejection and codec identity; PDF semantic tests and the explicit qpdf/Poppler gate.                                                                                          |
| Browser                      | Rust owns prefetch policy; generated packed-catalog and real package tests cover browser worker resource flow. Standalone catalog-only resolution disables speculation until bindings are installed.                                                                                                                            | Node transport tests, Firefox wasm-bindgen tests, allocator WASM steps, and Chromium package checks. The local Plain image has schema-3 metadata for format schema 12; the default hosted manifest remains unpublished. |

At the first-wave review, the public legacy node API, semantic and physical
diagnostic node channels, page/form PDF policies, and browser-specific
fetch/cache behavior served distinct contracts. Their coexistence was not
evidence of a duplicate production owner.
The caller-backed storage audit removed `ForkArena`'s synchronized live frontier
and unified the borrowed `ParagraphTape` cursor path. It retained World live
counts, publication columns and cursors, and the owned and arena-ID paragraph
paths for the distinct API, lifetime, and diagnostic contracts recorded in
[State Responsibility Boundaries](state_responsibility_boundaries.md). This is a
resolved boundary under the then-current functionality contract, not an unexamined
storage rewrite. Classic BibTeX and Biber-compatible migration is separate
work; this review does not claim its ignored upstream cases execute. The later
breaking cleanup removed the generic node API and owned paragraph path while
preserving the live semantic and physical diagnostic evidence.

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
The executor and non-executor dispositions below map the retired private guards
to behavioral or compiler-backed evidence. Harmless renames and extractions
no longer invalidate those tests.

**At the review baseline, a green result needed a clearer meaning.**
[check-and-test.sh](../scripts/check-and-test.sh) now reports a combined
`PASS`, `FAIL`, `BLOCKED`, or `PARTIAL` verdict for its native, quality, and
script steps. Bare Cargo still selects only its own Rust tests. The
[conformance asset helper](../crates/umber/tests/it/e2e_conformance/assets.rs)
permits an explicit `UMBER_CONFORMANCE_ORACLES=optional` downgrade when assets
are absent; Cargo can then succeed with reduced coverage. The combined runner
reports that downgrade as `PARTIAL`. Missing required assets remain failures
by default.

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
Building them as dependencies does not run those unit tests. `check-wasm.sh`
now has explicit `dense-prefix-wasm` and `dense-arena-wasm` steps alongside
the `umber-wasm` Firefox step. The host default-member guard remains valuable
for its separate native selection contract.

**The review baseline browser distribution integration gate validated an unavailable
placeholder.** At the baseline, [browser-tests/run.mjs](../crates/umber-wasm/browser-tests/run.mjs)
asserted schema 0 and an `unavailable` marker, printed an unavailable message,
and exited successfully. The separate `node-project.mjs` exercised generated
WASM with a custom resolver, and Rust wasm-bindgen tests remained independent
evidence. The replacement now builds a generated package and packed catalog,
then runs resource, tampering, and worker checks in a real browser. The
Plain format had schema-0 unavailable metadata at the integrated revision, and
its separately selected `default-format` step reported `BLOCKED`. The local
image now has schema-3 metadata for a schema-12 image.

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

`test-support` currently has seven integration-test targets. Their proposed
merge was conditional on a measured compile/link benefit, not a required
structural edit. The frozen [paired performance comparison](repository_simplification_performance.md)
measured the representative `tex-exec` lib-test target, not an isolated
seven-target merge; no such merge was made. Merging executor fixture parity
would also have to update the explicit regeneration invocation that names
`--test fixture_parity`. Test-target count is not itself an optimization result.

Use the existing command-semantic runner for new small oracle-backed command
cases. Preserve its typed projections, per-channel comparisons, and strict
expected-failure evaluation. The literal proposed migration of its fixture
I/O and comparison helpers into `test-support` was not made. `test-support`
already owns generic closed-case and DVI normalization helpers;
`tex-command-stream::semantic::channels` is the one typed channel-comparison
authority shared by the fixture gate and regeneration tool. The local
`compare_declared_channels` adapter only reads the declared channel files and
calls that authority. Moving it would add a dependency without removing a
second semantic comparator. Private engine setup stays where Rust visibility
and complete-job versus fragment behavior require it.

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

The following sections retain the review's diagnosis and describe the bounded
changes that landed under each owner. Recommendations outside that pass remain
explicitly scoped audits. Public interfaces and supported wire formats were
preserved. Compatibility adapters can remain at external boundaries; a
superseded internal owner should disappear when its replacement lands.
Owned/streaming codecs and semantic/diagnostic node channels intentionally
serve different consumers.

### 1. Make the evidence and current contract trustworthy

The testing entry documents now describe classes, commands, prerequisites, and
verdicts. The script inventory classifies separately selected tools and
distinguishes oracle-contract validation from live construction. The executor
source-shape pilot replaced private spelling checks with behavioral and
capability evidence; the disposition table above names the surviving tests.

### 2. Reduce executor coordination and scanner coupling

The baseline [structured scanner](../crates/tex-command/src/scanners/structured.rs)
file mixed PDF actions/resources, box requests, math requests, and input
scanning. The extraction kept its reexports and scanner episode ownership.

The extraction now places those families under `scanners/structured/` while
the existing `CommandProcessor` remains the sole scanning and recovery owner.
[Structured scanner ownership](structured_scanner_ownership.md) records the
module boundaries. The old fixture and source-boundary gates remain the
acceptance evidence for this mechanical step.

`MainControl` now groups discretionary processing, resource handling, and root
source methods by command responsibility while keeping one operation and
settlement authority. [Main-Control Responsibility Boundaries](main_control_responsibilities.md)
records that extraction. A later semantic split still needs exact channel and
rollback evidence; file movement alone does not authorize it.

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

The review identified the legacy node adapter surface as an ownership audit.
[node_arena.rs](../crates/tex-state/src/node_arena.rs) explicitly allows dead
code as retained compatibility substrate; [page_node_arena.rs](../crates/tex-state/src/page_node_arena.rs)
offers cold materializing views and compact cursors. The generic public
`NodeArena<L>` has no production constructors found outside its module; its
own tests and public compatibility contract remain. The live `PageNodeArena`
alias instead refers to `PageMaterialArena`, and the borrowed
`NodeView`/`NodeCursor` projection is used by production page readers. The
private borrowed projection and page membership path were separated as
described below; no public API or legacy-error contract was retired. A later
deletion requires an explicit public-compatibility decision.

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

`World` now groups input dependencies, effect publication, and checkpoint
methods; `ForkArena` groups checkpoint, batch, and whole-region transfer
methods; `CommandContext` groups PDF and page methods. These remain inherent
methods on the same types, with one checkpoint authority. The arena pass also
removed its separately synchronized live frontier: the accepted chunk set now
supplies the end and tail in constant time while the physical pool still owns
payload. [State Responsibility Boundaries](state_responsibility_boundaries.md)
records why the remaining World live counts, public borrowed columns, and
publication cursors are distinct facts. Its snapshot scalars are rollback
inverses, not another live owner. A wrapper around `tex-dense-prefix` or
`tex-dense-arena` would not remove an owner or conversion.

[ParagraphTape](../crates/tex-typeset/src/linebreak/mod.rs) now uses one
`NodeCursor` source for borrowed slice and borrowed page-arena traversal.
`tex-exec` uses the arena-ID path in production so it can release the execution
context borrow before its retained range sink runs. The owned sequence serves
the public typesetting API and detached semantic and physical projections.
Those diagnostic channels carry different evidence and remain distinct.

### 4. Clarify session and output ownership

At the review baseline, `VirtualCompileSession::provide_resources` manually
staged and swapped the workspace, font map, non-file admission state, and
PK-font map, then restored them after rejection. `LatexProjectSession`
separately staged project resources. The implemented admission transaction
stages mixed resources before publishing them to either layer. Registration
into incremental input state occurs before its rollback guard commits. The
current `tex-incr::Session::register_input_files` batch is infallible after
preparation; a future fallible validation step needs a targeted rollback test
before atomicity is claimed for that phase. Current tests reject invalid mixed
batches, late child font admission, and file-then-font retry without changing
the accepted revision.
Authorization and font policy remain with their respective layers.

`VirtualCompileSession` still owns configuration, resources, candidates,
accepted revisions, and output finalization because its public API coordinates
their shared lifecycle. Private admission and publication methods isolate the
two transaction boundaries. Repackaging unrelated fields would add another
representation without clarifying authority.

One candidate publication transaction now protects accepted output and the
workspace through resource arrival, failure, cancellation, and patch rejection.
The private seam prepares memory effects, auxiliary and rendered outputs,
output limits, and the HTML update while the candidate and generated
transaction are still private. It accepts generated files into a private
workspace, computes their fingerprints, accepts the incremental revision,
then installs accepted output, workspace, and render state together at the host
session boundary. The incremental layer keeps its own candidate-validation
error contract. Loaded format bytes remain available until the first revision
is accepted, so a failed initial output preparation retries with the same
format. A valid nonconsecutive HTML revision gets a full snapshot because a
patch requires adjacent revision numbers. `EngineSession` retains bounded
execution and resource resume; `tex-incr` retains candidate validation and
revision acceptance. Neither owns the host-facing output bundle or HTML
delivery state.
`accepted_publication_is_atomic_across_render_gap_output_failure_and_retry`
checks the revision-gap snapshot and a failed output budget: accepted output,
generated files, render state, reusable inputs, and stabilization remain at the
last accepted revision until a retry succeeds.
`rejected_render_patch_keeps_accepted_revision_and_retries` exercises direct
HTML planner rejection, and
`loaded_format_survives_initial_publication_failure_and_retry` compares the
retry with a fresh loaded-format run.

`TexFixedPointSession` already delegates to `LatexProjectSession`; preserve
that reuse. The editor's provisional/stable distinction and bibliography
fixed-point passes represent different product behavior and should not be
collapsed into one generic session state machine without proof of equivalence.

The shared [resource-transition cases](../tests/resource-transition-cases.json)
now drive the public native `VirtualCompileSession` and the generated WASM
package session with its real HTTP manifest resolver. Four cases cover a
required positive retry, authoritative absence from a blocking probe, an
empty speculative response followed by required demand, and cancellation of a
resource-waiting source patch followed by the same revision's successful retry.
The positive case also rejects a conflicting late response without admitting
part of its batch. Both runners compare ordered typed request roles, candidate
privacy, and accepted terminal observations. The browser shared-case runner uses the real
packed-catalog decoder with speculation disabled for its catalog-only resolver;
separate package tests exercise the full Rust prefetch policy. Native file
lookup and browser fetch, cache, worker, and abort-signal behavior keep their
host-specific tests. [Resource Lifecycle](resource_lifecycle.md) records the
bounded common contract.

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

JSON/fake-binding resolver tests still cover transport and cache behavior.
The generated package tests now exercise packed-catalog batch admission,
tampering and mispartition rejection, and a worker resource round trip. Shared
Rust policy supplies normalization, typed identity, font declarations, replay,
and dependency vectors without another JavaScript packed decoder.

## Delivery and acceptance

| Review deliverable                          | Disposition                                                                                                                                                                                            | Acceptance evidence and limit                                                                                                                                                                |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Test selection and honest verdicts          | Implemented. The seven classes, script and selected-Rust inventories, required-asset behavior, and combined/subsystem verdicts have one documented entry point.                                        | `test-support` selection tests, gate-verdict and inventory tests, `scripts/check-and-test.sh`; a selected optional subset reports `PARTIAL`.                                                 |
| Refactoring-safe test organization          | Implemented. Executor and other large test families retain their original Cargo targets; retired private source checks have named replacement evidence.                                                | Original fixture/parity selections, executor receipt and host-fact tests, compile-fail capability checks, and the assertion tables above.                                                    |
| Bounded production simplification           | Implemented. Scanner, command, state-method, node-view, resource-admission, session-publication, artifact codec, PDF lowering, and Rust prefetch changes keep one authority at each mutation boundary. | Revision-specific combined native/quality, real WASM/package, and external PDF receipts must be read with the limits below.                                                                  |
| Broader storage and paragraph consolidation | The audited `ForkArena` frontier and duplicate borrowed paragraph path were removed. World counts and publication columns remain for the documented API, history, and admission contracts.             | Arena and paragraph rollback/parity tests; [State Responsibility Boundaries](state_responsibility_boundaries.md) records why the remaining live World fields are retained.                   |
| Shared native/browser transition fixture    | Implemented as four ordered file-resource cases over the public native and generated WASM sessions.                                                                                                    | Both runners check request roles, positive/negative/empty outcomes, cancellation while a patch is pending, atomic rejection, and accepted observations; host-specific policy stays separate. |
| Bibliography migration                      | Deferred. Classic and Biber-compatible backends and their ignored upstream inventory keep their existing status.                                                                                       | A future scoped pass must distinguish executed matched cases, strict known failures, ignored declarations, and unsupported cases.                                                            |

The original first four pilots are complete: verdict and inventory work,
executor test organization and source-guard replacement, version-24 codec
cleanup, and mixed resource admission. Subsequent bounded scanner, command,
state, PDF, session-publication, and browser passes use the same evidence rule.
No migration changed committed reference fixtures to make its output pass.

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
line count is supporting evidence; no percentage deletion target is justified.

## Review validation and limits

At the review baseline (`11c7bf7cd`), the assessment was a source and
test-design review, not a claim that the native, browser, corpus, or
performance suites passed. Cargo metadata was inspected without a build.
`scripts/check-wasm.sh node-unit` passed 105 tests with no skips and reported
`PARTIAL` because it selected one of the then-six steps. No reference fixtures
were regenerated or production code changed for that review. The baseline
`scripts/check.sh` passed all four gates, including both clippy feature
resolutions over 32 workspace members.

The earlier `f22424e2d` receipt covered six combined stages and found the
then-stale local Plain metadata blocked. The subsequent selected-Rust inventory
run at `4e2c1fa21` passed all seven combined stages, including the
libtest-discovered inventory stage (40 ignored host tests in 18 suite rows),
and all four quality gates. These are historical scoped results, not a verdict
for the later frozen performance revision or the final repaired tree. The
local Plain image now has schema-3 metadata for format schema 12; publication
of a default hosted distribution is a separate operation. The later integrated
gate verdict is recorded below at its exact tested revision.

The [frozen performance and corpus comparison](repository_simplification_performance.md)
uses the original production code and integrated `1f32318e0` revision. It
records three cold test-build trials, three paired release runs on each of
three unchanged workloads, profiling allocation/owner counters, warm quality
timing, a repeated-edit probe, and all 210 command-semantic cases. The manual
compatibility selection still exits `FAIL` with 79 matched and 131 other
failures on both sides; the final revision removes two baseline panics but does
not pass the corpus. The snapshot-budget benchmark could not compile on
either frozen side, and its root-byte estimate includes a logical-coordinate
charge. Those are explicit evidence limits, not passing budget checks.

The follow-up repair at `ae6f86fad` updated the snapshot benchmark to the
resident node APIs and changed `SourceMap`'s root-byte estimate to charge live
registered source spans instead of the process-wide logical position.
`scripts/check-snapshot-budgets.sh` then met every existing warmed zero-allocation
and coarse-generation lifecycle assertion. A bounded prefix edit-restart probe
with two warmups, no memo layers, and cold DVI validation reported four
checkpoint roots, one retained generation, no candidate generation or
protected overage, and `checkpoint_root_bytes=2495081` at 6, 12, and 24 edits.
That is a flat charged-root estimate for this workload, not an RSS plateau or
a comparable wall-time measurement; the probes overlapped other builds.

At `78bd325dd`, the documented `cargo bench --manifest-path
benchmarks/tex-state/Cargo.toml --bench state_budgets --no-run` compiled the
Criterion benchmark and its package bins. The resident page-node destination
gate kept zero allocations, moves, and copies at 1 and 4,096 nodes; the PDF
checkpoint gate kept zero capture/restore allocations with exact 1-byte and
64-MiB retained payloads. `scripts/check-and-test.sh` passed all seven stages
with no blocked stage or coverage reduction; `scripts/check.sh` passed all four
quality gates. The script-suite inventory passed with 26 discovered `test-*`
suites. The snapshot budget command remains an explicit extended performance
gate documented in [Testing Infrastructure](testing_infrastructure.md), not a
routine `test-*` script.

The full platform checks ran at `ae6f86fad`: `scripts/check-wasm.sh` passed all
nine selected steps, including Firefox bindings, both WASM-only dense suites,
the schema-12 default Plain format, packaged browser flow, and npm packing.
`scripts/check-pdf-external.sh --ci` passed 17 qpdf structural cases (11
committed, 6 generated) with pinned qpdf 12.3.2 and 15 Poppler render/extraction
attestations with pinned Poppler 25.08.0. Only two benchmark-only source files
differ between that platform-tested revision and `78bd325dd`; production
source is identical. The platform checks do not validate those benchmark
adapters, which the focused benchmark commands above cover.
