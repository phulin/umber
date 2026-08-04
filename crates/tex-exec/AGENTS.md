# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes completed commands from `tex-command` and applies unexpandable TeX semantics to `tex-state::Universe`. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal/vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits execution diagnostics through the state/world boundary.

Use this crate when behavior mutates live engine state or depends on TeX's current mode. Keep assignment scanning thin: decode the primitive operand, create a short-lived `tex_state::ExpansionContext` over the owning `Universe`, scan the value through the shared expansion scanners, then write through the `Universe` facade.

## Boundaries

- Do not read raw input directly. Ordinary execution consumes canonical
  `tex-command` delivery.
- Do not bypass `Universe` or expose raw substores, checkpoint internals, or handle constructors.
- Keep pure list algorithms in `tex-typeset`, immutable font parsing in `tex-fonts`, artifact serialization in `tex-out`, and file/clock/random effects behind `World`.
- Preserve the mode boundary: stomach-side code owns baseline/interline side effects and list contribution, while pure packing/linebreaking routines should stay side-effect free.

## File Map

- `AGENTS.md`: crate-specific guidance and file ownership map for future agents.
- `Cargo.toml`: crate manifest declaring execution-layer dependencies and workspace lints.
- `src/assignments/legacy_arithmetic.rs`: retired expansion-driven arithmetic
  operand scanning plus checked arithmetic helpers for `\advance`, `\multiply`,
  and `\divide`.
- `src/canonical_paragraph_end.rs`: typed source-free canonical paragraph completion and display-interruption transaction.
- `src/canonical_paragraph_end/runtime.rs`: physical paragraph break, line materialization, packing, migration, contribution, diagnostics, and pretolerance-memo kernel.
- `src/canonical_paragraph_end/hyphenation.rs`: source-free paragraph hyphenation, physical discretionary projection, and pattern/exception application.
- `src/canonical_assignments/`: source-free canonical assignment identity,
  primitive registration/naming, admissibility, typed variable writes, and
  tracing.
- `src/canonical_box_runtime/`: typed canonical box-register, material,
  packing/migration, horizontal contribution, spacing, indentation, whatsit,
  and list-commit runtime surface; `mod.rs` owns pending-character flush before
  generic current-list appends; `packaging.rs` physically owns horizontal
  packing/diagnostic projection, box lookup, and `\lastbox` removal/diagnostics;
  `vsplit.rs` owns vertical-box register splitting; `hmode.rs` owns pending-character, ligature, shaping,
  hyphen-boundary, spacing, and italic-correction runtime, while scanner fronts
  remain under assignments; `material.rs` owns post-scan box-register reads,
  unboxing, saved discards, delete-last behavior, migration, contribution, and
  shifts, plus destructive/copy acquisition and box-dimension mutation;
  `leaders.rs` owns payload admissibility, leader-kind/fill conversion,
  register extraction, and final list/page contribution.
