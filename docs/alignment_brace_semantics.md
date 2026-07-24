# Alignment and brace semantics

Status: implementation contract for TeX82 alignment token delivery.

## Canonical boundary

The canonical sources are `third_party/reference/tex/tex/tex.web` and the
matching `third_party/texlive-source/src/texk/web2c/pdftexdir/pdftex.web`.
pdfTeX retains TeX82's alignment scanner, macro argument, conditional,
`scan_toks`, `off_save`, and `do_endv` rules; its extensions do not introduce
a second alignment depth.

Umber follows one invariant: `tex_command::AlignmentDeliveryState` owns the
only `align_state`, beside the one raw `get_next` loop. It is the value
produced by raw delivery, not semantic group depth. The executor may request
alignment lifecycle transitions but cannot classify an already delivered token.
A physical catcode-1 token adds one and a
physical catcode-2 token subtracts one. Control-sequence aliases such as
`\bgroup` and `\egroup` can open or close execution groups but do not affect
`align_state`. A top-level tab, `\span`, or `\cr` starts the v-template exactly
when the value is zero.

### Delimiter and template lifecycle

TeX82's `get_next` recognizes an active-cell terminator at `align_state=0`
before main control receives an ordinary command. It then takes the
v-template path. This is the lifecycle specified by `get_next`, `init_col`,
v-template insertion, `fin_col`, and `do_endv` in `tex.web` §§343 and
765--772. Consequently, when a command-owned scanner backs up a non-keyword
lookahead (for example after completing a `\vrule` specification), replay
must resume via `get_x_alignment_delivery`, not generic expanded delivery.
That typed path reports the original delimiter at state 0, backs up the exact
delivery, pushes the selected v-template, and then enters the `1000000`
sentinel. The intercepted delimiter is not separately observed as an ordinary
raw command; it resumes only after `do_endv` retires the exact v-template
frame. At that retained boundary, `get_next` first delivers frozen
`end_template`; TeX82 §343 expands it to frozen `endv`, and only then does
§772's `do_endv` retire the frame. Retention itself is not a retirement
observation. Both inaccessible frozen control sequences retain the canonical
`endtemplate` control-sequence identity; expansion changes only the effective
command from `end_template` to `endv` (TeX82 §§343, 765).

The sentinel values have their source meanings:

- `-1000000` scans an alignment preamble;
- `1000000` disables cell termination during row lookahead, u-template replay,
  and v-template replay;
- `0` is the cell-body base depth.

`tex_exec::mode::AlignState` owns row, column, span, and packaging progress. It
deliberately has no brace-depth shadow. Nested alignments suspend and restore
the complete outer `InputStack` alignment level, matching `push_alignment` and
`pop_alignment`.

Instrumentation carries a brace delivery's pre-change and committed
`align_state` directly from `tex-command`. The host-only command-stream
translator maps those begin/end-group records to canonical `state_change`
events; it neither reconstructs nor owns alignment depth.

## Canonical transition map

