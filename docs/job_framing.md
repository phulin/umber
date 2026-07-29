# Job Framing

TeX's terminal and transcript are not raw diagnostic streams. They are the two
halves of a **job**: a run that announces itself with a banner, echoes the
command line it was given, brackets every file it reads in parentheses, and
signs off with a page count and the transcript's name. tex.web spreads that
framing across §534's `open_log_file`, §537's `start_input`, §362's file
exhaustion, §642's `finish_dvi`, and §1333's `close_files_and_terminate`.

Umber emitted none of it. A `\showbox` run's transcript began at the `>␣\box0=`
line and ended there, while the same source through pdfTeX 1.40.27 produced:

```
This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) (INITEX)  9 JUL 2026 13:36
**show-box.tex
(./show-box.tex
> \box0=
\hbox(0.0+0.0)x1.0 []


! OK.
l.3 \showbox0

 )
No pages of output.
```

That is not a formatting difference. The two channels are different objects,
and no normalization makes one the evidence for the other, which is why
`umber2-alfh.1` -- validating every committed minifixture channel against the
pinned oracle -- was blocked behind this document's subject.

## What a job prints, and when

| tex.web               | Text                                                   | Sink                     |
| --------------------- | ------------------------------------------------------ | ------------------------ |
| §61 `wterm(banner)`   | `This is …␣(INITEX)`, then a newline                   | terminal, before the log |
| §536                  | the same banner plus `format_ident` and the clock      | log                      |
| etex.ch §536/§1337    | `entering extended mode`                               | both                     |
| §534                  | `**` and the job's first line, then a newline          | log                      |
| §537 `start_input`    | `(` and the opened file's name                         | both                     |
| §362                  | `)` when a file's last line is consumed                | both                     |
| §1335 `final_cleanup` | `␣)` once per still-open file                          | both                     |
| §642 `finish_dvi`     | `No pages of output.` / `Output written on …`          | both                     |
| §1335                 | `(see the transcript file for additional information)` | terminal only            |
| §1333                 | `Transcript written on ␣<jobname>.log.`                | terminal only            |

Three of those lines are conditional:

- etex.ch's "entering extended mode" line prints only when the job's command
  profile is `CommandProfile::ETEX26`; `begin_job` takes this as an explicit
  `etex: bool` rather than inferring it from `initex`, which means something
  different (tex.web's `init`/`tini` split, not the e-TeX extension set).
- §1335's "see the transcript file" note appears only when `history` is worse
  than `spotless` _and_ the job is not in `errorstopmode`. tex.web §76's
  `history` is a four-valued job-outcome high-water mark, raised by §82's
  `error` and §245's `begin_diagnostic`. `crate::print`'s module documentation
  recorded its absence as a known gap; this work closes it.
- §1333's transcript line is printed only when closing the log leaves the
  selector at `term_only`, i.e. only when the job was writing both channels.

## Where each piece lives

Printing is `tex-state`'s (`Printer` over §54's `selector`), file opening is
`tex-command`'s (§537 is an input-stack operation), and the job lifecycle is
`tex-exec`'s (`CanonicalMainControl` is what starts a job and what reaches
§1332). No one layer sees all three, so the framing is assembled rather than
placed:

- **`tex-state`** gains §76's `history` on `ErrorChannel`, beside the
  `error_count` §82 already maintains there.
- **`tex-command`** gains a _name_ on an opened source (§537's
  `a_make_name_string`, which Umber's anonymous byte registrations had no
  place for) and a drained queue of file-framing events. It still prints
  nothing: the queue records that a named file opened or that one retired, in
  order, and the engine renders it.

  The queue lives on `CommandState` rather than on the short-lived
  `CommandProcessor` that carries `take_restricted_integer_recoveries`. A
  processor-scoped accumulator is discarded when a step rolls back, which is
  right for a diagnostic scanned and printed inside one step -- but an open
  and its matching close are normally in _different_ steps, so a
  processor-scoped queue would lose every open before its close arrived.
  `CommandState`'s own step snapshot is a wholesale clone and restore, so a
  field there is captured and rolled back with everything else: a rolled-back
  open prints no paren, and a committed one cannot be dropped.
- **`tex-exec`** gains `job.rs`, which owns the banner, the `**` line, the
  `open_parens` count, and the §1332 tail, and drains the command layer's
  queue once per step.

## Why the banner says pdfTeX

Byte-for-byte comparison against a pinned reference engine is the whole point
of the minifixture corpus, so the banner is the reference engine's banner --
the same string `umber::pdf_output` already writes as a PDF `Producer`. A
banner that said "Umber" would make every committed channel differ on line 1
forever, and the difference would carry no information.

The clock is the one thing that cannot match: the reference log's first line
ends in the host's wall clock. Both sides are compared after the documented
normalization `sed '1s/)␣␣.*$/)␣<HOST-CLOCK>/'`, and nothing else is
normalized away.

## Why the notices are configuration, not output

pdfTeX's own startup adds two lines TeX82 has no analogue for:

`␣restricted␣\write18␣enabled.` and `␣%&-line␣parsing␣enabled.`, each with the
leading space tex.web's `print_nl` leaves.

Umber could print them unconditionally and match, but both would be lies: the
minifixture world runs `ShellEscapePolicy::Disabled`, and Umber does not
implement `%&` first-line parsing at all. The honest fix is on the other side
-- the oracle runner passes `-no-shell-escape -no-parse-first-line`, which is
the configuration Umber actually is, and neither engine prints either line.

This is the general rule for this corpus: when the two engines' output differs
because they were _configured_ differently, fix the configuration; normalize
only what cannot be configured, and let everything else stand as a real
divergence.

## What this does not fix

§310's `show_context` -- the `l.3␣\showbox0` line and its split-line
continuation after every error -- is a separate and larger absence, measured
across the corpus as the dominant log-channel divergence class and tracked as
`umber2-alfh.8`. Job framing is its prerequisite, not its substitute.

Likewise, `\showbox` and `\showlists` still write their dump to both channels
rather than through §245's `begin_diagnostic` redirection, so a
`\tracingonline=0` run puts on the terminal what the reference engine sends
only to the transcript. `crates/tex-exec/src/diagnostics.rs` records that;
it is `umber2-alfh.9`.