- `src/assignments/boxes/`: box-making, `\setbox`, leader payload/glue scanning, `\vsplit`, packing scans, and box list contribution; `mod.rs` holds command-facing handlers while `leaders.rs`, `packaging.rs`, and `vsplit.rs` hold focused helpers.
- `src/assignments/boxes/tests.rs`: focused box-list operation tests, including pdfTeX margin-kern removal and immutable source-list identity during unboxing.
- `src/assignments/boxes/vsplit/tests.rs`: direct TeX82 `\vsplit` void-box and wrong-box recovery tests.
- `src/assignments/fonts.rs`: `\font` scanning and driver-resolved TFM/OpenType selection loading, plus font parameter, hyphenchar, and skewchar assignment behavior.
- `src/assignments/hmode.rs`: horizontal-mode character, glue, kern, discretionary, and ligature handling.
- `src/assignments/hmode/tests.rs`: focused text-accent scanner recovery and traced-token replay tests.
- `src/assignments/hyphenation.rs`: installation of the words `\patterns` and `\hyphenation` scanned (the scan itself is `tex-command`'s §934/§960 scanner), the retired `InputStack` scanner, and paragraph hyphenation.
- `src/assignments/legacy_macros.rs`: retired macro-definition scanner plus
  `\aftergroup` and `\afterassignment` execution fronts.
- `src/assignments/mod.rs`: assignment dispatcher, prefix handling, group commands, and shared scan helpers.
- `src/assignments/paragraph.rs`: retired paragraph scanner, start/reuse adapters, parshape, indentation, and prevdepth logic; break/materialization delegates to typed canonical paragraph results.
- `src/assignments/pdf_fonts.rs`: pdfTeX map, font-attribute, and forced-character action scanning into host-neutral state.
- `src/assignments/pdf_actions.rs`: shared pdfTeX action scanner for catalog, link, and outline consumers.
- `src/assignments/legacy_scan.rs`: retired assignment classification and
  expansion-driven operand scanners for variables and definitions.
- `src/assignments/shipout.rs`: legacy `\shipout` scanner adapter plus shared transaction, commit, publication orchestration, and finalized effect-free artifact reuse; canonical callers enter through `canonical_shipout.rs`.
- `src/assignments/shipout/tests.rs`: direct TeX82 huge-page dimension-boundary, deletion-path, and maximum-legal-page tests.
- `src/canonical_shipout/direct.rs`: fused fresh-page artifact and DVI
  emission over compact state node lists, borrowing only fuel-free expansion
  and output policy from its caller.
- `src/canonical_shipout/direct/tests.rs`: positioned-traversal fast-path classification tests for direct shipout.
- `src/canonical_shipout/direct/normalize.rs`: mutable pre-emission normalization for effects, math substitutions, and direction permutations.
- `src/canonical_shipout/direct/materialize.rs`: localized owned-node replay support for repeated DVI leader payloads.
- `src/canonical_shipout/direct/lower.rs`: state-to-artifact scalar and enum lowering helpers.
- `src/assignments/tokens.rs`: prefix validation, globaldefs policy, optional equals, and token-list assignment helpers.
- `src/canonical_page_output.rs`: input-free page-output selection, `\box255` packaging, insertion distribution, held-over material, page marks, diagnostics, and final `\end` state shared by canonical command control and the compatibility front.
- `src/canonical_main_control.rs`: production command delivery and execution
  driver, including the explicit complete-job versus fragment root-completion
  contract and bounded §360 terminal-input continuation.
- `src/canonical_paragraph_memo.rs`: source-free canonical paragraph dependency and mutation validation, deterministic mutation replay, and compact provenance-recipe construction shared by canonical replay and legacy recording.
- `src/canonical_shipout.rs`: typed source-free canonical page/PDF-form staging transaction, detached shipout origin, and command-owned write/special/literal replay host contracts.
- `src/canonical_shipout/transaction/tests.rs`: focused shipout transaction ownership tests, including live command-context precedence for pre-staging errors.
- `src/canonical_diagnostics.rs`: source-free canonical error reporting, `\show` rendering, activity/page diagnostics, token rendering, and diagnostic sink policy; it has no legacy scanner or executor dependency.
- `src/assignments/legacy_variables.rs`: retired register, parameter, font
  variable, and stream assignment scanner routing.
- `src/assignments/legacy_variables/streams.rs`: retired `\openin`, `\read`,
  `\openout`, `\write`, and stream whatsit execution fronts.
- `src/legacy_assignments.rs`: explicit retired assignment facade used only by
  legacy Executor, dispatch, output, and math fronts; canonical command control
  must not call it.
- `src/checkpoint.rs`: executor-owned named boundary sessions plus opaque aggregate checkpoint restore over `Universe`, live input, and the rooted mode nest.
- `src/align/`: alignment machinery split between canonical and retired fronts. `canonical_execution.rs`, `packaging.rs`, `support.rs`, `transitions.rs`, and `widths/` are source-free completion, unset-node, state-access, lifecycle, and width-resolution owners. `legacy_front.rs` owns the retired aggregate InputStack transaction; `legacy_execution.rs`, `preamble.rs`, `template.rs`, and `noalign.rs` own Executor row/cell execution and scanning. Focused tests remain under the corresponding historical `execution/`, `preamble/`, `widths/`, and `noalign/` test directories.
- `src/dispatch.rs`: source-free canonical dispatch result, execution statistics, and prepared-page contract shared by canonical command control and the compatibility front.
- `src/error.rs`: execution error enum, conversions, and display text. Its
  `Fatal` variant is TeX82 §81's non-local `goto end_of_TEX`: it propagates by
  `?` through every frame exactly as `jump_out` cuts across every active
  procedure level, and only the main-control driver may catch it, which it
  does by latching the session's terminal state instead of returning an
  error. No other handler may recover from it or roll back over it.
- `src/effects/tests.rs`: canonical-replay tests for stream lifecycle, immediate and deferred effects, and shipout-time special/write/open/close behavior.
- `src/fixtures/etex-empty-botmark-fire-up.tex`: bounded e-TeX page-fire-up semantic fixture proving that an empty class-zero bot mark remains present while an empty sparse-class bot mark becomes absent before later enquiries.
- `src/effective_tail.rs`: merged e-TeX blocks 99/253 effective-tail selection and generated `beginM`/`endM` removal planning shared by enquiries and list mutations.
- `src/executor.rs`: retired `Executor` scanner/run loop, concrete legacy execution context, expansion snapshot synchronization, runtime-only session command-fuel ownership across detached operations, and step/replay telemetry.
- `src/host_api.rs`: stable host resource lookup values plus font and PDF-image resolver contracts shared by canonical session adapters and compatibility tests.
- `src/job.rs`: TeX's job framing -- the start-up banner (§61/§536), the `**`
  first line (§534), rendering `tex-command`'s drained §537/§362 `(name`/`)`
  file-bracketing queue, §1335's `final_cleanup` tail (paren close,
  incomplete-conditional and history notes, the `\dump`-outside-INITEX
  note), one-acquisition §360 terminal continuation after root EOF, and
  §1333's `close_files_and_terminate` DVI/transcript report. See
  `docs/job_framing.md`; tests in `src/job/tests.rs`.
