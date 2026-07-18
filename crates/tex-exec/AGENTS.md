# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes fully expanded tokens from `tex-expand` and applies unexpandable TeX semantics to `tex-state::Universe`. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal/vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits execution diagnostics through the state/world boundary.

Use this crate when behavior mutates live engine state or depends on TeX's current mode. Keep assignment scanning thin: decode the primitive operand, create a short-lived `tex_state::ExpansionContext` over the owning `Universe`, scan the value through the shared expansion scanners, then write through the `Universe` facade.

## Boundaries

- Do not read raw input directly here except through the gullet interfaces required by TeX semantics; ordinary execution should pull expanded tokens from `tex-expand`.
- Do not bypass `Universe` or expose raw substores, checkpoint internals, or handle constructors.
- Keep pure list algorithms in `tex-typeset`, immutable font parsing in `tex-fonts`, artifact serialization in `tex-out`, and file/clock/random effects behind `World`.
- Preserve the mode boundary: stomach-side code owns baseline/interline side effects and list contribution, while pure packing/linebreaking routines should stay side-effect free.

## File Map

- `AGENTS.md`: crate-specific guidance and file ownership map for future agents.
- `Cargo.toml`: crate manifest declaring execution-layer dependencies and workspace lints.
- `src/assignments/arithmetic.rs`: checked arithmetic helpers for `\advance`, `\multiply`, and `\divide`.
- `src/assignments/admissibility.rs`: authoritative assignment-family and math-mode pass-through metadata.
- `src/assignments/boxes/`: box-making, `\setbox`, leader payload/glue scanning, `\vsplit`, packing scans, and box list contribution; `mod.rs` holds command-facing handlers while `leaders.rs`, `packaging.rs`, and `vsplit.rs` hold focused helpers.
- `src/assignments/fonts.rs`: `\font` scanning and driver-resolved TFM/OpenType selection loading, plus font parameter, hyphenchar, and skewchar assignment behavior.
- `src/assignments/hmode.rs`: horizontal-mode character, glue, kern, discretionary, and ligature handling.
- `src/assignments/hmode/tests.rs`: focused text-accent scanner recovery and traced-token replay tests.
- `src/assignments/hyphenation.rs`: `\patterns`, `\hyphenation`, and `\showhyphens` execution support.
- `src/assignments/macros.rs`: macro-definition primitives plus `\aftergroup` and `\afterassignment`.
- `src/assignments/mod.rs`: assignment dispatcher, prefix handling, group commands, and shared scan helpers.
- `src/assignments/paragraph.rs`: paragraph start/end, parshape, line breaking, indentation, prevdepth logic, and the optional detached pretolerance-plan experiment.
- `src/assignments/pdf_fonts.rs`: pdfTeX map, font-attribute, and forced-character action scanning into host-neutral state.
- `src/assignments/pdf_actions.rs`: shared pdfTeX action scanner for catalog, link, and outline consumers.
- `src/assignments/primitives.rs`: registration table for unexpandable primitive meanings.
- `src/assignments/scanning.rs`: assignment classification and operand scanners for variables and definitions.
- `src/assignments/shipout.rs`: `\shipout` transaction, commit, publication orchestration, and finalized effect-free artifact reuse; deferred and host-effect paths remain explicit barriers.
- `src/assignments/shipout/direct.rs`: fused fresh-page artifact and DVI emission over compact state node lists.
- `src/assignments/shipout/direct/tests.rs`: positioned-traversal fast-path classification tests for direct shipout.
- `src/assignments/shipout/direct/normalize.rs`: mutable pre-emission normalization for effects, math substitutions, and direction permutations.
- `src/assignments/shipout/direct/materialize.rs`: localized owned-node replay support for repeated DVI leader payloads.
- `src/assignments/shipout/direct/lower.rs`: state-to-artifact scalar and enum lowering helpers.
- `src/assignments/tokens.rs`: prefix validation, globaldefs policy, optional equals, and token-list assignment helpers.
- `src/assignments/variables.rs`: register, parameter, font variable, and stream assignment routing.
- `src/assignments/variables/streams.rs`: `\openin`, `\read`, `\openout`, `\write`, and stream whatsit execution.
- `src/assignments/variables/variable_access.rs`: typed read/write accessors for registers, parameters, and font variables.
- `src/checkpoint.rs`: executor-owned named boundary sessions plus opaque aggregate checkpoint restore over `Universe`, live input, and the rooted mode nest.
- `src/align/`: alignment stomach sub-mode machinery; `preamble.rs` scans `\halign`/`\valign` preambles into frozen u/v templates, tabskip boundaries, and repeat metadata; `execution.rs` runs the row/cell sub-mode and appends rows with TeX's live prev-depth spacing; `template.rs` replays u/v templates and owns the span-time expansion interleave; `noalign.rs` scans `\noalign` groups on that same alignment list so vertical state changes reach the next row; `packaging.rs` builds unset row/cell records; `support.rs` holds alignment state accessors and token classifiers; `widths.rs` orchestrates `fin_align`, with `widths/resolution.rs` resolving span-aware column widths, `widths/set.rs` converting unset rows/cells to ordinary set boxes, and `widths/debug.rs` checking that unset nodes do not escape.
- `src/diagnostics.rs`: diagnostic primitives such as `\show`, `\showthe`, `\showbox`, and message writing.
- `src/dispatch.rs`: main-control token dispatch, group exits, token replay, and execution statistics.
- `src/error.rs`: execution error enum, conversions, and display text.
- `src/executor.rs`: `Executor` run loop, concrete execution context, localized font resolver and atomic `FontSource` handoff, and expansion snapshot synchronization.
- `src/lib.rs`: public crate surface and module wiring for the TeX execution engine.
- `src/math/`: math-mode stomach front-end that builds frozen mlists, noads, fractions, choices, styles, and mu nodes; split into dispatch, display packaging, lowering, scanner, and support modules.
- `src/math/scan/tests.rs`: focused math scanner coverage for numeric delimiter bounds and traced-token recovery.
- `src/mode.rs`: mode nest, mode summaries, pending horizontal chars, paragraph state, and list metadata.
- `src/mode/tests.rs`: mode-summary root sharing, restoration, and copy-on-write isolation tests.
- `src/node_dump.rs`: TeX-style node-list dumping used by diagnostic output.
- `src/output.rs`: output-routine fire-up, `\box255` packaging, held-over material, deadcycle handling, and final `\end` page cleanup.
- `src/paragraph_memo.rs`: centralized fail-before-mutation accepted-history paragraph validation, exact-stamp and typed semantic dependency tiers, accepted-history-owned hlist/finished-line mounts, ordered count/integer-parameter redo, full source-transition checks, output-reachable provenance recipes and mounted rebinding, barrier classification, and telemetry.
- `src/packing_params.rs`: execution-side snapshots of packing-related integer and dimension parameters before calling pure `tex-typeset` kernels.
- `src/page_builder.rs`: TeX.web page-builder accounting, insertion splitting, pending fire-up records, and detached page-episode reuse up to the output-routine barrier.
- `src/splitting.rs`: shared vertical split helpers for insertion and `\vsplit` remainder pruning/repacking.
- `src/transaction.rs`: lifetime-bound recursive execution transactions that restore mode and Universe roots unless explicitly committed.
- `src/tests.rs`: crate-internal test harness module and shared imports.
- `src/tests/assignments.rs`: tests for registers, definitions, arithmetic, token assignments, and assignments.
- `src/tests/core.rs`: tests for mode nest behavior, execution context, dispatch, and core errors.
- `src/tests/fonts.rs`: tests for font loading, font parameters, and font-related grouping semantics.
- `src/tests/grouping_parity.rs`: grouping, after-token, magnification, and box-register tests that read committed reference micro fixtures.
- `src/tests/groups.rs`: tests for braces, explicit groups, `\globaldefs`, and aftergroup replay.
- `src/tests/hyphenation.rs`: tests for hyphenation patterns, exceptions, minima, paragraph hyphenation, pure-plan cache keys, malformed misses, and cache-on/off parity.
- `src/tests/io.rs`: tests for input/output streams, reads, writes, immediate effects, and shipout effects.
- `src/tests/math.rs`: tests for math-mode parsing, noad construction, scripts, fractions, choices, families, and mu material.
- `src/tests/support.rs`: shared test helpers for seeded fonts, terminal output, and meaning lookup.
- `tests/it.rs`: external-boundary compile-fail coverage for the public checkpoint API.
- `tests/ui/engine_checkpoint_forgery_forbidden.rs`: compile-fail fixture proving callers cannot forge named engine checkpoints.
- `tests/ui/execution_transaction_private.rs`: compile-fail fixture proving live-stack transactions cannot escape as public capabilities.
- `src/vertical.rs`: vertical-list appends, baseline skip insertion, prevdepth, and list contribution helpers.

## Validation

Run `cargo test --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration tests or corpus fixture checks. Regenerate `tex_exec`/`tex_exec_io` fixtures through `scripts/regen-fixtures.sh`.
