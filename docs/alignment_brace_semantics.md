# Alignment and brace semantics

Status: implementation contract for TeX82 alignment token delivery.

## Canonical boundary

The canonical sources are `third_party/reference/tex/tex/tex.web` and the
matching `third_party/texlive-source/src/texk/web2c/pdftexdir/pdftex.web`.
pdfTeX retains TeX82's alignment scanner, macro argument, conditional,
`scan_toks`, `off_save`, and `do_endv` rules; its extensions do not introduce
a second alignment depth.

Umber follows one invariant: the active `tex_lex::InputStack` alignment level
owns the only `align_state`. It is the value produced by raw `get_next`
delivery, not semantic group depth. A physical catcode-1 token adds one and a
physical catcode-2 token subtracts one. Control-sequence aliases such as
`\bgroup` and `\egroup` can open or close execution groups but do not affect
`align_state`. A top-level tab, `\span`, or `\cr` starts the v-template exactly
when the value is zero.

The sentinel values have their source meanings:

- `-1000000` scans an alignment preamble;
- `1000000` disables cell termination during row lookahead, u-template replay,
  and v-template replay;
- `0` is the cell-body base depth.

`tex_exec::mode::AlignState` owns row, column, span, and packaging progress. It
deliberately has no brace-depth shadow. Nested alignments suspend and restore
the complete outer `InputStack` alignment level, matching `push_alignment` and
`pop_alignment`.

## Canonical transition map

| Canonical operation                          | State transition                                                                                 | Umber owner                                                                                                |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `init_align` / `push_alignment`              | save outer level; install `-1000000`                                                             | `tex_exec::align::execute_alignment`, `InputStack::suspend_alignment_cell`, `begin_alignment`              |
| preamble restart                             | assign `-1000000`                                                                                | `tex_exec::align::preamble`, `AlignmentScannerPhase::Preamble`                                             |
| physical or replayed brace in `get_next`     | `{`: `+1`; `}`: `-1`                                                                             | `tex_expand::classify_alignment_token`, `InputStack::intercept_alignment_token`                            |
| control-sequence brace alias                 | no depth change                                                                                  | `tex_expand::classify_alignment_token`                                                                     |
| exhausted u-template                         | if state is above 500000, assign `0`; otherwise interwoven-preamble failure                      | `InputStack::intercept_alignment_token` and its exact replay marker                                        |
| top-level tab, `\span`, or `\cr`             | retain terminator, push v-template, assign `1000000`                                             | `InputStack::intercept_alignment_token`                                                                    |
| `\omit` cell                                 | begin directly at `0`                                                                            | `InputStack::begin_alignment_cell`                                                                         |
| `align_peek` and `fin_col` restart           | assign `1000000` before lookahead/u-template                                                     | `tex_exec::align::execution`, `AlignmentScannerPhase::BetweenEntries`                                      |
| `back_input` / `back_error`                  | undo the delivered brace adjustment before replay                                                | `tex_expand::back_input`, `InputStack::undo_alignment_delivery`                                            |
| macro delimiter ending in `#{`               | cancel the duplicate opening-brace adjustment                                                    | `tex_expand::args`                                                                                         |
| macro extra `}` recovery                     | undo backup accounting, then let inserted recovery input account normally                        | `tex_expand::args`, `InputStack::back_input_alignment_token`                                               |
| aborted macro argument with unmatched groups | remove its accumulated imbalance                                                                 | `tex_expand::args` argument-level correction                                                               |
| alphabetic constant `` `} `` / `` `{ ``      | cancel `get_token` brace accounting                                                              | `tex_expand::scan_int`                                                                                     |
| compulsory-brace recovery                    | inserted `{` contributes one even when synthesized outside raw delivery                          | `tex_exec::assignments::scanning`, `tex_expand::scan`, `InputStack::account_inserted_alignment_left_brace` |
| ordinary `scan_toks`                         | real opening and closing braces account through raw delivery                                     | `tex_expand::scan`                                                                                         |
| expanded `scan_toks` detached replay         | add one synthetic opening-brace level, remove only that level, retain expansion-produced deltas  | `tex_expand::scan::{scan_toks_expanded_with_driver,expand_replacement_text}`                               |
| `pass_text` conditional skipping             | discarded tokens still pass through `get_next`; their braces change depth                        | `tex_expand::conditionals::skip_until`, `next_semantic_raw_token`                                          |
| `\ifx` operands                              | unexpanded operands still use `get_next` brace accounting                                        | `tex_expand::conditionals::scan_ifx_operand`                                                               |
| nested alignment / `pop_alignment`           | restore the exact outer alignment level                                                          | `InputStack::{suspend_alignment_cell,resume_alignment_cell}`                                               |
| `do_endv`                                    | accept only an exhausted active v-template (or its exact driver-carried marker)                  | `tex_exec::align::execution`, `InputStack::has_exhausted_alignment_v_template`                             |
| successful `do_endv`                         | retire the exact v-template frame before an alias can escape                                     | `InputStack::{finish_alignment_cell,retire_alignment_v_template}`                                          |
| `off_save` above bottom level                | back up end-v/delimiter with accounting undone; insert the closer for the actual execution group | `tex_exec::assignments::off_save_alignment`                                                                |
| `off_save` at bottom level                   | drop the offending command; do not replay it                                                     | `tex_exec::assignments::off_save_alignment`                                                                |
| `align_error` at depth `-1` or `1`           | back up delimiter, insert the missing brace, and return to zero through ordinary delivery        | `tex_exec::align::execution::run_cell_body_until_terminator`                                               |
| alignment closing brace before `\cr`         | back up brace and insert frozen `\cr`                                                            | `tex_exec::align::execution`                                                                               |
| `fin_align` / `pop_alignment`                | retire inner level and resume saved outer level                                                  | `tex_exec::align`, `InputStack::{finish_alignment,resume_alignment_cell}`                                  |
| failed nested alignment                      | discard the incomplete inner level and restore the outer level atomically                        | `InputStack::abort_alignment_and_resume`, execution transaction rollback                                   |

## Regression ownership

Focused regressions live at the owning boundary:

- aliases and `do_endv` recovery: `crates/tex-exec/src/tests/align.rs`;
- raw brace accounting, nested suspension, and v-template retirement:
  `crates/tex-lex/src/tests.rs`;
- skipped braces and `\ifx`: `crates/tex-expand/src/tests.rs`;
- macro arguments and expanded definitions:
  `crates/tex-expand/src/args_tests.rs` and `src/scan/tests.rs`;
- integer-scanner corrections: `crates/tex-expand/src/scan_int/tests.rs`.

The fixture-only native correctness gate remains `cargo test --tests`; format
and clippy remain `scripts/check.sh`.
