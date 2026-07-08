# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes fully expanded tokens from `tex-expand` and applies unexpandable TeX semantics to `tex-state::Universe`. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal/vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits execution diagnostics through the state/world boundary.

Use this crate when behavior mutates live engine state or depends on TeX's current mode. Keep assignment scanning thin: decode the primitive operand, scan the value through the shared expansion scanners, then write through the `Universe` facade.

## Boundaries

- Do not read raw input directly here except through the gullet interfaces required by TeX semantics; ordinary execution should pull expanded tokens from `tex-expand`.
- Do not bypass `Universe` or expose raw substores, checkpoint internals, or handle constructors.
- Keep pure list algorithms in `tex-typeset`, immutable font parsing in `tex-fonts`, artifact serialization in `tex-out`, and file/clock/random effects behind `World`.
- Preserve the mode boundary: stomach-side code owns baseline/interline side effects and list contribution, while pure packing/linebreaking routines should stay side-effect free.

## File Map

- `AGENTS.md`: crate-specific guidance and file ownership map for future agents.
- `Cargo.toml`: crate manifest declaring execution-layer dependencies and workspace lints.
- `src/assignments/arithmetic.rs`: checked arithmetic helpers for `\advance`, `\multiply`, and `\divide`.
- `src/assignments/boxes.rs`: box-making, `\setbox`, packing scans, and box list contribution.
- `src/assignments/fonts.rs`: `\font`, font parameter, hyphenchar, and skewchar assignment behavior.
- `src/assignments/hmode.rs`: horizontal-mode character, glue, kern, discretionary, and ligature handling.
- `src/assignments/hyphenation.rs`: `\patterns`, `\hyphenation`, and `\showhyphens` execution support.
- `src/assignments/macros.rs`: macro-definition primitives plus `\aftergroup` and `\afterassignment`.
- `src/assignments/mod.rs`: assignment dispatcher, prefix handling, group commands, and shared scan helpers.
- `src/assignments/paragraph.rs`: paragraph start/end, parshape, line breaking, indentation, and prevdepth logic.
- `src/assignments/primitives.rs`: registration table for unexpandable primitive meanings.
- `src/assignments/scanning.rs`: assignment classification and operand scanners for variables and definitions.
- `src/assignments/shipout.rs`: `\shipout` lowering from state nodes into `tex-out` page artifacts.
- `src/assignments/tokens.rs`: prefix validation, globaldefs policy, optional equals, and token-list assignment helpers.
- `src/assignments/variables.rs`: register, parameter, font variable, and stream assignment routing.
- `src/assignments/variables/streams.rs`: `\openin`, `\read`, `\openout`, `\write`, and stream whatsit execution.
- `src/assignments/variables/variable_access.rs`: typed read/write accessors for registers, parameters, and font variables.
- `src/diagnostics.rs`: diagnostic primitives such as `\show`, `\showthe`, `\showbox`, and message writing.
- `src/dispatch.rs`: main-control token dispatch, group exits, token replay, and execution statistics.
- `src/error.rs`: execution error enum, conversions, and display text.
- `src/executor.rs`: `Executor` run loop and expansion-hook integration for engine mode and recovery.
- `src/lib.rs`: public crate surface and module wiring for the TeX execution engine.
- `src/mode.rs`: mode nest, mode summaries, pending horizontal chars, paragraph state, and list metadata.
- `src/node_dump.rs`: TeX-style node-list dumping used by diagnostic output.
- `src/tests.rs`: crate-internal test harness module and shared imports.
- `src/tests/assignments.rs`: tests for registers, definitions, arithmetic, token assignments, and assignments.
- `src/tests/core.rs`: tests for mode nest behavior, executor hooks, dispatch, and core errors.
- `src/tests/fonts.rs`: tests for font loading, font parameters, and font-related grouping semantics.
- `src/tests/grouping_parity.rs`: pdfTeX parity tests for grouping and after-token behavior.
- `src/tests/groups.rs`: tests for braces, explicit groups, `\globaldefs`, and aftergroup replay.
- `src/tests/hyphenation.rs`: tests for hyphenation patterns, exceptions, minima, and paragraph hyphenation.
- `src/tests/io.rs`: tests for input/output streams, reads, writes, immediate effects, and shipout effects.
- `src/tests/support.rs`: shared test helpers for seeded fonts, terminal output, and meaning lookup.
- `src/vertical.rs`: vertical-list appends, baseline skip insertion, prevdepth, and list contribution helpers.

## Validation

Run `cargo test --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration tests or corpus fixture checks.
