# `show_context` Fallback Closure Audit

Issue: `umber2-alfh.14`

Authority: TeX82 §§82, 310--318, 641, 825, 976, 993, 1004, 1009, 1015,
1110, and 1257.

TeX82 §82 calls `show_context` synchronously from `error`. Therefore a
command-time report must use the command cursor captured at its scanner or
dispatch boundary. A detached continuation may use an owned context captured
before that borrow ended. Only a continuation with neither may render the
last input summary published in `Universe`.

The production `input_summary()` audit has these dispositions:

| Owner                                                           | Fallback disposition                                                                                                                                                             |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `diagnostics::report_page_infinite_shrinkage` (§1004)           | Every production page-builder entry now receives the triggering command's live context. The summary branch is exercised only by the source-free page-builder test seam.          |
| `diagnostics::report_split_infinite_shrinkage` (§976)           | `\vsplit` receives its scanner-captured live context; insertion splitting receives its page-builder context. Only the same source-free page-builder seam can select the summary. |
| `diagnostics::report_insertion_skip_infinite_shrinkage` (§1009) | Receives the live page-builder context in production; the summary is reserved for source-free page-builder tests.                                                                |
| `page_builder::ensure_insertion_vbox` (§993)                    | Receives live context both while accepting an insertion and while preparing output distribution. The summary is reserved for source-free page/output tests.                      |
| `page_output::report_box255_not_void` (§1015)                   | `select_pending_page_output` supplies the command context captured immediately before output selection. The summary branch is the explicit input-free page-output test seam.     |
| `shipout::transaction::shipout_error_context` (§641)            | Prefers the command-owned context captured in `ShipoutOrigin`; if absent, it renders the detached input summary that travelled with the page, never an unrelated current cursor. |
| `shipout::direct` deferred output-open context                  | Prefers the context captured when the shipout began; its fallback renders the detached page input summary because direct normalization owns no command cursor.                   |
| `shipout::transaction::report_invalid_pdf_version`              | pdfTeX fixes the version on first-page output, after the assigning command has ended. This input-free continuation intentionally uses the last published summary.                |

Three command-time summary uses were not canonical and are removed:

- TeX82 §1257 font-capacity recovery now receives the font scanner's owned
  context.
- TeX82 §976 `\vsplit` infinite-shrink recovery now receives the completed
  split scanner's owned context.
- TeX82 §825 paragraph recovery and every synchronous §994 page-builder
  entry now receive the current command context, including alignment,
  display, leader, box, penalty, and backed-up end-job paths.

`box_runtime::report_incompatible_unbox` (§1110) had an unreachable summary
fallback: its sole production entry already owns the completed register
scan's context. Its type now requires that context.

Focused evidence covers live mid-scan context, backed-up `\vsplit` context,
terminal exhaustion, TeX82 pseudoprint cropping, paragraph/output-routine
context, detached shipout context, and the source-free page-builder and
pre-output `\box255` summary classes. No command-semantic manifest cites this
issue.