| Canonical operation                          | State transition                                                                                                                                                                                                                                                                     | Umber owner                                                                                                                |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `init_align` / `push_alignment`              | save outer delivery state; install `-1000000`                                                                                                                                                                                                                                        | `tex_exec` request, `CommandState::begin_alignment` / `suspend_alignment`                                                  |
| preamble opening brace                       | validate it through expanded command input, then replay it through two backups; after the final brace delivery, enter `aligning`, restore `-1000000`, and retire that backup through raw preamble delivery                                                                           | `CommandProcessor::{scan_alignment_preamble_opening,replay_alignment_preamble_opening,begin_alignment_preamble_scan}`      |
| preamble restart                             | assign `-1000000`                                                                                                                                                                                                                                                                    | `tex_exec::align::preamble`, `AlignmentScannerPhase::Preamble`                                                             |
| raw preamble collection                      | retain raw delivery through `#`, `&`, and the terminating `\cr`; freeze u/v pairs, publish preamble completion, then restore the live `aligning` scanner status to normal                                                                                                            | `tex_command::CommandProcessor::begin_alignment_preamble_scan`                                                             |
| first cell after preamble completion         | after the completion and `aligning`-to-normal observations, transfer the frozen columns once and request the executor-selected first cell; command processing delivers and backs up the first source opener before the typed template-install request pushes the optional u-template | `CommandProcessor::scan_alignment_cell_opening`, `AlignmentRequest::InstallCellTemplate`, `tex_exec::CommandReplayControl` |
| u-template lifecycle observation             | after the command-owned opener backup correction, publish the exact `u_template` input push and template-push event; on exhaustion, publish its retirement before restoring the cell-body sentinel from `1000000` to `0`                                                             | `CommandState::install_alignment_cell_template`, `CommandProcessor::retire_and_restart`, `tex_exec::CommandReplayControl`  |
| physical or replayed brace in `get_next`     | `{`: `+1`; `}`: `-1`                                                                                                                                                                                                                                                                 | `tex_command::CommandProcessor::get_next`                                                                                  |
| control-sequence brace alias                 | no depth change                                                                                                                                                                                                                                                                      | `tex_command::CommandProcessor::get_next`                                                                                  |
| exhausted u-template                         | if state is above 500000, assign `0`; otherwise interwoven-preamble failure                                                                                                                                                                                                          | `InputStack::intercept_alignment_token` and its exact replay marker                                                        |
| top-level tab, `\span`, or `\cr`             | convert to frozen end-template delivery and assign `1000000`                                                                                                                                                                                                                         | `tex_command::CommandProcessor::get_next`                                                                                  |
| completed cell-body scanner                  | resume through typed expanded alignment delivery, so a backed-up delimiter remains an opaque end-template event                                                                                                                                                                      | `tex_exec::CommandReplayControl`, `CommandProcessor::get_x_alignment_delivery`                                             |
| `\omit` cell                                 | begin directly at `0`                                                                                                                                                                                                                                                                | `InputStack::begin_alignment_cell`                                                                                         |
| `align_peek` and `fin_col` restart           | assign `1000000` before lookahead/u-template                                                                                                                                                                                                                                         | `tex_exec::align::execution`, `AlignmentScannerPhase::BetweenEntries`                                                      |
| `back_input` / `back_error`                  | undo the recorded delivery adjustment before replay and publish the correction after the backup lifecycle transition                                                                                                                                                                 | `tex_command::CommandProcessor::back_input`                                                                                |
| macro delimiter ending in `#{`               | cancel the duplicate opening-brace adjustment                                                                                                                                                                                                                                        | `tex_expand::args`                                                                                                         |
| macro extra `}` recovery                     | undo backup accounting, then let inserted recovery input account normally                                                                                                                                                                                                            | `tex_expand::args`, `InputStack::back_input_alignment_token`                                                               |
| aborted macro argument with unmatched groups | remove its accumulated imbalance                                                                                                                                                                                                                                                     | `tex_expand::args` argument-level correction                                                                               |
| alphabetic constant `` `} `` / `` `{ ``      | cancel `get_token` brace accounting                                                                                                                                                                                                                                                  | `tex_expand::scan_int`                                                                                                     |
| compulsory-brace recovery                    | inserted `{` contributes one even when synthesized outside raw delivery                                                                                                                                                                                                              | `tex_exec::assignments::scanning`, `tex_expand::scan`, `InputStack::account_inserted_alignment_left_brace`                 |
| ordinary `scan_toks`                         | real opening and closing braces account through raw delivery                                                                                                                                                                                                                         | `tex_expand::scan`                                                                                                         |
| expanded `scan_toks` detached replay         | add one synthetic opening-brace level, remove only that level, retain expansion-produced deltas                                                                                                                                                                                      | `tex_expand::scan::{scan_toks_expanded_with_driver,expand_replacement_text}`                                               |
| box-body closer after alignment work         | package only on the delivered closer for the active box save-stack group; alignment depth remains token-delivery state                                                                                                                                                               | `tex_exec::assignments::boxes::packaging::scan_box_group`, `InputStack`                                                    |
| `pass_text` conditional skipping             | discarded tokens still pass through `get_next`; their braces change depth                                                                                                                                                                                                            | `tex_expand::conditionals::skip_until`, `next_semantic_raw_token`                                                          |
| `\ifx` operands                              | unexpanded operands still use `get_next` brace accounting                                                                                                                                                                                                                            | `tex_expand::conditionals::scan_ifx_operand`                                                                               |
| nested alignment / `pop_alignment`           | restore the exact outer state, active identity, and cell templates                                                                                                                                                                                                                   | `CommandState::{suspend_alignment,resume_alignment}`                                                                       |
| `do_endv`                                    | accept only an exhausted active v-template (or its exact driver-carried marker)                                                                                                                                                                                                      | `tex_exec::align::execution`, `InputStack::has_exhausted_alignment_v_template`                                             |
| successful `do_endv`                         | retire the exact v-template frame before an alias can escape                                                                                                                                                                                                                         | `InputStack::{finish_alignment_cell,retire_alignment_v_template}`                                                          |
| `off_save` above bottom level                | back up end-v/delimiter with accounting undone; insert the closer for the actual execution group                                                                                                                                                                                     | `tex_exec::assignments::off_save_alignment`                                                                                |
| `off_save` at bottom level                   | drop the offending command; do not replay it                                                                                                                                                                                                                                         | `tex_exec::assignments::off_save_alignment`                                                                                |
| `align_error` at depth `-1` or `1`           | back up delimiter, insert the missing brace, and return to zero through ordinary delivery                                                                                                                                                                                            | `tex_exec::align::execution::run_cell_body_until_terminator`                                                               |
| alignment closing brace before `\cr`         | back up brace and insert frozen `\cr`                                                                                                                                                                                                                                                | `tex_exec::align::execution`                                                                                               |
| `fin_align` / `pop_alignment`                | retire inner level and resume saved outer level                                                                                                                                                                                                                                      | `tex_exec::align`, `InputStack::{finish_alignment,resume_alignment_cell}`                                                  |
| failed nested alignment                      | discard the incomplete inner level and restore the outer level atomically                                                                                                                                                                                                            | `InputStack::abort_alignment_and_resume`, execution transaction rollback                                                   |

## Regression ownership

Focused regressions live at the owning boundary:

- aliases and `do_endv` recovery: `crates/tex-exec/src/tests/align.rs`;
- raw brace accounting, nested suspension, and v-template retirement:
  `crates/tex-lex/src/tests.rs`;
- skipped braces and `\ifx`: `crates/tex-expand/src/tests.rs`;
- macro arguments and expanded definitions:
  `crates/tex-expand/src/args_tests.rs` and `src/scan/tests.rs`;
- integer-scanner corrections: `crates/tex-expand/src/scan_int/tests.rs`.
- canonical delivery/lifecycle observation:
  `tests/tex82-oracle/instrumentation.ch` and
  `tests/tex82-oracle/transitions.tex`; the detached schema records semantic
  nesting and transition classes without pointer or input-stack identities.
- replay observer ordering for the first u-template opener and retirement:
  `crates/tex-exec/src/command_replay/tests.rs`.

The fixture-only native correctness gate remains `cargo test --tests`; format
and clippy remain `scripts/check.sh`.