- `src/job_output.rs`: TeX82 §§532--536's engine-owned lazy DVI/transcript
  names, `texput` fallback, transcript-open state, and interactive open retry;
  direct lifecycle tests live in `src/job_output/tests.rs`.
- `src/legacy_output.rs`: retired `Executor` input-stack fronts for page fire-up and `\output` token-list replay; it delegates all input-free page state to `canonical_page_output.rs` and has no canonical command-control caller.
- `src/legacy_diagnostics.rs`: retired `Executor` scanners for `\show`, `\showthe`, `\showtokens`, `\showifs`, message, case-change, and ignore-spaces primitives; source-free rendering delegates to `canonical_diagnostics.rs`.
- `src/legacy_dispatch.rs`: retired `InputStack`/`ExecutionContext` main-control token dispatch, group exits, token replay, and unsupported-command routing; source-free result types live in `dispatch.rs`.
- `src/legacy_editor_restart.rs`: isolated retired `InputStack` reconstruction
  used only by the synchronous incremental compatibility path; canonical editor
  sessions restore through command-owned checkpoints.
- `src/lib.rs`: public crate surface and module wiring for the TeX execution engine.
- `src/math/mod.rs`: source-free canonical math owner for font validation plus lowering and display-packaging module wiring.
- `src/math/display.rs`, `src/math/lower.rs`, and `src/math/support.rs`: dependency-clean display construction, mlist lowering, and shared noad/list kernels.
- `src/math/legacy_front.rs` and `src/math/legacy_scan.rs`: retired `Executor` InputStack command dispatch and operand/nested-list scanners; legacy character scanning remains in `src/math/scan/chars.rs`.
- `src/math/tests.rs`: direct TeX82 display-alignment finish, inline/display entry, equation-number, exit, lookahead, and recovery tests.
- `src/math/scan/tests.rs`: focused math scanner coverage for numeric delimiter bounds and traced-token recovery.
- `src/mode.rs`: mode nest, mode summaries, pending horizontal chars, paragraph state, and list metadata; alignment brace depth belongs exclusively to `tex-command`, not this execution-state projection.
- `src/mode/journal.rs`: production generation-checked nested inverse journal behind the typed mode-list mutation boundary and the authoritative canonical aggregate mode rollback path.
- `src/mode/tests.rs`: mode-summary root sharing, restoration, and copy-on-write isolation tests.
- `src/node_dump.rs`: TeX-style node-list dumping used by diagnostic output.
- `src/pack_report.rs`: TeX82 selector-aware overfull and underfull box diagnostics.
- `src/pack_report/tests.rs`: interaction-mode channel and ordering coverage for packed-box diagnostics.
- `src/legacy_paragraph_memo.rs`: retired `Executor` paragraph recording and reuse front, including InputStack transition validation, ExecutionContext caches, accepted-history hlist/line mounts, break-graph observation, barrier classification, and telemetry; source-free validation, replay, and provenance recipes live in `canonical_paragraph_memo.rs`.
- `src/raw_delivery.rs`: single retired lexer bridge for compatibility scanners
  that still require one unexpanded semantic token; canonical execution
  receives command delivery from `tex-command`.
