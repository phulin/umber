# Job Framing

TeX's terminal and transcript are not raw diagnostic streams. They are the two
halves of a **job**: a run that announces itself with a banner, echoes the
command line it was given, brackets every file it reads in parentheses, and
signs off with a page count and the transcript's name. tex.web spreads that
framing across §534's `open_log_file`, §537's `start_input`, §362's file
exhaustion, §642's `finish_dvi`, and §1333's `close_files_and_terminate`.

Umber emitted none of it. A `\showbox` run's transcript began at the `>␣\box0=`
line and ended there, while the same source through pdfTeX 1.40.29 produced:

```
This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) (INITEX)  9 JUL 2026 13:36
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

The retained public driver exposes startup acquisition through
`StartupInput` and `EngineSession::acquire_startup_root`.
The adapter supplies the initial `**` line and any §530 replacement lines;
the existing `ResourceHost` still selects immutable bytes or reports
absence. This keeps terminal interaction out of engine crates and gives native,
browser, and test drivers the same bounded protocol.

The startup line and the selected root name are separate facts. TeX82 §534
echoes the complete terminal buffer after `**`, including driver syntax such
as web2c's `&trip`, while §§528--529 derive `job_name` from only the parsed
filename component. Prepared-format jobs therefore carry explicit startup
invocation text alongside the immutable root registration; a format selector
is preserved in the log without becoming part of `\jobname` or source
identity.

## What a job prints, and when

| tex.web                     | Text                                                               | Sink                     |
| --------------------------- | ------------------------------------------------------------------ | ------------------------ |
| §61 `wterm(banner)`         | `This is …␣(INITEX)`, then a newline                               | terminal, before the log |
| §536                        | the same banner plus `format_ident` and the clock                  | log                      |
| etex.ch §536/§1337          | `entering extended mode`                                           | both                     |
| §534                        | `**` and the job's first line, then a newline                      | log                      |
| §537 `start_input`          | `(` and the opened file's name                                     | both                     |
| §362                        | `)` when a file's last line is consumed                            | both                     |
| §1335 `final_cleanup`       | `␣)` once per still-open file                                      | both                     |
| §1333 `tracingstats`        | TeX82 allocator and stack usage report                             | live selector            |
| pdftex.web §§794--798/§1600 | unresolved destination, structure-destination, and thread warnings | both                     |
| pdftex.web §73              | `pdf_error` diagnostic and PDF-specific fatal close line           | both                     |
| §642 `finish_dvi`           | `No pages of output.` / `Output written on …`                      | both                     |
| §1335                       | `(see the transcript file for additional information)`             | terminal only            |
| §1333                       | `Transcript written on ␣<jobname>.log.`                            | terminal only            |

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
- §1333's usage block is emitted only when `\tracingstats` is positive. It is
  routed through the ambient selector and precedes the DVI and transcript
  termination lines. Every §1334 row, including the final stack-usage row,
  retains its direct `wlog_ln` terminator. Those direct writes change the log
  bytes without changing §54's `file_offset`; if the log line was open before
  the usage block, §62's guard therefore makes §642's shared `print_nl` emit
  one more line break after the final row. A separately partial terminal line
  has the same effect. The two channel offsets must not be collapsed.
  `Universe::engine_usage_statistics` projects interned strings and characters,
  token/glue/node words, control sequences, font information, fonts, and
  hyphenation exceptions without exposing raw stores to the execution layer.
  Its §1334 stack fields are runtime-only high-water
  projections supplied by their semantic owners: `tex-command` records
  §§31/374 buffer use, §321 input depth, and §390 macro parameters;
  `tex-state` projects §§273--280's boundary and restore words from its typed
  journal, `tex-command` contributes command-owned `\aftergroup` words, and
  `tex-exec` merges those with §645's box-specification words while recording
  §216's mode nest. Formats and semantic hashes do not include these
  diagnostic counters; rollback restores live save-stack projections while
  retaining job-lifetime high-water marks.

## Where each piece lives

Printing is `tex-state`'s (`Printer` over §54's `selector`), file opening is
`tex-command`'s (§537 is an input-stack operation), and the job lifecycle is
`tex-exec`'s (`MainControl` is what starts a job and what reaches
§1332). No one layer sees all three, so the framing is assembled rather than
placed:

- **`tex-state`** gains §76's `history` on `ErrorChannel`, beside the
  `error_count` §82 already maintains there, and `file_framing.rs`: §54's
  `open_parens` plus the three prints that maintain it (§537's `(name`,
  §362's `)`, §1335's `␣)` apiece). Both are print-adjacent `World` state and
  both roll back with the effects that carried what they count.
- **`tex-command`** owns the name on an opened source (§537's
  `a_make_name_string`, which Umber's anonymous byte registrations had no
  place for). A file or traced-`\scantokens` push returns its one opening name
  to the processor that already owns the live `CommandContext`, which renders
  it immediately. Retirement returns its one close fact through the existing
  `InputRetirement` result. Neither transition enters persistent command or
  snapshot state.

  **The close point is part of the contract.** §362 is

  ```text
  print_char(")"); decr(open_parens); ... end_file_reading;
  check_outer_validity;
  ```

  -- the `)` precedes a diagnostic printed one statement later from inside
  `get_next`. The processor therefore prints the retirement's call-local close
  after `file_warning` and before it resumes outer-validity checks. Delaying it
  to the end of the step would put `Incomplete \iffalse` and the runaway
  family _inside_ a file bracket tex.web had already closed.
- **`tex-exec`** gains `job.rs`, which owns the banner, the `**` line, and
  the §1332 tail. A retained `EngineSession` has already opened its root before
  command execution, so its startup seam reads the name from that live source
  level and routes §537's opening directly through `tex_state::file_framing`.
  An externally framed root uses the host-supplied startup name instead, and
  §1335 can close an unconsumed root with `␣)`.

  pdfTeX's navigation warnings are later than that generic cleanup: the
  session first completes `\end`'s last page ejection, then `job.rs` walks
  the checkpointed destination and thread ledgers. Only undefined ordinary
  and structure destinations and threads with no beads are reported. Their
  dedicated terminal-publication phase keeps the late notices atomic, while
  retained root-body projections extend through that phase only when it
  emitted a warning.

  pdfTeX navigation scanner and traversal failures use a distinct fatal
  publication. The failed operation or page transaction is settled first,
  then `pdf_error` runs §93's `succumb`, publishes the exact terminal/log
  asymmetry, and returns the original typed extension error to the retained
  session owner. Thus the host sees both canonical fatal output and the
  specific ext1/ext4 failure identity; an already committed earlier page does
  not make the failed page or its navigation objects visible.

The same ordering rule binds the diagnostics that share these channels: a
report the command core detects has to be _printed_ by the command core.
§§433--437's restricted-integer range error was queued for the executor to
render, which put `! Bad register code (256).` after the `)` of the file it
was still reading; it now reports from inside `scan_restricted_integer`.

The retained result snapshots process status only after this entire
finalization sequence. `TexRunStatus::Success` covers §76's `spotless` and
`warning_issued` histories, matching Web2C's successful exit rule;
`CompletedWithErrors` exposes `error_message_issued` without converting a
recoverable TeX diagnostic into a session failure, and `Fatal` accompanies
the existing typed fatal terminal state. Structured in-memory runs return the
same result, while the string convenience helpers continue to return the
selected terminal projection for every semantically completed run.

## Why the banner names the selected engine

Byte-for-byte comparison against pinned reference engines is the whole point
of the conformance corpus, so job framing selects the canonical banner from
the immutable engine-binary identity: TeX82 uses tex.web §2's `TeX` banner,
e-TeX 2.6 uses etex.ch §2's replacement, and production pdfTeX uses
pdftex.web §2's replacement. The loaded format name and dump clock remain
separate framing facts. The binary identity is distinct from the semantic
command profile, because a newer reference binary can exercise an older
profile; incompatible combinations fail at the provider boundary. That
identity also selects the canonical compiled semantics which survive loading
an older format: pdfTeX 1.40.29 executing a TeX82 profile still uses
pdftex.web §459's extended invalid-unit help. It does not change the loaded
profile or its format fingerprint. The pdfTeX string is also the value
`umber::pdf_output` writes as PDF producer metadata. A banner that said
"Umber" would make every committed channel differ on line 1 forever, and the
difference would carry no information.

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