- `src/session_api.rs`: host-neutral execution budgets, checkpoint counters, cancellation, and recoverable-interrupt latches shared by canonical sessions and compatibility tests.
- `src/packing_params.rs`: execution-side snapshots of packing-related integer and dimension parameters before calling pure `tex-typeset` kernels.
- `src/packing_params/tests.rs`: source-backed packing-observation ordering tests, including TeX82's pre-readjustment `vpackage` geometry for `\vtop`.
- `src/page_builder.rs`: TeX.web page-builder accounting, insertion splitting, pending fire-up records, tex.web §§987/1005/1006's `\tracingpages` reporting, and detached page-episode reuse up to the output-routine barrier.
- `src/splitting.rs`: shared vertical split helpers for insertion and `\vsplit` remainder pruning/repacking.
- `src/splitting/tests.rs`: direct TeX82 split-top pruning and adjusted split-skip tests.
- `src/transaction.rs`: lifetime-bound recursive execution transactions that restore mode and Universe roots unless explicitly committed.
- `src/timing.rs`: process-local execution telemetry timer with an inert fallback for WASM hosts that do not provide `std::time::Instant`.
- `src/tests.rs`: crate-internal test harness module and shared imports.
- `src/tests/assignments.rs`: tests for registers, definitions, arithmetic, token assignments, and assignments.
- `src/tests/boxes.rs`: focused TeX82 every-hbox/every-vbox timing, grouping, provenance, format, and rollback tests.
- `src/tests/core.rs`: tests for mode nest behavior, execution context, dispatch, and core errors.
- `src/tests/fonts.rs`: tests for font loading, font parameters, and font-related grouping semantics.
- `src/tests/grouping_parity.rs`: grouping, after-token, magnification, and box-register tests that read committed reference micro fixtures.
- `src/tests/groups.rs`: tests for braces, explicit groups, `\globaldefs`, and aftergroup replay.
- `src/tests/hyphenation.rs`: tests for hyphenation patterns, exceptions, minima, paragraph hyphenation, pure-plan cache keys, malformed misses, and cache-on/off parity.
- `src/tests/io.rs`: tests for input/output streams, reads, writes, immediate effects, and shipout effects.
- `src/tests/math.rs`: tests for math-mode parsing, noad construction, scripts, fractions, choices, families, and mu material.
- `src/tests/support.rs`: shared test helpers for seeded fonts, terminal output, and meaning lookup.
- `tests/it.rs`: external-boundary compile-fail coverage for the public checkpoint API.
- `tests/paragraph_replay.rs`: focused canonical editor-checkpoint paragraph replay regressions.
- `tests/ui/engine_checkpoint_forgery_forbidden.rs`: compile-fail fixture proving callers cannot forge named engine checkpoints.
- `tests/ui/execution_transaction_private.rs`: compile-fail fixture proving live-stack transactions cannot escape as public capabilities.
- `src/vertical.rs`: already-flushed current-list routing, vertical-list
  appends, baseline skip insertion, prevdepth, and list contribution helpers.
- `src/whatsits/tests.rs`: canonical-replay and white-box tests for whatsit construction, ownership, passive list visitation, and language-state boundaries.

## Validation

Run `cargo test --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration tests or corpus fixture checks. Regenerate `tex_exec`/`tex_exec_io` fixtures through `scripts/regen-fixtures.sh`.

Working a canonical-command (`umber2-johp`) semantic or DVI divergence that
touches this crate's main-control dispatch: read
[Canonical Divergence Working Contract](../../docs/canonical_divergence_workflow.md)
first for the oracle hierarchy, diagnosis order, fix discipline, successor-filing
rule, and standing gates.
