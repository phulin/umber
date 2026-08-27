# TeX Command Core

Status: authoritative target architecture for Beads epic `umber2-johp`.

## 1. Purpose

Umber implements one `tex-command` command-processing subsystem whose semantic
structure follows TeX82, e-TeX 2.6, and pdfTeX 1.40.29 directly.

The central architectural fact is that TeX does not have a clean semantic
boundary between lexical input and expansion. TeX's `get_next` reads physical
characters or stored token lists, substitutes macro parameters, resolves
control-sequence meanings, enforces scanner status, updates `align_state`, and
intercepts alignment delimiters. Its real downstream boundary is the delivery
of an unexpandable current command to main control.

`tex-exec::MainControl` is the production main-control seam for that boundary:
it accepts no raw input cursor, obtains each `CurrentCommand` and every
assignment operand through `CommandProcessor`, then applies only the completed
typed structural mutation after the processor borrow ends. Macro calls and
registered `\\input` nesting therefore remain command-core operations; the
executor never rereads their source text.

`umber::EngineSession` constructs that command machine at startup and registers
its retained root plus already-acquired nested `World` resources through typed
capabilities. A session with a registered root executes its bounded operations
through `MainControl`; resource acquisition, transaction rollback,
and final effect/artifact commit remain host-owned. Direct, CLI, virtual,
format-loaded, editor/incremental, and WebAssembly entry points all compose
this same command-owned transition machine; there is no runtime command-path
selector.

When main control starts a paragraph from vertical mode, replay makes the
mode decision at the executor seam but asks the still-live command processor
to perform TeX82's `back_input` on the triggering ordinary character. The
typed paragraph-start result then changes the executor mode after that borrow
ends; the first character is reconsidered only through the command core's
backed-up input level.

Canonical `\font` definitions scan their target, optional equals, expanded
filename, and `at`/`scaled` clause into an immutable `FontLoadRequest`. TeX82
§1257's `new_font` runs `define(u,set_font,null_font)` on the `get_r_token`
target _before_ the optional equals and filename, so `CommandProcessor` makes
that provisional null-font definition and publishes its meaning mutation
there, under the `\global`/`\globaldefs` scope main control selects, exactly
as §1224's provisional `\relax` is. §1257's `common_ending: equiv(u):=f` then
overwrites the equivalent directly rather than through a second `eq_define`,
so the completed definition publishes no second mutation. After
that processor borrow ends, replay resolves the request through its transient
registered-font capability and installs the loaded meaning atomically. An
absent capability is a typed font suspension, so the enclosing aggregate rolls
back before a fresh processor episode retries; a completed unavailable lookup
instead recovers the target to `nullfont`. Font capabilities and loaded
resources never enter command snapshots or durable summaries.
If a valid TFM reaches the font-bank bound, replay follows TeX.web §567: it
reports `not loaded: Not enough room left`, retains the provisional
`nullfont` meaning, and commits no partial font row.

The same boundary has a typed canonical math-request vocabulary for TeX82
§§691–734 and §1030+: math characters and family selectors, text-field and
script markers, limit switches, generalized fractions, styles and choices,
delimiters/radicals/accents, `mu` glue or kerns, and equation-number entry.
Command processing completes scalar operands (including optional fraction
delimiters and `mu` units) and retains range recovery/provenance before the
borrow ends. It also freezes opaque completed math-field and braced-mlist
episodes, including script attachments, four `\mathchoice` branches, and
`\left`/`\right`/`\middle` delimiter boundaries. The executor replays those
episodes only through typed command-state handles, so mlist construction and
display packaging receive no raw token, input stack, or scanner-status
capability.

Math-shift pairing is command-owned: entry and display-closing lookahead
either consumes the second shift or restores a non-shift through the ordinary
backup level. Main control then owns only typed math-shift
group/mode transitions, `\everymath` or `\everydisplay` replay, paragraph
interruption/resumption, Appendix-G lowering, equation-number subformula
packaging, and vertical display contribution.

The replay seam also retains the executor-side mode projection and obtains
observable general-text effects (currently `\\message`) through the typed
structured scanner. Alignment lifecycle state crosses it only as
`AlignmentRequest`; active-cell delivery uses
`CommandProcessor::get_x_alignment_delivery`, and an intercepted delimiter is
returned to that same processor episode for v-template installation. Thus
replay does not turn alignment, mode changes, or effects into a second source
consumer.

Canonical replay opens typed alignment, row, and cell mode frames around that
delivery and freezes each completed cell list as structural material. The
later `fin_align` migration alone is responsible for resolving unset widths,
converting those frozen records, and inserting the finished alignment list.

An executor-requested stored replay list has a second, typed delivery result:
after command processing retires the exact stored level (including its normal
observer and provenance transition), it delivers `Completed(CommandReplayEpisode)` before
fetching from the enclosing input level. If expanding the stored level's final
token pushes a macro replacement or another descendant input level, completion
remains pending until every such newer level retires. The executor uses that
result to finalize an isolated mode/group—for example, to freeze an mlist field or a
braced math group—then requests the next expanded delivery. Ordinary
`get_x_token` retains TeX82's uninterrupted behavior by consuming this
boundary internally. Thus no stomach operation can peek at or back up a
parent token merely to discover that a stored episode ended.

Because ordinary `get_x_token` consumes that boundary internally, the
one-shot `Completed` event is only guaranteed to surface at the executor's
own top-level fetch for the _next_ command. A scalar operand scan belonging
to the episode's own _last_ command (`scan_dimen`'s optional-space lookahead
after a trailing `\kern1pt`, for example, per TeX82 §455) can just as well be
the exact probe that retires the stored level, and that swallowed retirement
never reaches the executor as an event. A driving loop must therefore treat
`CommandState::replay_episode_is_active` as the authoritative retirement
test—polled after each step, not merely inferred from one `Completed`
delivery—exactly as `execute_discretionary_part` already does; watching only
for the event risks continuing past the episode's true end and folding
unrelated following material into it.

Text `\\accent` and `\\discretionary` use the same completed-scanner boundary:
the command processor owns the accent number, expanded base-character lookup,
and non-character replay, and freezes each discretionary group as traced,
immutable material. `MainControl` replays each frozen part as its own
stored command level inside a `disc_group` restricted-horizontal episode,
flushes and freezes the completed node list, then applies the typed `Disc`
node. Group-local definitions and recovery remain live command/Universe state;
the group's `\aftergroup` payload is backed up like any other group's (§33.9),
because §1120's `build_discretionary` opens with a bare `unsave`. This
aggregate operation remains under one rollback
snapshot: it must not recreate parallel command/input state or expose raw group
delimiters to the executor.

During the command-owned preamble scan, TeX82 §760 removes catcode-10 spacer
commands only while collecting the beginning of each u-template; a v-template
and spaces after the first u-template token remain frozen verbatim. This
normalization happens before `AlignmentPreamble` crosses to `tex-exec`, which
therefore still receives immutable templates and never interprets their raw
tokens.

Umber will preserve that command-machine boundary while deliberately replacing
TeX's global variables, linked `mem` nodes, synchronous host I/O, and
allocation identities with:

- typed Rust state;
- immutable content stores;
- transactional snapshots;
- compact provenance;
- typed resource suspension;
- explicit engine profiles;
- persistent incremental sessions; and
- static TeX82, e-TeX, and pdfTeX command dispatch.

The result is a faithful kernel inside a modern envelope:

```text
host/session policy
        |
        v
World, resources, effects, incremental execution
        |
        v
Universe and aggregate state mutation
        |
        v
tex-command
  InputState -> get_next -> get_x_token -> CurrentCommand
                    |             |
                    |             +-- expand
                    +-- scanners, conditions, alignments, macro_call
        |
        v
tex-exec main control and stomach
```

This document defines the intended end state. Work decomposition and status
belong in Beads epic `umber2-johp`; this file is not a task checklist.

### 1.1 Typed command delivery

Raw `get_next`/`get_token`, expanded `get_x_token`/`x_token`, replay-aware
fetches, main-loop lookahead, protected and undefined-preserving fetches, and
alignment delivery are policies over one private command-delivery driver.
The policy names replay-completion handling, expansion and its depth, terminal
observation ownership, first-command handling, protected and undefined
meanings, and alignment `end_template` interception independently. It does not
name control-sequence creation: every token reaching the driver is already a
character or a packed stable control-sequence identity. Canonically named
methods remain as thin entry points; they do not own alternate fetch loops.

The driver is destination-directed. Its caller provides the one final
`Option<CurrentCommand<G>>` slot for that active request; raw resolution and
expanded settlement construct or mutate the command in that slot. The return
is only a compact `DeliveryStatus` naming end of input, command completion,
replay completion, pending observation, or an alignment boundary. A command
moves out of this slot only into its final consumer or the one typed expansion
suspension slot at a real resource barrier. There is no process-global slot,
mailbox, destination inference, or nested-request reuse.

The input side of that same request uses one fixed call-local
`RawDeliverySlot`. The top input level keeps its packed frame position,
backing handle, source cursor, replay-completion frontier, and rollback
authority. Its storage-lifetime tag was selected when the level was created;
delivery borrows that domain, writes the spelling and only-present provenance
directly into the raw slot, and advances the fixed frame in place. A macro
parameter candidate pushes its argument range and restarts before current
command construction. The raw slot is discarded on return and never enters a
scanner or resource continuation.

Control-sequence resolution borrows the already-admitted dense meaning row
through the live `CommandContext`. Static words decode during that borrow; a
macro meaning clones its generation-branded definition owner exactly once into
the owned `CurrentCommand`. The row borrow ends with resolution, before outer
recovery, alignment handling, expansion, execution, assignment, replay, or
suspension can mutate state. Assignment level remains solely in the dense bank,
so delivered-command ownership does not duplicate journaling or reinterpret a
meaning after delivery.

`CurrentCommand::resolve_into` lends the initialized caller destination back
to the raw driver, so the remaining delivery steps mutate and observe that
one final command in place. Alignment classification writes its exact
`AlignmentDeliveryAdjustment` into the same command before raw observation;
backup later consumes that recorded adjustment rather than reclassifying the
spelling. Internal ErrorStop deletion, math-shift lookahead, and recovery-list
draining likewise provide their discard-or-backup slot directly to the driver.

The value-returning entry points are conveniences over the same destination
driver; the executor hot loop and destination-aware callers use
`get_next_into`, `get_token_into`, `get_x_token_into`, and the replay/alignment
counterparts. In particular, alignment lookahead carries either `Committed` or
`PendingExpanded`; no boolean can silently invert who must commit the expanded
observation before backup. Every expanded invocation enters and leaves the
persistent expansion-depth counter through the same balanced boundary,
including error returns.

## 2. Canonical authorities

Compatibility behavior is derived in this order:

1. Knuth's TeX82 `tex.web`;
2. e-TeX 2.6 and its canonical change files;
3. pdfTeX 1.40.29 and its canonical change files;
4. reference behavior from pinned, transparently instrumented builds of those
   engines;
5. explicit Umber extension contracts for behavior outside their domains.

The retired Umber implementation is never a behavioral oracle. Existing tests
may be retained only when their expectations can be traced to a canonical
source, a pinned reference fixture, or an explicit Umber extension contract.

Live command-oracle regeneration has one supported interface:

```bash
scripts/regen-fixtures.sh --oracle all --profile canonical [--offline]
```

`tests/oracle-regeneration-manifest.txt` pins the regeneration-contract and
event-schema versions plus exact source-manifest, profile, fixture-area, and
build identities for TeX82, e-TeX, and pdfTeX. Engine-specific selectors use
the profiles documented in the three oracle references. The aggregate command
validates identity before acquisition, delegates to the pinned builders, and
records success only after every clean/instrumented artifact comparison and
semantic-trace validation passes. Correctness tests consume committed fixtures
and never invoke this live workflow.

The primary TeX procedures mapped by this design are:

- input stacks and scanner status in TeX.web part 22;
- `get_next`, `get_token`, `get_x_token`, and `x_token` in parts 24 and 25;
- `expand` and `macro_call` in part 25;
- basic scanners and `scan_toks` in parts 26 and 27;
- `pass_text` and conditional state in part 28; and
- alignment template delivery and main-control integration in the alignment
  and main-control parts.

### 2.1 Alignment template delivery

`AlignmentCellTemplates` carries immutable traced u- and v-template lists.
Starting a cell establishes its delivery state, then command processing
delivers and backs up the first source opening brace before the executor's
typed template-install request pushes its ordinary u-template as a stored
input level, even when its token list is empty; only `\\omit` selects no
u-template level. When that exact level retires, raw delivery returns to
cell-body depth. A completed cell-body scanner resumes through that same alignment
delivery entry point: a tab, `\span`, or `\cr` is recognized by
`get_next` while the body depth is zero and becomes the opaque delimiter event,
not generic expanded delivery. A scalar `get_x_token` probe (including
`scan_keyword` from `scan_rule_spec`) instead completes the same typed
command-owned v-template handoff immediately and restarts expansion, matching
TeX82 §25: it never converts that intercepted boundary to `endv` or exposes it
to the executor. This follows TeX82 `get_next`/`get_x_token` (§§24--25),
`scan_keyword` and `scan_rule_spec` (§26), and the
`init_col`/`fin_col`/`do_endv` template lifecycle (§§765--772). During the
raw preamble scan, TeX82 §760's `&&` records the start of a periodic suffix;
the second tab is not an empty column, and §772 selects that frozen suffix
when an exhausted preamble is extended. When that selected u-template begins
with `\hskip`, TeX82's `scan_glue` (§458) probes and backs up its first
numeric token before `scan_dimen` (§455) replays it. The command processor
returns the completed `GlueSpec` only after that backup lifecycle; replay
then appends the typed glue node and never reads the operand itself. An ordinary
empty u-template follows that same `begin_token_list`/`end_token_list`
lifecycle: TeX82 §37 selects the non-omit branch, §760 preserves the empty
preamble prefix, §765 installs it, and §772 resumes after the matching
retirement. On an
intercepted delimiter delivered to main control, executor `end_template` handling calls
`CommandProcessor::begin_alignment_v_template`: it records the original
delimiter as an opaque `fin_col` outcome while the stored v-template replays,
so all suffix tokens (including macro expansion and definitions) restart
through `get_next`. Exhausting the exact v-template retains its frame and
emits raw frozen `end_template`; §343 changes `cur_cs` to frozen `endv` while
preserving the canonical `endtemplate` observation spelling,
after which successful `do_endv` uses `CommandState::finish_alignment_cell`
to return a typed completion proof and retire that exact frame structurally.
When a scalar scanner has backed up the effective `endv`, the proof validates
the exhausted backup immediately above the retained frame; TeX82 §§343, 765,
and 772 require the canonical completion order: `fin_col`'s state change,
backup retirement, then v-template retirement. Active frame identities live in
`AlignmentDeliveryState`, and therefore suspend with nested alignments and
are cloned by command snapshots.

`tex-exec` crosses this boundary through `AlignmentRequest` only: begin and
restart preamble scanning, begin/install/finish a selected cell,
suspend/resume an outer alignment, and finish the alignment. `CommandState` applies those
structural requests without receiving a token. Expanded delivery uses
`CommandProcessor::get_x_alignment_delivery`; an intercepted delimiter is an
opaque `AlignmentDeliveryEvent::EndTemplate`, which is handed back to
`begin_alignment_v_template`. Thus the executor never reclassifies a tab,
`\span`, or `\cr`, while `off_save` recovery and group policy remain
executor-owned after the typed event has been delivered.

`get_x_alignment_delivery` also surfaces `AlignmentDelivery::Completed` for
an executor-owned replay episode (a `\mathchoice` branch or a discretionary
part) that retires while a cell's own content is being
delivered -- for example plain.tex's `\vphantom`/`\mathpalette` building a
`\mathchoice` inside an inline `$#$` cell template. It uses the same
completion-aware raw fetch as ordinary (non-alignment) `get_x_token`
(`docs/tex_command_core.md` §2.1's swallowed-retirement discussion applies
identically here): the retiring level can sit below several other
exhausted-but-unpopped levels (nested macro expansions, parameter
substitutions, a trailing operand scan with no lookahead of its own), so the
next real token the cascade finds can belong to the _enclosing_ cell/field
context rather than the episode. `scan_alignment_delivery_step` reports this
as `ColdOperation::ReplayCompleted`, exactly like ordinary `scan_step` already
does via `get_x_token_with_replay_completion`, rather than risking that
enclosing token being misattributed to the just-retired episode.

For TeX82 §§1064, 1066, and 1131 `off_save`, the executor chooses only the
typed structural closer. `tex-command` reports `off_save_replay` before it
backs up the offending command and inserts that closer, so raw delivery cannot
overtake the diagnostic; at bottom level it reports the drop before the backup
retires. This applies equally to ordinary `\endgroup` recovery and alignment
`endv` replay.

TeX82 §1215 aliases of `\endgroup` retain its `end_group` command and take
the same §§1063--1066 dispatch regardless of their user spelling. Main control
dispatches on that meaning, while inaccessible frozen alignment tokens retain
their distinct `end_template`/`endv` meanings and continue through the
alignment-owned paths above.

`CommandProcessor::recover_off_save` takes the closer as a token slice rather
than a single token, because §1065's four cases are not all one token: a
`math_left_group` needs the two-token `\right.` (frozen `\right` followed by
a `.` other-character), while the other three cases each need exactly one.
`CommandProcessor::frozen_primitive_token` looks up the frozen,
redefinition-proof control-sequence token backing a primitive by name (e.g.
`"endgroup"`, `"right"`), shared with the pre-existing `check_outer_validity`
frozen-insertion recoveries (`\par`/`\fi`/`\cr`) rather than duplicated.
`tex-exec`'s `scan_off_save` (`main_control.rs`) is the executor-side
half: given a command and the innermost `GroupKind`, it selects and issues the
matching closer (`SemiSimple` → `\endgroup`, `MathShift` → `$`, `MathLeft` →
`\right.`, otherwise → `}`, or the bottom-level drop when no group is open),
returning `ColdOperation::OffSave`/`OffSaveBottomDrop` so the execute phase
prints the matching "Missing ... inserted"/"Extra ..." text once, after
scanning already ran the recovery. It is written as a general, reusable
routine rather than inlined at its first call site (TeX82 §1095's
`head_for_vmode` restricted-`hmode` branch, reached by `\vskip`/`\vfil`/
`\vfill`/`\vss`/`\vfilneg` inside an `\hbox`); future primitives that reach
`off_save` in other modes should call it too instead of re-deriving the
four-way group dispatch.

`\end` and `\dump` (TeX82's `stop` command) dispatch on `abs(mode)` like
every other main-control case, and every arm is one of the general recoveries
above:

- `hmode+stop` (§1094) takes §1095's `head_for_vmode`. Unrestricted
  horizontal mode backs up `\end` (or `\dump`) and then backs up the
  synthesized primitive `\par` with inserted input ownership, both owned by
  `tex-command`; restricted horizontal mode takes the same §1064 `off_save`
  the `\vskip` family does.
- `mmode+stop` is one of §1046's math-only cases, so §1047's
  `insert_dollar_sign` closes the math and retries the stop in the resulting
  mode.
- `vmode+stop` (§1045) is §1054's `its_all_over`, and it is the only exit
  from main control.

§1054 is the whole end-game decision and it has exactly two outcomes. §1051's
`privileged` first rejects any mode below outer vertical -- inside a `\vbox`,
an `\insert`, or `\output` itself -- with `report_illegal_case`. Then the job
ends if and only if the current page and the contribution list are both empty
and the last output was not a dead cycle. Otherwise the stop is backed up and
`\hbox to \hsize{}`, `\vfill`, and `\penalty-'10000000000` are appended to
the contribution list before §994's `build_page` runs.

Nothing about `\output` is decided there. Whether the ejection reaches an
output routine at all, with which `\box255`, and whether §1005's dead-cycle
escape ships the page directly instead, is §1005/§1012's decision, and §1025
is the single place a `\output` token list is ever pushed. A `stop` dispatch
that consulted `\output` itself would run the routine on a job with nothing
to print, where TeX82 reports "No pages of output." instead.

Once `its_all_over` is true, §1335's `final_cleanup` unwinds every input
level main control has abandoned -- the root file at end of text, an
unfinished macro body, a live token list -- without reading any of them.
Normal stop, fatal §93 `succumb`, and terminal exhaustion then converge at
§1332's `end_of_TEX` label. Main control publishes the following
`close_files_and_terminate` observation after either a scanned stop's cleanup
records or a fatal diagnostic. Terminal exhaustion has no scanned command, so
the retained session publishes the same boundary after its source-stop record.
Every completed outcome therefore publishes it once; a resource suspension or
exhausted command-fuel budget publishes no termination.

Canonical `\shipout` replay likewise crosses the executor only as a completed
box. The artifact kernel receives an already-published detached input summary;
it never constructs or resumes a command/input machine as a publication
fallback.
TeX82 §1293's `show_whatever` follows the same seam: `\show` consumes and
renders its raw operand, while `\showthe` and `\showbox` complete their
expanded/internal scans in `tex-command`; replay writes only the frozen
diagnostic request and never probes an executor input source.
Explicit `\begingroup` is a typed `SemiSimple` entry and `\endgroup` its
matching typed exit. TeX82 §1064 recovery remains command-owned: malformed
macro targets and parameter markers preserve/replay their source spellings in
the command machine, while main control applies only the resulting diagnostic
effect and recovered definition.

Replay models that routine as an explicit output group and internal-vertical
mode. Its required opening brace is consumed by `scan_left_brace`, not as a
nested ordinary group; the matching close ends its paragraph, leaves the
output group, and restores outer vertical mode before output-list retirement.
A stop backed up below the output list can therefore retire before a later
`its_all_over` retry, instead of spuriously taking `head_for_vmode`'s
horizontal recovery path (TeX82 §§1025, 1095, 1131).

At TeX82 §23's `check_outer_validity` boundary, an aligning scanner reports
the typed `outer_validity` alignment recovery after its EOF diagnostic and
before command processing pushes the frozen `\cr` recovery list. The record
carries the live command-owned alignment identity and `align_state`; frozen
token spelling and recovery kind remain the separate input/recovery records.
Unlike other recovered scanner episodes, `aligning` remains live through the
inserted frozen `\cr`; typed preamble completion then publishes its sole
`aligning -> normal` transition before the executor begins cell lookahead.

When that same delivery entry point expands the retained frame's
`frozen_end_template` to `EndV`, replay routes the command through the typed
`FinishCell` request rather than treating `EndV` as ordinary main-control
continuation. This is TeX82 §§343 and 772: the command processor still owns
expansion and the retained frame, while the executor applies the cell
lifecycle only after receiving the completed command. On success, replay
publishes the canonical state-change, v-template input retirement, and
v-template retirement observations in that order. TeX82 §772's `fin_col`
then selects the next column, span continuation, or row from that typed saved
delimiter; §785's following-entry lookahead remains command-owned, delivering
and skipping spaces before it backs up the first next-cell token for the
selected u-template. The delimiter never reappears as ordinary raw delivery.

Command observation preserves TeX82's raw command identity before that
interception: `\cr` and `\crcr` are both `car_ret`, with `cur_chr` 257 and
258 respectively. `\omit` is likewise observed as its own `omit` command with
`cur_chr` 0. After a completed row, `\noalign` is likewise observed as
`no_align` with `cur_chr` 0 before TeX82 §37's `align_peek` consumes it; it
must not be projected as a generic unexpandable primitive. TeX82 §37's
`init_col` consumes that expanded lookahead, assigns the cell-body sentinel
from `1000000` to `0`, and bypasses both backup and u-template replay; the
executor receives only this typed omit-cell transition.
These identities are emitted at both
raw and ordinary expanded delivery boundaries; they do not alter alignment
delivery semantics. This follows TeX82 §15 (command codes), §18 (primitive
initialization), and §37 (`init_col`/`fin_col` alignment handling).

Likewise, `\shipout` is observed as `leader_ship` with `cur_chr` 99 before
replay scans its box. TeX82 §15 assigns the shared `leader_ship` command code,
and §35 initializes `\shipout` with `a_leaders - 1` (`99`); the executor
receives only the completed typed box/output semantics.

Likewise, `\expandafter` is observed at raw delivery as `expand_after` with
`cur_chr` 0 before TeX82 §25 reads its first and second tokens. The command
processor owns that lifecycle: it expands (or backs up) the second token and
replays the first above the resulting input, so the identity is never inferred
from the replayed token or a fixture-specific observer rule.

Likewise, `\csname` is observed at raw delivery as `cs_name` with `cur_chr`
0 before TeX82 §25's name-construction loop. TeX82 `tex.web` defines
`cs_name=max_command+7` and installs `\csname` with selector zero; the loop
then expands name characters through `\endcsname`, interns the completed
control sequence, and injects it through ordinary input. The matching boundary
is likewise `end_cs_name` with selector zero. `CurrentCommand` carries both
identities from raw delivery through that lifecycle, rather than having
observation infer either from a generic expandable meaning.

Likewise, TeX82's classic text conversions are observed as the shared
`convert` command while `CurrentCommand` retains their §35 selector:
`\number`, `\romannumeral`, `\string`, `\meaning`, `\fontname`, and
`\jobname` use selectors 0 through 5. TeX82 §470's `conv_toks` continues to
own their respective operand scans and inserted-token lifecycle; the command
identity is selected at raw delivery, rather than projected later from a
generic expandable primitive. Startup filename scanning installs the selected
area-free, extension-free job name through `CommandHostCapabilities`; it is a
borrow-scoped environment fact, so snapshots retain neither a host path nor a
fixture-derived conversion value.

For TeX82 §§1075 and 1084, `\shipout` begins a typed box-completion episode:
command control delivers the next `make_box` command and owns every scalar
box-register scan, including its one-token terminator backup. On the closing
brace, replay performs §1075's `box_end`/`ship_out(cur_box)` synchronously;
the DVI-page effect is consequently published before that terminator backup
retires on the next raw fetch. The executor receives the completed box node,
not an input capability or a token to reread.

The shipout effect itself belongs to the page commit, never to a command.
TeX82 §640's `dvi_out(eop); incr(total_pages)` is the only place a page
reaches the DVI file and §638's `ship_out` is the only routine that reaches
it, so replay derives the effect from the committed-artifact delta across an
applied step rather than from the step's identity. That covers both of
tex.web's entry points into `ship_out` by construction: §1075's `box_end` for
an explicit `\shipout`, and §1012's `fire_up` through §1025's null-`\output`
case for every residual page §994's `build_page` ejects on its own -- notably
the one §1054's `its_all_over` forces before `\end` may finish. As in §640
the published page number is read after the increment, so it is the one-based
ordinal of the page just committed.

Likewise, `\box` is observed as `make_box` with `cur_chr` 0 before command
control invokes TeX82 §1079's `scan_int` for its box-register operand. That
command-owned scan preserves raw digit delivery and any terminator backup;
TeX82 §15 assigns the shared `make_box` command code, and §35 initializes
`\box` with `box_code` (`0`). The executor consumes only the resulting typed
box semantics. Under the e-TeX profile, [47.1079] replaces the eight-bit
selector for both `\box` and `\copy` with `scan_register_num`, so this same
command-owned scan accepts sparse box registers through 32767 while TeX82
retains its 0--255 bound.

The same profile boundary owns box-dimension access. TeX82 §424 reads and
§1247 writes `\wd`, `\ht`, and `\dp` through an eight-bit box selector;
e-TeX [26.420] and [49.1247] replace both directions with
`scan_register_num` plus the sparse box fetch. Assignment and later internal
scans therefore address one extended box register rather than independently
recovering an out-of-range selector to box zero.

Vertical splitting follows that boundary too. TeX82 §1082 scans `\\vsplit`'s
source as an eight-bit box number, while e-TeX [47.1082] replaces that scan
with `scan_register_num`; the sparse source is then fetched and updated by
[44.977]. The target of an enclosing `\\setbox` and the source consumed by
`\\vsplit` therefore share the same profile-sized register domain.

After `init_align` validates and replays its opening brace, the command
processor owns the complete `get_preamble_token` episode. It retains raw
delivery while `scanner_status=aligning`, freezes u/v template pairs at `#`
and their column boundaries, and restores normal scanner status only after
the terminating `\cr`. TeX82 `init_row` then reaches command-owned
`align_peek` before `init_col` selects the first cell, so a following right
brace enters `fin_align` instead of being backed up as a u-template opener.
A preamble `\message{...}` is therefore template data, never an executor
effect.

Source citations in implementation comments should identify the narrowest
relevant TeX.web or pdfTeX.web section.

## 3. Goals

The command core must:

1. make every semantic transition recognizable from the canonical programs;
2. have one raw semantic command path corresponding to `get_next`;
3. have one ordinary expanded command loop corresponding to `get_x_token`;
4. represent TeX's current command explicitly without global variables;
5. keep condition state independent of input levels and the save stack;
6. keep the only `align_state` beside raw command delivery;
7. model scanner status as live semantic state;
8. preserve exact macro, conditional, scanner, and alignment recovery;
9. support exact TeX82, e-TeX, and pdfTeX character modes;
10. provide a separately identified Unicode extension mode;
11. carry precise source and expansion provenance without changing semantics;
12. snapshot and restore the complete future-relevant command state
    transactionally;
13. integrate typed resource suspension without retaining Rust
    continuations;
14. record incremental dependencies at aggregate read boundaries;
15. use static extension dispatch with no hot-path plugin abstraction; and
16. permit optimizations only when they preserve canonical semantic events and
    produce a measured whole-workload improvement.

## 4. Non-goals

The command core will not:

- reproduce TeX's Pascal globals or packed `mem` layout;
- load TeX or pdfTeX binary format files directly;
- make scanner calls individually resumable;
- checkpoint arbitrary expansion or scanner continuations;
- support interactive TeX error prompting as a semantic execution mode;
- use the old Umber implementation as an oracle;
- require Unicode extensions to match an 8-bit reference engine;
- use provenance in token equality, meaning resolution, state hashes, or
  control flow;
- dynamically register command handlers in the hot path;
- pre-tokenize source across observable catcode changes;
- publish an optimization merely because it reduces source lines or local
  instruction counts; or
- preserve current crate or API boundaries for compatibility.

## 5. Crate boundary

The final workspace contains one command crate:

```text
crates/tex-command/
```

It replaces `crates/tex-lex` and `crates/tex-expand`. `tex-command` depends on
`tex-state` and shared pure value/resource crates; it does not depend on
`tex-exec`. `tex-exec` depends on `tex-command` and receives completed
unexpandable commands.

The dependency direction is:

```text
tex-state <- tex-command <- tex-exec
```

`tex-state` remains unaware of command interpretation. It exposes typed
aggregate contexts for meanings, code tables, parameters, registers,
provenance, sources, World enquiries, and mutation. `tex-command` owns when
and why those facilities are used.

### 5.1 Public API

The intentionally small public surface is expected to include:

- `CommandState`;
- `CommandStateSnapshot`;
- `CommandSummary`;
- `CommandProcessor`;
- `CurrentCommand`;
- `CommandError`;
- `CommandProfile`;
- source-registration and source-construction inputs;
- narrow alignment lifecycle operations required by `tex-exec`; and
- primitive installation for each supported profile.

### 5.2 Executor scalar-scanner contract

`CommandProcessor` owns every scalar operand read required by executor main
control. Its public `scan_integer`, `scan_dimension`, `scan_glue`,
`scan_keyword`, `scan_optional_equals`, and
`scan_internal_value` operations consume only the command-owned expanded input
stream. Each returns a typed `ScannedScalar` value carrying the first-token
provenance and any canonical recovery; callers never receive an input cursor,
token frame, or raw-delivery capability. Failed optional keyword scans replay
through the same `get_next` path, so executor code cannot create a second
lexer, expansion loop, or backup mechanism. `CommandState::snapshot` remains
the transaction boundary for the resulting future input state.

Within one synchronous scalar call, integer and internal-value results use a
bounded caller-owned frame with disjoint typed-value and `CommandError` slots.
Integer and internal-value call boundaries return only a compact
complete/suspended/failed status; the successful path never transfers the
error-sized carrier. Legacy `Result`-returning scalar boundaries settle at the
producing call site instead of handing the whole carrier to a generic helper.
Completion consumes the value immediately, while only a genuine resource
suspension installs the existing move-only continuation edge in
generation-owned scratch. The call frame owns no heap allocation, retained
arena, cache, or durable state.

Recoverable scalar diagnostics use the same borrow-scoped `CommandContext`
that the processor already holds: its §73 `print_err` forwarding method opens
the live `Universe` error report only for the duration of that report. Thus
TeX82 scanner recovery text reaches the terminal/log channel without placing a
host capability in command state, snapshots, formats, or summaries.
Reports whose canonical `back_error` falls between message/help setup and
§82's `error` may defer that report into an owned, selector-preserving value,
perform the command-owned input backup, and resume it through the same
`CommandContext`. This preserves §403's diagnostic/recovery ordering without
retaining the live `Universe` borrow in command state.

The integer scanner also owns TeX82 §442's backtick character-code form: its
following token is delivered raw and interpreted from `cur_tok`, rather than
from its resolved meaning. Therefore active characters and one-character
control sequences are valid character constants even though they resolve
through a control-sequence meaning; only null or multi-character control
sequences take the improper-constant recovery. The optional trailing space is
then consumed by the normal expanded scanner path, which backs up any
non-space probe. Replay therefore receives only the completed code-table
character and value; it applies `\catcode` and `\lccode` through `Universe`
without inspecting the underlying tokens.

Input levels, scanner frames, condition frames, replay payloads, macro
activations, expansion budgets, provenance construction, and command dispatch
remain crate-private.

### 5.3 Executor structured-scanner contract

`CommandProcessor` also owns the non-scalar main-control operands whose TeX
semantics cross token, macro, and source boundaries. `scan_balanced_text` and
`scan_macro_definition` reuse the one canonical `scan_toks` collector and
return frozen `TracedTokenList` values plus deterministic first-token
provenance. The latter returns parameter and replacement lists separately;
expanded balanced scans continue to use the canonical macro matcher, so macro
arguments never become executor-owned input.

The collector applies TeX82 §479's parameter-character rule -- `#<digit>`
becomes an out-parameter token and `##` becomes one parameter character --
exactly when §477's `macro_def` is set, that is, only for a macro definition's
replacement text. It is never gated on how many parameters the parameter text
declared: a parameterless `\def` still collapses `##`. Every other balanced
scan (`\message`, `\write`, `\toks`, `\mark`, e-TeX `\unexpanded`) stores
parameter characters verbatim.

Within an expanded `scan_toks` collection, `\the` expands only its
internal-value target, as TeX82's internal-value scanner requires. Primitive
register targets (`\count`, `\dimen`, `\skip`, `\muskip`, and `\toks`) keep
their `scan_eight_bit_int` episode in command core: every index digit is
delivered before the first non-index token is backed up, then the completed
value is rendered or spliced. When that target yields a token-list value, the
collector appends the frozen list directly: its tokens are neither reintroduced
into input nor recursively expanded, and they do not affect the collector's
brace depth. This remains a special case inside the collector's one-step
`get_next`/`expand` loop rather than a second ordinary expanded-delivery loop.

`scan_file_name` returns a typed filename after consuming a group or space
terminator, backing up a non-character terminator, or reaching end of input.
`open_registered_input` composes that scan with the borrow-scoped
registered-input capability, then registers
and opens the immutable source through `CommandState`. An unresolved backing
is the typed `CommandError::MissingInput` suspension. After the retained host
binds an authoritative negative response, the same live processor owns TeX82
§530's missing-file text and input context: scroll/error-stop reads a
replacement filename through `CommandContext::input_ln` and retries the
unchanged typed capability boundary, while batch/nonstop composes §93's
`Emergency stop`, fatal history, and terminal state. Neither API exposes a
source cursor, input level, raw token, or host filesystem operation. Snapshot
rollback therefore restores the complete future input and terminal state after
every structured scan.

### 5.3.1 Canonical main-control ownership gate

`tex-exec::MainControl` is the only production executor-facing command driver.
It may classify `CurrentCommand::meaning()` and apply completed typed values,
but it must not construct an independent input stack, call raw-token delivery,
or inspect a raw token carried by a delivered command. The retired executor,
lexer, expander, scanner fronts, and replay adapters have been deleted; the
`tex-exec` architecture test enforces that absence across compiled and dormant
source, while canonical tests exercise typed scanner rollback and
registered nested input. Canonical INITEX replay installs the static TeX82
primitive registries before source registration. Prefix collection, bounded
classical-register recovery, `\globaldefs` resolution, integer/dimension/glue
assignments, and `\advance`/`\multiply`/`\divide` all scan their selectors,
optional equals signs, keywords, and operands through `CommandProcessor`.
This includes ordinary and mu glue: the scalar scanner recognizes `mu` only
for a mu-glue request. Replay retains only a completed typed selector and
value, resolves the effective global bit exactly once at the `Universe`
mutation boundary, then publishes the committed mutation observation. Code
tables (`\catcode`, case, space-factor, math, and delimiter codes) follow the
same boundary and validate their completed values before mutation.

For TeX82 §914 `\uppercase` and `\lowercase`, `CommandProcessor` owns the
unexpanded balanced-text scan, including its opening-brace backup and
absorbing collection episode. Replay applies the selected current `\uccode`
or `\lccode` table only to character tokens, retaining each token's original
category and origin, leaving control-sequence and parameter tokens untouched,
and leaving a zero table entry unchanged. The resulting immutable list is a
typed stored command replay level, so normal command-owned retirement occurs
before the enclosing source resumes; consequently definitions in the shifted
text are defined only when that replay reaches ordinary main control.

Likewise, replay publishes a typed `\halign` or `\valign` begin observation
immediately after applying its executor-selected alignment transition. Command
state supplies the committed alignment identity and `align_state`; the next
canonical raw fetch then retires any exhausted scanner backup. This preserves
the TeX82 order without exposing a second input path to the executor.

Named glue parameters such as `\tabskip` follow the same replay gate: command
processing consumes their optional equals sign and ordinary or mu-glue operand
as one typed assignment, preserving scanner backup and alignment delivery
ordering. The replay adapter then interns and assigns that completed glue value
through `Universe`; it never leaves an operand for later main control or scans
inside an alignment preamble.

Token-register assignments use the same ownership rule through
`CommandProcessor::scan_token_register_assignment`: it scans the bounded
eight-bit register integer and optional equals sign, then probes the RHS as
TeX82 does. An internal token-list value (`\toks<n>`, a token-register
shorthand, or a token parameter) is copied directly; otherwise the rejected
token is backed up and unexpanded `scan_toks` collects balanced text. The
bounded-index scanner observation or optional-equals backup and absorbing
scanner-status transition therefore precede replay's committed register
mutation, which is emitted only after `Universe` applies the frozen list.

TeX82 `\let` and `\futurelet` likewise cross replay as completed typed
meaning assignments. `CommandProcessor` owns their raw target and source
deliveries, optional-equals handling, and `\futurelet`'s two-token lookahead
the canonical driver; `MainControl` applies the captured meaning only after that
processor borrow ends and then publishes the committed meaning mutation.

TeX82 §1224 `\chardef` and `\mathchardef` use that same completed-definition
boundary. `CommandProcessor` owns the raw control-sequence target, optional
equals sign, and complete integer scan. `MainControl` selects the
effective `\global`/`\globaldefs` scope for TeX82's provisional
`\relax`, validates the eight- or fifteen-bit code, emits the recoverable
restricted-code diagnostic where required, and commits the resulting character
or math-character meaning without raw input.

TeX82 §1224 `\countdef`, `\dimendef`, `\skipdef`, `\muskipdef`, and
`\toksdef` follow the same split. `CommandProcessor` installs the scoped
provisional `\relax`, then owns target delivery, optional-equals handling,
and the `scan_eight_bit_int` register selector. `MainControl` maps
the completed selector to the corresponding named register meaning under the
effective `\global`/`\globaldefs` scope. Those meanings re-enter the same
typed count, dimension, glue, muglue, and token-list assignment scanners as
their primitive forms; a named token register therefore uses the shared
optional-equals and RHS collection path.

TeX82 `\hrule` and `\vrule` cross the same gate as completed
`ScannedRuleSpec` values. `CommandProcessor::scan_rule_spec` owns every
expanded `width`, `height`, and `depth` keyword and dimension scan, including
the failed-keyword backup that begins subsequent main control. Replay only
appends the resulting rule node; it neither reads a source token nor rebuilds
rule provenance. This remains true inside alignment cells, where template
delivery and rule scanning share the one command-owned input stream. `\vrule`
in math mode (`mmode+vrule`) takes this same completed-spec path, since TeX82
§1056 treats it as an ordinary direct contribution. `\hrule` in math mode
(`mmode+hrule`) never reaches `scan_rule_spec`: `tex-exec` recognizes the
mode before scanning and calls `CommandProcessor::recover_missing_math_shift`
instead, TeX82 §1047's `insert_dollar_sign` recovery from the §1046
"math-only cases in non-math modes, or vice versa" list. That method owns the
same two-backup shape as §1095's `head_for_vmode` above -- back up the
offending command, then back up a synthesized `$` with inserted ownership --
so the next two deliveries close math and replay `\hrule` in the resulting
mode; the executor applies only the "Missing $ inserted" diagnostic text.

TeX82 `\setbox` follows the same split in two phases. `CommandProcessor`
scans the register integer and optional equals sign as a typed
`ScannedSetBoxAssignment`, including the canonical backup of the equals
delivery. The following `\vbox` remains an ordinary command delivery; the
processor scans its required opening brace, and replay opens, packages, and
assigns the executor-owned box group.
This keeps box construction from acquiring a raw-input API while retaining the
observable scanner and backup ordering.

For `\hbox`, `\vbox`, and `\vtop`, command processing owns all of TeX82
§645's `scan_spec` -- the optional `to`/`spread` packing clause and dimension,
and then the mandatory opening brace, which §403's `scan_left_brace`
_consumes_. §774's `init_align` calls the same `scan_spec`, so `\halign` and
`\valign` run the identical clause through the identical routine; the value
it scans is what §805 packages the preamble prototype box with
(`hpack(preamble, saved(1), saved(0))`). That brace is never redelivered to main control: `scan_spec` runs
`new_save_level(c)` before it, so the group it opens is exactly the one replay
enters when it receives the construction. Replay enters the typed group and
mode, schedules the matching immutable `\everyhbox`/`\everyvbox` command
episode (§1083 begins that token list after the brace is gone), and applies
pure packing only after the body closes; scoped `\setbox` assignment then
occurs at the same aggregate boundary. An active box body therefore owes no
opener: the body's own closer is exactly the brace delivered while the
innermost group is still the group `scan_spec` opened. Braces _inside_ the
body are ordinary `simple_group` levels (§1063's `non_math(left_brace):
new_save_level(simple_group)`), closed by §1069's `simple_group: unsave` like
any other group; replay keeps no separate brace-depth count, because §1068's
`handle_right_brace` dispatches purely on `cur_group` and the save stack
already records every open level.
If the opener is malformed, §403 recovers by backing up the rejected command
and proceeding as though a brace had been read; replay enters the typed box
group without fabricating a command event, so that backed-up command becomes
the first box-body material.

TeX82 §1099's `\insert<class>{...}` (`begin_insert_or_adjust`) follows the
same box-opener split, minus the packing clause: `CommandProcessor` owns the
raw `scan_int` class-number scan (any_mode; the reserved-255 rejection and the
ordinary 0..=255 range clamp both write a `Universe` diagnostic, so they are
deferred to replay) and the same mandatory-opening-brace scan
`\hbox`/`\vbox`/`\vtop` use (§1099's own `new_save_level(insert_group);
scan_left_brace`). Replay enters `GroupKind::Insert`/internal
vertical mode, applies §1099's `normal_paragraph` reset, and reuses the box
family's body-closing bookkeeping (`active_boxes`/`BoxEndGroup`) purely to
recognize its own closing brace -- an insertion body is not a box and
schedules no `\everyhbox`/`\everyvbox` hook. Its closing action is a
dedicated branch (`finish_insert_or_adjust_group`): §1100's `end_graf`, then
TeX82's `vpack` macro (unconstrained depth, but the box's _current_
`\vbadness`/`\vfuzz`, unlike an ordinary `\vbox`) packages the body, and the
resulting `ins_node` is appended to whatever list was open when `\insert`
began -- not a side channel -- exactly like `\mark`/`\penalty`. Outer vertical
mode then invokes `build_page`, which owns §§980--987's insertion-class
splitting and height accounting (`tex-exec::page_builder`) once the node
reaches the page contribution list.

`\vadjust{...}` (the exact same `begin_insert_or_adjust`/`handle_right_brace`
procedures as `\insert` above, §1099/§1100) shares that entire construction
rather than duplicating it: tex.web's own
`begin_insert_or_adjust` sets `cur_val:=255` directly for `\vadjust` instead
of calling `scan_eight_bit_int`, so both reach one
`scan_insert_construction(is_vadjust)` that builds the same
`ScannedInsertConstruction{class: 255, ..}` used for `\insert`, with a
dedicated `is_vadjust` flag telling replay to skip the 0..=255 clamp and the
"you can't `\insert255`" rejection -- both diagnostics are specific to a
user-scanned class and would otherwise misfire on `\vadjust`'s always-valid
255 sentinel. `\vadjust` in (outer or internal) vertical mode is one of
tex.web's "Forbidden cases" (`ColdOperation::IllegalInsertOrAdjust`, sharing
`report_illegal_case` with `IllegalBoxShift`/`IllegalItalicCorrection`) rather
than `any_mode` like `\insert`. `finish_insert_or_adjust_group` branches on
the completed class: 255 builds a bare `adjust_node` (`Node::Adjust`,
carrying only the packed content -- `\splittopskip`/`\splitmaxdepth`/
`\floatingpenalty` are still read at the same point as `\insert`, mirroring
tex.web's unconditional read before the branch, but never stored) instead of
an `ins_node`. Splice-back out of an enclosing `\hbox`/paragraph line is not
new architecture: it already existed as a general `Node::Mark`/`Node::Ins`/
`Node::Adjust` migration mechanism (`extract_box_migrations` in
`tex-exec::assignments::boxes`, and `extract_migrating_material` in
`tex-exec::assignments::paragraph`, mirroring tex.web's `hpack`
`adjust_tail` and `post_line_break`), so wiring the canonical primitive was
sufficient to exercise it correctly with no further changes.

pdftex.web §§1275--1276 extend that construction in the pdfTeX profile: an
optional `pre` keyword is scanned before the mandatory body brace and retained
in `AdjustNode` beside the content list. The compact-node sidecar, semantic
identity, format-image DTO, survivor traversal, and diagnostics all preserve
the marker; `\showbox` prints `\vadjust pre`. Migration maintains separate pre
and ordinary adjustment streams, placing pre material before its packed hlist
or paragraph line while ordinary `\vadjust` material remains after it. TeX82
and e-TeX profiles do not probe the keyword and retain their original scanner
behavior.

`\mark{...}` (TeX82 §1101's `make_mark`) needs none of the box-opener
machinery: `CommandProcessor::scan_balanced_text(true)` (the same fully
expanded general-text scan already used for `\special`/`\message`) is the
entire command-side operand, and replay appends a class-0 `Node::Mark`
directly to whatever list is current -- `any_mode`, and unlike `\insert`,
never followed by a `build_page` call. When diagnostics later visit that node,
TeX82 §200 sends the `mark` header through §63's `print_esc`, so node-list
rendering observes the live `\escapechar` just like neighboring symbolic node
headers. The e-TeX numbered `\marks<n>{...}`
variant is not wired (tracked separately for the e-TeX phase, umber2-johp.9).

TeX82 §1073's box-shift prefixes (`\raise`, `\lower`, `\moveleft`,
`\moveright`) cross the boundary as one completed operation, unlike the
box-opener family above: `CommandProcessor::scan_box_shift` owns the mode
legality that `tex.web`'s `abs(mode)+cur_cmd` dispatch performs by construction
(`\raise`/`\lower` legal only outside vertical mode, `\moveleft`/`\moveright`
only inside it; the three complementary "Forbidden cases" combinations never
reach a scanner at all, so replay's `IllegalBoxShift` reports
`report_illegal_case` without having scanned anything), the already-signed
dimension (`\lower`/`\moveright` keep it, `\raise`/`\moveleft` negate it), and
then `scan_box`'s own operand (§1076): any `make_box` command. `\hbox`/`\vbox`/
`\vtop` reuse `scan_box_construction` and are returned as an ordinary
`ScannedBoxShiftPayload::Construction`, deferring to the same
`active_boxes`/`BoxEndGroup` machinery as `BeginBox` --
`ActiveReplayBox` carries the pending signed shift, applied to
`shift_amount(cur_box)` immediately before `BoxEndGroup`'s ordinary
(non-register, non-shipout, non-leader) append, since a box-shift's own box
can never itself be a `\setbox` target, `\shipout` operand, or leader payload
(`scan_box` requires `cur_cmd=make_box`, which `vmove`/`hmove` never are).
`\box`, `\copy`, `\lastbox`, and `\vsplit` instead resolve to a node
immediately inside `tex-command`, exactly as they do outside a shift, and
replay shifts and appends that completed value the same way `\box<n>` does
standalone. A rejected non-box operand replays `scan_box`'s own "A <box> was
supposed to be here" backup exactly like `scan_leader_payload`'s twin
recovery, leaving the diagnostic to replay since it needs a `Universe` sink.

Several no-operand primitives cross the boundary as a bare mode-classification
with no scanner of their own, since `scan_command` already has the live mode
and (for the two that need it) `command` to back up: TeX82 §1105's
`any_mode(remove_item): delete_last` (`\unpenalty`/`\unkern`/`\unskip`) is
identical in every mode, so `scan_command` returns `ColdOperation::DeleteLast`
unconditionally and replay's existing mode/list-sensitive `delete_last` helper
(shared with the legacy dispatcher) does the rest. TeX82 §1112's
`hmode+ital_corr`/`mmode+ital_corr` (`\/`) differ only in which apply-time
kern gets appended (hmode's font-metric kern vs. math's fixed zero kern, whose
subtype must stay `KernKind::Font` -- `new_kern`'s default -- rather than
`Explicit`, since only an explicit kern is a legal kern-then-glue line-break
point); §1111's "Forbidden cases" makes vertical mode `IllegalItalicCorrection`
instead, mirroring `IllegalBoxShift`'s `report_illegal_case` reuse. TeX82
§1030/§1045/§1090's `\noboundary` similarly resolves per mode at apply time
(a flag on the current list in hmode, `do_nothing` in mmode) except in
vertical mode, where `scan_command` backs the token up and returns the
existing generic `ColdOperation::ParagraphStart` exactly as `\hskip`/`\accent`
already do. TeX82 §1171's `mmode+non_script` (`\nonscript`) and §1046's
`non_math(non_script)` recovery for every other mode reuse
`CommandProcessor::recover_missing_math_shift` unchanged -- the same generic
`insert_dollar_sign` call already used for `\vskip` reached in math mode --
proving that helper is not `\vskip`-specific. TeX82 §1045's
`any_mode(ignore_spaces)` needs no apply-side step at all, and no backup
either -- see §33.7.

TeX82 §1046's `non_math(...)` table is _not_ primitive-shaped throughout, and
reading it as though it were is what left `math_given` with no math-mode
dispatch at all (umber2-johp.194). Three of its members carry no
`UnexpandablePrimitive`: `non_math(sup_mark)` and `non_math(sub_mark)` are
character categories, and `non_math(math_given)` is a `\mathchardef` target,
whose math code lives in the delivered `Meaning::MathCharGiven` itself.
`scan_math_request` is therefore keyed on the delivered `Meaning`,
not on the primitive, so `math_given` joins `math_char_num` in the same
§1154/§1155 `set_math_char` path instead of falling through to a loud
`UnimplementedMeaning`.

Text `\accent` has one additional ordering constraint in math mode. TeX82
§1110 calls `error` while the delivered `accent` command is still current,
then continues into §436's `scan_fifteen_bit_int`; `\mathaccent` starts with
that scan directly. The typed math request therefore leaves the text-accent
character pending until the executor has rendered §82's context and emitted
the diagnostic. This keeps an exhausted backup level visible as
`<recently read> \accent` instead of retiring it while scanning the operand.

The rest of the table is the math-noad, math-style, and math-delimiter
primitive family that `scan_math_request` (§5's math-request
vocabulary above) and the `\left`/`\right`/`\middle` gate otherwise dispatch
only under `Mode::Math`/`Mode::DisplayMath`: `\mathchar`,
`\delimiter`, the eight `mathord`/`.../mathinner` component primitives plus
`\underline`/`\overline`, `\left`/`\right`/`\middle`, the six `\above`/`\atop`/
`\over` (with and without delimiters) fraction primitives, `\radical`, the
four style primitives, `\mathchoice`, `\vcenter`, `\mkern`, the three
`\limits`/`\nolimits`/`\displaylimits` switches, `\mskip`, and `\mathaccent`
(`\nonscript` above is one member of this same family, not a special case).
`scan_unclassified_primitive`'s fallback match expresses that primitive part
of the table as one grouped arm -- not 34 duplicated ones -- calling the same
`recover_missing_math_shift` used by `mmode+hrule`/`mmode+vskip`/
`non_math(non_script)`, since every member reaching that fallback has already
proven `mode` is not math (the math-mode dispatch above would have consumed
it first). `\eqno`/`\leqno` are deliberately excluded even though tex.web
registers them under the same `eq_no` command code as the math-request
vocabulary: TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)` (added to the
shared Forbidden-cases list first built at §1048) routes vmode/hmode
`\eqno`/`\leqno` through the same `report_illegal_case` (``You can't use
`\eqno' in ... mode``) already reused by `IllegalBoxShift`/
`IllegalItalicCorrection`/`IllegalInsertOrAdjust`, via their own dedicated
`ColdOperation::IllegalEqNo` arm (umber2-johp.88), not `insert_dollar_sign`.
`mmode+eq_no` itself (gated by `privileged`/`cur_group`, §§1140-1142) is
unaffected. This is umber2-johp.79's generalization of umber2-johp.56's
original `mmode+hrule` mechanism to the entire §1046 table.

TeX82 §1264's `new_interaction` (`\batchmode`/`\nonstopmode`/`\scrollmode`/
`\errorstopmode`, any_mode via §1210's `set_interaction`) sets `interaction`
directly from the delivered primitive's own `chr_code`, with no operand scan
of its own -- the same no-scan shape as `\unpenalty`/`\unkern`/`\unskip`'s
`ColdOperation::DeleteLast`. `interaction` is a plain global Pascal variable
outside `eqtb`, so this assignment ignores `\global`/`\globaldefs` and is
never undone by group exit, unlike an ordinary parameter assignment
(umber2-johp.83).

`\globaldefs` itself was already a plain `Meaning::IntParam` assignment (the
generic `Meaning::IntParam(index)` scan/apply arms, shared with every other
integer parameter) rather than a distinct `UnexpandablePrimitive` variant, so
its own value was already written correctly. The actual defect was that two
of canonical replay's assignment apply arms -- `ColdOperation::MacroDefinition`
(`\def`/`\edef`/`\gdef`/`\xdef`) and `ColdOperation::Let` (`\let`/
`\futurelet`) -- used their raw `\global` prefix bit directly instead of
resolving it through `assignment_global` (the shared helper every other
assignment kind, including `\count`/`\dimen`/`\toks`/code-table/
`\countdef`-family/`\chardef`-family assignments, already calls), so a
nonzero `\globaldefs` silently had no effect on macro or `\let` scope --
precisely the write-that-lands-nowhere pattern this epic's exhaustiveness
work exists to surface (umber2-johp.83). `UnexpandablePrimitive::GlobalDefs`
is a distinct, currently dead enum variant: no primitive name registers it,
so it can never actually reach `scan_unclassified_primitive`'s `Err` bucket
in production.

TeX82 §1090's vmode-paragraph-starting list (`back_input; new_graf(true)`)
also includes `vmode+un_hbox` and `vmode+valign` -- unlike `vmode+un_vbox`
and `vmode+halign`, which are not in that list and legitimately act directly
in vertical mode. A bare `\unhbox`/`\unhcopy` or `\valign` reached in
`Mode::Vertical`/`Mode::InternalVertical` therefore backs up the token and
starts a paragraph before its operand (the box register, or the alignment
preamble) is ever scanned, exactly like `vmode+math_shift` and
`vmode+no_boundary` (umber2-johp.87).

### 5.4 Proposed module layout

```text
crates/tex-command/src/
    lib.rs
    profile.rs
    state.rs
    command.rs
    error.rs

    input/
        mod.rs
        source.rs
        lines.rs
        tokenizer.rs
        levels.rs
        stack.rs

    processor/
        mod.rs
        next.rs
        expand.rs
        status.rs
        alignment.rs

    macro_call.rs
    conditionals.rs
    scan_toks.rs

    scanners/
        mod.rs
        scalar.rs
        structured.rs
        token_list.rs
        font.rs
        hyphenation.rs
        restricted.rs
        expression.rs

    primitives/
        mod.rs
        catalogue.rs
        metadata.rs
        primitive_metadata.rs
        generated.rs
        parameters.rs
        registry.rs
        prefixed.rs

    observation/
    snapshot.rs

    tests/
```

Files should remain organized around canonical state machines. Mechanical
splitting is not a substitute for ownership separation.

The integrated primitive catalogue is the exhaustive authority for enum-backed
commands, parameter cells and defaults, profile availability, aliases,
canonical observation identities, page/internal quantities, `nullfont`, and
the store-local frozen `endwrite` meaning. Fresh INITEX installation,
format-load registry reconstruction, tracing names, pdfTeX setup, exact profile
name sets, and documentation consume its generated views. Restoration registers
frozen meanings without replacing format-shadowed live meanings. Handwritten
dispatch remains the behavioral authority.

## 6. Engine and character profiles

Command behavior is selected by an immutable profile installed before job
start:

```rust
pub struct CommandProfile {
    pub dialect: CommandDialect,
    pub characters: CharacterMode,
}

pub enum CommandDialect {
    TeX82,
    ETeX26,
    PdfTeX14027,
}

pub enum CharacterMode {
    EightBitExact,
    UnicodeExtended,
}
```

The exact compatibility profiles are:

```text
TeX82         + EightBitExact
e-TeX 2.6     + EightBitExact
pdfTeX 1.40.29 + EightBitExact
```

The command profile identifies the loaded format's command family and frozen
state, but it does not erase the canonical engine implementation executing
that state. `CommandEngineSemantics` records that second immutable job fact.
Thus a pdfTeX 1.40.29 binary loading a TeX82 format keeps the TeX82 profile and
format fingerprint while shared compiled routines retain pdfTeX behavior, such
as pdftex.web §459's `nd`/`nc` invalid-unit help. A genuine TeX82 engine keeps
tex.web §459's original wording. `tex-exec::MainControl::set_engine_binary`
installs both the framing identity and this command-semantic implementation;
the newer implementation must support the loaded profile.

`UnicodeExtended` is an Umber extension. It may combine e-TeX or pdfTeX
command families with Unicode input and sparse code tables, but it must have a
distinct engine identity and format fingerprint. Its behavior is not described
as byte-for-byte TeX82 or pdfTeX input compatibility.

### 6.1 Character codes

The semantic character type is not Rust `char`:

```rust
#[repr(transparent)]
pub struct CharacterCode(u32);
```

`CharacterCode` uses a private tagged `u32` representation. Exact bytes and
Unicode scalars are different semantic domains even where their numeric values
overlap: byte `0x41` is not scalar `U+0041`. Stable four-byte encoding preserves
that distinction, rejects noncanonical byte encodings and invalid Unicode
scalars, and is independent of host text encoding.

In `EightBitExact`, valid input character codes are `0..=255`, physical input
is consumed byte-for-byte, and `^^` notation follows the selected canonical
engine. In `UnicodeExtended`, valid values are Unicode scalar values and the
input decoder must preserve exact byte ranges for provenance. Canonical source
tokens additionally retain half-open decoded-scalar positions within their
normalized physical line; a synthetic `endlinechar` occupies one scalar
position while retaining a zero-width physical byte range. Every token a
catcode-5 character produces -- the `mid_line` space, the `new_line` `\par`,
and nothing at all in `skip_blanks` -- is anchored at TeX82's `buffer[limit]`
rather than at the character that triggered it, because tex.web §348, §350,
and §351 each finish the line with `loc:=limit+1`. With an active
`\endlinechar` that anchor is the zero-width synthetic endline position after
the retained prefix; with `\endlinechar` inactive tex.web §362 decrements
`limit`, so the anchor becomes the line's last retained character. Physical
terminator bytes are never claimed by a token. An unterminated final line
remains zero-width.

Each direct source delivery also carries a typed canonical location distinct
from its raw span. It is the physical source column of the final byte the
spelling consumed: ordinary one-byte spellings therefore locate at their span
start, while a decoded `^^41` retains its full four-byte raw span but locates
at the final `1`. Zero-width synthetic spellings retain their physical anchor.
This provenance pair is ordinary snapshot-owned input state, including backup
replay; it is not reconstructed by fixture observation.

TeX82 observes that column as `loc - start - 1`, and the two agree only while
`buffer` still mirrors the source line. tex.web §355 reduces an expanded code
inside a control-sequence name _in place_, writing the decoded character over
`buffer[k-1]` and shifting the remainder of the line down by `d` (two or
three) while decrementing `limit` and `first`; from that point on every
`buffer` index on that line is `d` smaller than the source column it came
from, and the deficit accumulates across further reductions on the same line.
tex.web §352's character-path reduction has no such effect: it only advances
`loc` past the whole `^^` form. The canonical location is always the source
column, so the instrumented oracles accumulate each line's collapsed bytes
(`umber_line_shift`, stacked per input level alongside `line_stack`) and add
them back before emitting `byte`. A location that shifted because of an
unrelated earlier `^^` control sequence on the same line would not be
provenance at all.

Control-sequence names are semantically sequences of `CharacterCode`. Their
storage may use a compact UTF-8 representation when lossless for the active
profile, but string encoding is an implementation detail. Name identity,
`\string`, `\meaning`, format identity, and diagnostics must operate on
canonical character codes.

Code-table queries use `CharacterCode` and the active profile. A profile
conversion cannot occur during a job.

The implemented immutable `CommandProfile` has exact TeX82, e-TeX 2.6, and
pdfTeX 1.40.29 eight-bit constants plus explicit Unicode-extension
construction. Dialect facilities (e-TeX and pdfTeX families) and Unicode
semantics are derived capabilities, not host capabilities. Its versioned
stable bytes feed a fixed domain-separated FNV-1a-64 profile fingerprint;
format and checkpoint identities compose that fingerprint.

## 7. State taxonomy

The design distinguishes five state classes:

| Class                       | Examples                                                            | Snapshot rule                                                  |
| --------------------------- | ------------------------------------------------------------------- | -------------------------------------------------------------- |
| Semantic command state      | input cursors, macro activations, conditions, `align_state`         | captured and restored                                          |
| Aggregate engine state      | meanings, registers, code tables, fonts, World stream state         | owned and snapshotted by `Universe`                            |
| Call-local semantic state   | `CurrentCommand`, local scanner accumulator, expansion budget scope | absent at durable boundaries; replayed from the enclosing step |
| Diagnostic/provenance state | source map, origins, macro invocation DAG                           | rollback-coupled but excluded from semantic equality           |
| Discardable acceleration    | measured future caches or buffer pools                              | may be dropped without changing behavior                       |

No type may mix fields from these classes merely because they are used by the
same procedure.

The command state exposes one content-free failure projection to main control:
the live input-level count and an innermost-first tail of at most eight level
families (`source`, macro body/argument, backup, inserted, alignment template,
or generic stored/transient replay). It exposes no cursor, token, source name,
line text, macro argument, or replay handle. Main control samples this only
after an error and couples it to the aggregate rollback root; it is diagnostic
metadata, not a second input consumer or durable command summary.

## 8. Persistent command state

The future-relevant command-machine state is one ownership unit:

```rust
pub struct CommandState {
    input: InputState,
    parameters: ParameterState,
    scanner: ScannerState,
    conditions: ConditionStack,
    alignment: AlignmentDeliveryState,
    expansion: ExpansionState,
    transient: TransientState,
}
```

This is the command owner for direct execution. It is never independently
restored against a different `Universe`, mode nest, execution state, effect,
generated-write, or output timeline. Main control settles expansion and
scanning before semantic apply. Resource suspension retains an explicit typed
continuation over the exact activation, argument, input, and provenance owners
already consumed; it does not clone or rewind aggregate command state.

### 8.1 Input state

`InputState` owns actual input levels and source-id allocation. It does not own
condition frames, meanings, scanner policy, or host callbacks.

### 8.2 Parameter state

`ParameterState` owns the live macro-argument activation chain. Its Rust
representation may attach arguments directly to macro input levels, but it
must implement the same ownership semantics as TeX's `param_stack` and
`param_start`.

### 8.3 Scanner state

`ScannerState` owns `scanner_status`, warning identity, and the typed handle
needed to describe incomplete input. Each non-normal context owns its warning
identity, and a scoped installation restores the complete previous state; no
scanner caller reconstructs warning ownership after nested work. The detached
observer records each actual typed `from`/`to` transition, so a macro matcher
entered during an expanded definition reports `defining -> matching` rather
than flattening the enclosing scanner scope to `normal`. Terminal EOF
is classified centrally as legal or as one typed runaway family. A physical
source EOF invokes that classification immediately after its own input level
retires, before raw delivery resumes a parent source or token list. That
decision does not inject recovery tokens; command-owned outer-validity
recovery consumes it at that boundary.

`CommandProcessor` owns one `ScannerEpisode` lifetime mechanism above that
state. It installs the typed status and warning together, applies the explicit
observer-visibility policy, reasserts an enclosing collector after nested
outer-validity recovery, and publishes the recovery-aware exit before
restoring the complete prior state. Balanced token collection, `read_toks`,
macro matching, conditional skipping, temporary-normal operand reads, and
alignment preamble scanning all use that seam rather than pairing status and
warning mutations by hand. e-TeX general-text recursion selects the hidden
visibility policy explicitly; it still receives the same live absorbing
recovery semantics.

If outer-validity recovery clears a non-normal episode before its scoped
caller returns, that caller publishes the installed-status-to-prior-status
transition (rather than a misleading `normal -> normal`) and then restores the
complete prior scanner state. This applies uniformly to `scan_toks`, macro
matching, and skipped-condition scanning. Alignment preamble recovery is the
exception: it retains `aligning` until the recovered frozen `\cr` completes
the typed preamble scan.

```rust
pub enum ScannerStatus {
    Normal,
    Skipping(SkippingContext),
    Defining(DefinitionContext),
    Matching(MatchingContext),
    Aligning(AlignmentScanContext),
    Absorbing(AbsorbingContext),
}

pub struct DefinitionContext {
    target: Option<Symbol>,
    builder: TokenBuilderId,
    warning: ScannerWarning,
}
```

Status installation and restoration use one scoped mechanism so every success,
recoverable error, hard error, and resource suspension restores the canonical
outer status.

### 8.4 Condition state

`ConditionStack` is independent of input levels and the environment save
stack. It owns the typed equivalents of `cond_ptr`, `if_limit`, `cur_if`, and
`if_line`. Stable condition identities permit an outer condition's limit to be
updated after recursive operand expansion pushes a newer condition.

### 8.5 Alignment delivery state

`AlignmentDeliveryState` owns the only `align_state`, nested suspension stack,
active cell delivery, and exact u/v-template identities. Execution modes do
not keep a second brace-depth counter.

`align_state` is a whole-run brace count, not a per-alignment counter: TeX82
§331 starts it at `1000000`, and §772's `push_alignment`/`pop_alignment` save
and restore it around _every_ alignment, nested or not. `begin_alignment`
therefore pushes the running count onto the `align_stack` before §774 sets
`-1000000`, and `finish_alignment` pops it back. Keeping the outer count is
what lets §1127's `abs(align_state) > 2` still mean "no alignment entry is in
progress" for material that follows a completed `\halign`; resetting the count
to zero there would make the next stray `\cr` insert a brace instead of being
reported and dropped. `suspend_alignment`/`resume_alignment` consequently save
only the outer cell, never a second copy of `align_state`.

### 8.6 Expansion state

`ExpansionState` owns only persistent expansion facts:

- cumulative job-level expansion accounting that can affect a future result;
- deterministic resource-resolution order;
- recoverable diagnostics awaiting executor delivery;
- the active profile;
- expansion-derived dependency-recording state; and
- semantic barriers needed by incremental reuse.

Per-request expansion fuel is call-local but shared by nested expansion within
that request. A resource retry restarts the complete executor step and
therefore recreates the same budget deterministically.

### 8.7 Transient state

`TransientState` owns builders referenced by live scanner status and rollback
roots for temporary command data. The contents of a live builder are semantic
until its enclosing call completes or rolls back. Discardable pooled buffers
remain process-local and outside this snapshotted state.

## 9. Discardable runtime state

Acceleration does not live in `CommandState`. One process-local authority owns
a typed, bounded traced-token pool.

The `Arc` owns a mutex-protected array of exactly two optional
`Vec<TracedTokenWord>` values. Checkout takes the first available vector or
creates an empty one; the returned RAII guard exclusively owns that vector
while it is checked out. Guard destruction clears the vector on success and
error paths. It returns the empty allocation only when its capacity is at most
4,096 token words and a slot is empty; a larger allocation or a third returned
buffer is dropped. The pool therefore retains at most two empty vectors and at
most 8,192 token words of total capacity. An outstanding private guard keeps
the pool allocation alive through its cloned `Arc`, but never enters semantic
state.

The pool is used solely for copied-before-return collectors in balanced and
macro-definition `scan_toks`, `\read` line collection, and output replay
expansion. Before the guard returns, those paths copy or intern the live token
contents into semantic ownership. Macro arguments, shift-case output,
alignment columns and templates, glue specs, character and name vectors,
pattern data, and every other element type are excluded. The pool is absent
from snapshots, summaries, equality, and hashing; step rollback neither
captures nor reconstructs it. Scratch warmth cannot affect semantic events,
diagnostics, effects, output, snapshots, or summaries. Further discardable
acceleration belongs here only after measurement and with canonical identity
plus exact generation or content guards.

The input stack does not carry meaning-cache state or expansion-policy bits.

## 10. Command processor

`CommandProcessor` is an ephemeral capability facade:

```rust
pub struct CommandProcessor<'episode, 'admission> {
    command: &'episode mut CommandState,
    state: &'episode mut tex_state::CommandContext<'admission>,
    host: CommandHostContext<'episode>,
    observer: Option<&'episode mut dyn CommandObserver>,
}
```

It does not own state and cannot outlive one bounded executor operation. The
executor admits one call-local `CommandContext`, refreshes its transient mode
and page capabilities through a borrow of that value, and lends the same
context in place to the processor. Processor retirement ends the borrow; it
does not move the complete admitted context out of and back into a facade.
`CommandHostContext` contains only the capabilities installed for that
operation, such as input resolution and optional read recording. Host
capabilities never enter snapshots or formats.

Production methods include:

```rust
impl CommandProcessor<'_> {
    pub fn get_next_into(
        &mut self,
        destination: &mut Option<CurrentCommand>,
    ) -> Result<DeliveryStatus, CommandError>;
    pub fn get_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand>,
    ) -> Result<DeliveryStatus, CommandError>;
    pub fn get_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand>,
    ) -> Result<DeliveryStatus, CommandError>;
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand>, CommandError>;
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand>, CommandError>;
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError>;
    pub fn get_x_or_protected(
        &mut self,
    ) -> Result<Option<CurrentCommand>, CommandError>;
    pub fn back_input(&mut self, command: CurrentCommand);
}
```

Macro calls, conditions, scanners, and primitives are crate-private operations
over the same processor. They do not receive separate mutable input, state,
expansion, alignment, or policy objects.

## 11. Semantic token and current command

Tokens and commands are distinct.

### 11.1 Token word

`TokenWord` is a compact immutable spelling:

```rust
pub enum TokenWord {
    Character {
        code: CharacterCode,
        catcode: Catcode,
    },
    ControlSequence(Symbol),
    OutParameter(u8),
    Frozen(FrozenToken),
}
```

The physical packed representation remains private. Semantic equality ignores
provenance and delivery identity.

### 11.2 Traced token

Production input always carries provenance:

```rust
#[repr(transparent)]
pub struct TracedTokenWord(u64);
```

An untraced production interpreter is not maintained. Tests that need a
synthetic unknown origin use `OriginId::UNKNOWN`.

### 11.3 Meaning

Meanings preserve TeX's command/modifier model in typed form:

```rust
pub struct Meaning {
    command: CommandCode,
    operand: CommandOperand,
}
```

The hot representation may be a packed word. The semantic API remains typed.
Expandable versus unexpandable classification is a property of
`CommandCode`.

### 11.4 Current command

`CurrentCommand` is the value equivalent of TeX's `cur_cmd`, `cur_chr`,
`cur_cs`, and `cur_tok`:

```rust
pub struct CurrentCommand {
    spelling: TracedTokenWord,
    meaning: Meaning,
    control_sequence: Option<Symbol>,
    delivery: DeliveryStamp,
}
```

The spelling and effective meaning may differ for the one delivery suppressed
by `\noexpand`. `DeliveryStamp` proves the exact live input level and position
that delivered the token. It is ephemeral, excluded from summaries, and never
reconstructed from semantic token equality.

Meaning resolution occurs once at raw delivery. An ordinary character resolves
to its literal character-token meaning, while a control sequence reads its
current meaning cell. An active character first resolves to its distinct
active-character control-sequence identity and then reads that cell; it never
aliases an escaped control sequence with the same printed spelling. The
resolved value is retained in `CurrentCommand`, so later assignments cannot
alter an already delivered command. Test and instrumentation observation
translates TeX82 conditional primitives from their Rust meaning variants to
the shared `if_test` command identity and canonical `cur_chr` operand (for
example, `\iftrue` is operand 14), and maps the raw delimiters `\fi`,
`\else`, and `\or` to `fi_or_else` with operands 2, 3, and 4 respectively.
It likewise projects the horizontal glue family as `hskip` and the vertical
family as `vskip`, preserving TeX82's `cur_chr` selector rather than leaking
the executor's shorthand representation: `\hfil`/`\vfil` are 0,
`\hfill`/`\vfill` are 1, `\hss`/`\vss` are 2,
`\hfilneg`/`\vfilneg` are 3, and `\hskip`/`\vskip` are 4. This follows
TeX82 §15's command-code definitions and §18's primitive initialization.

Executor-owned references to an _original_ immutable primitive use a packed
`PrimitiveHandle<G>` issued only after the driver has completed INITEX
installation or loaded-format registry reconstruction. The handle contains a
direct registry row plus its completed registry extent and is typed by the
engine generation. Resolution validates that extent and indexes the immutable
row directly; it never reads or caches the mutable eqtb meaning cell carrying
the same spelling. Consequently `\def`, `\let`, and primitive shadowing retain
their ordinary command-delivery semantics. These handles are process-local
accelerators: command snapshots, formats, detached continuations, and semantic
hashes contain none.

Likewise TeX82 §53 registers `\openout`, `\write`, `\closeout`, `\special`,
`\immediate`, and `\setlanguage` as the shared `extension` command with
operands 0 through 5. The command core emits those raw identities. For
`\immediate`, it also owns §53's recursive `get_x_token` lookahead, the
integer/optional-equals/filename scan for `\openout`, and backup of every
other expanded token. An immediate `\write` first freezes its ordinary
unexpanded general text, then command control replays that list between
synthetic braces and the inaccessible outer `\endwrite` stopper before an
expanded `scan_toks` collection. The stopper retires without reading the
following source token; this preserves TeX82's raw/expanded deliveries,
input lifecycle, and detached write observation. The §53 scanner registers
the replay level's observer identity when it pushes that list; raw delivery
consumes only that identity at retirement and never derives observation from
replay trace/provenance. It returns only the final
typed immediate-effect request; the executor applies that request without
access to live command input. This metadata does not participate
in execution dispatch.
This metadata does not participate in conditional evaluation or condition-stack
state. Its `spelling` retains the traced token,
and `origin` is exposed directly from that spelling; provenance never affects
the meaning lookup. Engine-owned frozen tokens resolve through their immutable
frozen meaning rather than a mutable control-sequence cell.

Condition-stack observations are likewise a projection, not condition state:
each push, limit change, branch, and pop carries the canonical TeX condition
name and `if_limit` name at that seam (and a branch name where applicable).
The observer keeps a private frame identity only for host diagnostic context;
it never enters the portable oracle event. Branch observations retain the
pre-change limit, and when a true-limb `\else` skips to its matching `\fi`,
the `fi` branch is recorded before the independent frame is popped, matching
the TeX82 trace.

No `CurrentCommand` is live at a durable named checkpoint.

## 12. Input levels

Input is a dense stack:

```rust
enum InputLevel {
    Source(SourceLevel),
    Tokens(TokenCursor),
}
```

Conditions are not input levels. Profiling and paragraph-transition state do
not alter the level enum.

### 12.1 Source cursor

Opening an input resolves host policy once into immutable registered backing:

```rust
struct SourceCursor {
    source: SourceId,
    backing: RegisteredBacking,
    next_physical_offset: u64,
    line: SourceLineState,
    lexer: LexerState,
    end_after_line: bool,
    trace: SourceTrace,
}

struct SourceLevel {
    frame: PackedInputFrame,
    cursor: SourceCursor,
    open_depths: Option<Box<SourceOpenDepths>>,
    // source classification and cold diagnostic state
}
```

`RegisteredBacking` refers to World input, generated immutable bytes, a
registered editor fragment layout, or an explicitly typed read-line source.
Ordinary line refill does not call host file search. Dynamic dispatch, if
retained, is confined to cold source acquisition or physical-line refill and
does not enter character delivery.

`SourceLineState` stores one canonical byte cursor and enough physical metadata
to map normalized characters to immutable backing ranges. There is no parallel
mutable character-index representation.

The implemented registration boundary accepts only complete, already acquired
immutable bytes and records whether they came from World, generated input, an
editor fragment, or typed read-line acquisition. Registration allocates the
stable `SourceId`; Unicode mode validates the complete UTF-8 backing before
allocation and reports the exact malformed byte range, while exact mode never
decodes or rewrites bytes. Opening a registered source consumes its one-shot
pending backing into the live source level and therefore cannot invoke host
policy. A `SourceId` cannot be reopened; provenance source-map registration is
independent and retains the exact ID after the source level retires.

Physical refill distinguishes LF, CR, CRLF, and a missing final terminator.
A final terminator does not manufacture another empty line. Normalization
removes trailing byte `0x20` values, captures the current profile-valid
`endlinechar`, and delivers that synthetic character at the zero-width anchor
after the retained prefix. Unicode scalar delivery advances the same canonical
byte cursor by the UTF-8 width and retains a scalar delivery offset; exact
eight-bit delivery advances it by one. In both modes ordinary character ranges
address only immutable physical bytes, and terminator and stripped-space ranges
remain available as physical metadata without being claimed by tokens.

Comments and catcode-5 characters share one line-abandonment mechanism,
tex.web's `loc:=limit+1`: the remainder of the line is skipped, but the line
stays loaded and is retired only on the following step, at §343's `loc>limit`
branch. That is the branch §362 uses to observe `\endinput`, so an explicit
`^^M` or a `%` comment on the same line as `\endinput` still retires the
source. An explicit catcode-5 character therefore ends its line exactly as
the synthetic `endlinechar` does.

### 12.2 Token cursor

Span ownership, semantic delivery behavior, retirement, and trace explanation are
orthogonal:

```rust
struct TokenCursor {
    span: PackedTokenSpanHandle,
    behavior: TokenBehavior,
    retirement: RetirementBehavior,
    trace: ReplayTrace,
    frame: PackedInputFrame,
}

enum PackedTokenSpanHandle {
    Replay { replay: ReplayPayloadId, len: u32 },
    MacroReplacement {
        definition: MacroDefinitionId,
        len: u32,
    },
    MacroArgument { range: MacroArgumentRange, len: u32 },
    DurableList { cursor: TokenListCursor, len: u32 },
    AttemptList { list: AttemptTokenListId, len: u32 },
}

enum TokenBehavior {
    Ordinary,
    Recovery,
    MacroBody(MacroActivation),
    Parameter,
    BackedUp(BackupTreatment),
    UTemplate(TemplateId),
    VTemplate(TemplateId),
}

enum RetirementBehavior {
    Pop,
    StopAtEnd,
    RetainExhaustedVTemplate,
    AwaitingVTemplateRetirement,
    CloseScantokens,
}
```

The implemented `PackedInputFrame` is the canonical fixed 40-byte frame from
`tex-state`. Its copy-only owner coordinate is the level identity and its
32-bit current offset is the sole live token delivery cursor; `SourceLevel`
and `TokenCursor` carry no duplicate identity, and `TokenCursor` carries no
duplicate index. A source retains its exact 64-bit byte/scalar/line cursor in
the physical sidecar because source sizes are not bounded by the frame's token
offset domain. The source frame's current offset is a delivered-token count,
not future semantics; unchanged future-state comparison normalizes that count
after verifying the exact source cursor and frame identity. TeX82's token types
retain their exact values, with disjoint source, `everyeof`, and Umber replay
kinds. Flags represent noexpand suppression, terminal-stop retirement, and
retained v-template retirement independently of storage.

Every source is adapted once at level creation into the same
`PackedTokenSpanHandle` plus the packed frame's scalar offset. Stored delivery
then calls `PackedTokenSources::token_at(handle, offset)` and advances only
that offset. The storage boundary makes one small direct owner-domain match
required by safe Rust; no delivery caller discriminates source variants,
builds a generic stored-delivery object, advances a second durable or macro
cursor, or clones a definition/token-list owner per word. `token_at` returns
canonical `TokenWord`; origin and source provenance remain adjacent diagnostic
coordinates and never become token or meaning semantics.

`CommandState::push_input_level` is the single live source/token frame
transition. It updates TeX82's `max_in_stack` scalar on the singular live
session owner and then makes the frame visible; it takes no lock and belongs to
neither snapshot roots nor portable identity. `push_token_level` first admits
the token storage and then delegates to that transition. Transient insertions,
backup/noexpand levels, alignment templates, stored every-hooks, output replay,
and other source-adjacent replay stream their words and optional source
provenance directly into one generation-owned `ReplayLane`.
The input level retains only a typed entry coordinate and length. The lane uses
coarse stable word/provenance segments: exact LIFO retirement returns whole
segments to reusable high-water storage, while a snapshot shares immutable
active segments and the current root opens a fresh tail before mutation. There
is no per-level allocation, live-row relocation, compaction, forwarding, root
search, or third generation. e-TeX aftergroup prefix linking appends a second
span to the top entry and delivers that span before its body without shifting
either span.

Macro replacement, argument-range, durable-list, and attempt-list spans remain
direct coordinates into their existing owners. They do not enter the replay
lane, acquire a shared token buffer, or copy their packed words at admission.

Detached resource continuations deliberately do not serialize a runtime frame
or arena coordinate. Detachment projects packed words, backup coordinates,
portable identity, and current offset into the existing handle-free DTO;
materialization admits a fresh destination-lane entry and frame, then advances
it to that offset. Command snapshots clone the compact frame/coordinate and
share immutable coarse lane segments. Source continuations retain their exact
physical byte/scalar/line cursor and rebuild the source frame after
registration, preserving diagnostic positions without publishing runtime
coordinates.

The implemented ownership model keeps copy-only `MacroActivation` values in
the `ParameterState` activation chain and stores a typed activation identity
in `TokenBehavior::MacroBody`. One admitted owner retains up to 64 macro
records and their live token/provenance closure. `MacroArguments` and every
live `ArgumentRange` name the same command-owned argument chunk by compact
coordinates. `InputLevelId` is typed separately from source identity and is
present on both source and token levels. Exact-byte and Unicode source cursors
use this identical enum.

An executing macro resolves its generation-safe meaning identifier through the
strong environment root already held for that meaning, without cloning the
root or upgrading a weak entry. Diagnostic parameter/replacement context is
rendered from the admitted packed macro owner, so retirement of the original
definition-store entry cannot invalidate an active input level. General cold
and stale lookup APIs retain their validation and rejection behavior.

The centralized transient and backed-up constructors avoid caller-side rich
staging for fixed insertions and stream directly into the replay lane. e-TeX's
optimized `\aftergroup` prepend adds a prefix span to the same replay entry
while preserving save order and its compact frame identity. Snapshot
normalization, durable continuation remapping, origin adoption, and
edited-source rehoming project the lane entry as the canonical semantic
payload.

`EveryPar`, `EveryHBox`, `EveryVBox`, `EveryJob`, `EveryCr`, `Mark`,
`OutputRoutine`, and similar explanations belong in `ReplayTrace` unless they
demonstrably change retirement behavior. Trace reasons never select expansion
semantics.

`ReplayTrace::Inserted` is the one exception, and it is not an explanation but
TeX82 §307's `inserted` token type: the level §323's `ins_list` installs, which
retires as a recovery rather than as a token list. It is a token _type_, not a
storage strategy, and every expansion that hands tokens back to the scanner
reaches the input stack through that same macro -- §470's `conv_toks` renders a
fresh transient buffer, §467's `ins_the_toks` shares §465's already immutable
copy of a token register, parameter, or font identifier. Nesting the type under
a payload-shaped `Transient` reason tied the two together, and §467's level was
installed as an ordinary stored token list instead: it published no push at all
and retired as a token list rather than a recovery (`umber2-johp.188`). §386's
`begin_token_list(cur_mark[cur_chr], mark_text)` is the contrasting case and
keeps an ordinary stored reason, because a mark's text is the stored list
itself and is never a copy handed back through `ins_list`.

Exhaustion commits against the exact `InputLevelId`. Ordinary, terminal-stop,
and `\scantokens` levels pop once; popping releases only the cursor's ownership
of transient or stored backing. An exhausted v-template instead transitions
once to `AwaitingVTemplateRetirement`, remains the exact top level through
end-template delivery, and is popped only after successful `do_endv`.
Macro-body retirement atomically removes the activation matching that level's
typed `param_start`; a mismatched activation chain is rejected before either
owner is mutated. A source pop moves its boxed `SourceOpenDepths` owner into
the retirement result, so `file_warning` consumes the already-validated top
frame record without an identity walk or ancestry clone. Nested `\input` and
`\scantokens` install that record in the source-opening transition before the
frame becomes observable. The committed lifecycle record may copy `ReplayTrace` for
observation, but neither its action nor activation cleanup consults that trace.
Its detached observation also retains the exhausted level's immutable class
(source, backup, macro, parameter, alignment template, recovery, or token
list), so host-side canonical translation preserves lifecycle ordering without
letting diagnostic explanation select retirement behavior.

An executor-owned stored replay has a second, command-state-owned completion
fence. TeX82 §390 can retire the stored level and immediately install the
replacement text of its final macro token; if that replacement yields an
unexpandable command, main control ends the current processor borrow while the
macro body still owns input. The retired replay identity therefore remains in
`CommandState` until every newer descendant level retires. A later processor
episode surfaces that completion before it may resume an older enclosing
source. The fence is snapshotted with the input stack and is never processor-
local observation state.

### 12.3 Macro parameters

A macro activation owns one compact argument record with at most nine ranges:

```rust
struct MacroActivation {
    definition: MacroDefinitionId,
    arguments: MacroArguments,
    invocation: OriginId,
}

struct MacroArguments {
    chunk: u32,
    start: u32,
    len: u32,
    record: u32,
}
```

`MacroArguments` is fixed at 16 bytes, `MacroActivation` at 48 bytes, and each
live scratch frame stores nine relative ranges plus the exact §394 paragraph
and removable-outer-group facts established during their first scan, beside
one segmented traced-word suffix. The paragraph fact records only
the ordinary `cur_tok=par_token` branch: an equal token first held as delimiter
prefix and later committed after a mismatch is not reclassified. The scalar
matcher admits the one live frame before collection and updates its direct
current-argument slot in definition order. It consumes those facts for the
non-`\long` decision and outer-pair removal without rereading stored words.
Sealing advances the live depth of that same metadata frame without moving its
physical words because admission already appended to the frame's segment chain;
no segment owner, argument table, range, fact, or word moves.
Empty arguments retain empty half-open ranges. A compact
`OutParameter(u8)` remains distinct from a literal parameter character emitted
by the canonical `##` escape, so replay can substitute only the former without
rewriting immutable macro definition token lists. Argument segments hold 4,096
words. A sealed `(frame, argument slot)` selects its physical range directly,
with no range search or second argument-local cursor. Sequential iterators
follow each segment link once. Exact LIFO frame retirement returns its disjoint
chain to the generation's intrusive reusable free head, so an older active
frame can retire beneath a pending child without moving either frame's words.
Quiescent top-level calls clear lengths but retain every
warmed allocation. The processor appends one fixed-width invocation provenance
record using the active activation's invocation coordinate as parent; no rooted
weak value is created on replay. Node, diagnostic, and continuation boundaries
materialize a structural root on demand. The activation is installed before
its immutable replacement-body span becomes visible.

An `OutParameter` read directly from a macro body pushes a parameter range.
An `OutParameter` read from other nested token input resolves against the
nearest live macro activation when canonical TeX semantics require
`param_start`. A parameter level replays its already materialized range
literally and cannot recursively substitute itself.

### 12.4 Backup and `\noexpand`

Backup is a one-token input level or a compact run of explicitly inserted
tokens. It contains the exact spelling and an optional one-delivery treatment:

```rust
enum BackupTreatment {
    Ordinary,
    SuppressExpandableControlSequence,
}
```

`\noexpand` temporarily uses normal scanner status to fetch its target, undoes
that delivery through the canonical backup operation, and backs it with
`SuppressExpandableControlSequence`. When `get_next` reads the target, an
expandable control sequence receives the inaccessible frozen-`\relax` command
identity (`cur_cmd=relax`, `cur_chr=no_expand_flag=257`); its spelling remains
the original control sequence. This identity belongs to the ephemeral current
command, alongside its delivery proof, rather than to snapshot state or
observer reconstruction. The treatment ends with that delivery.

TeX82 §§358–359 physically represents this treatment by prefixing the
backup with `frozen_dont_expand`. The canonical input stack keeps the marker
structural, but §315 error-context pseudoprinting projects its §258 spelling
`\notexpanded:` before the operand on the appropriate side of the live cursor.
The structural representation therefore preserves both one-delivery behavior
and TeX's exact diagnostic view.

There is no general `NoExpand` token-list replay kind and no sticky suppression
property on returned tokens.

### 12.5 `\\csname` construction

`\\csname` collects through the same ordinary `get_x_token` loop used by all
scalar expansion. Expanded character commands contribute their character code
until `\\endcsname` is delivered as the collector boundary. The completed
spelling is interned in the named control-sequence namespace; an undefined
name receives `\\relax`, while an existing meaning is preserved. The resulting
control-sequence token is injected as ordinary transient input with an
expansion-synthesized origin, so its next delivery follows the canonical raw
path.

If a non-character command appears before `\\endcsname`, the command is backed
up through the normal recovery path, the missing-endcsname diagnostic is
queued, and the partial name is still constructed. `\\endcsname` is excluded
from ordinary expandable dispatch solely as this collector boundary; no
second expansion interpreter is introduced.

## 13. Source tokenization

Source tokenization implements:

- physical line boundaries and terminators;
- TeX trailing-space stripping;
- `endlinechar`;
- M/N/S states;
- ignored, invalid, active, escape, letter, space, comment, brace, alignment,
  and other catcodes;
- control-word and control-symbol formation;
- canonical `^^` notation for the active profile;
- blank-line `\par`;
- exact source ranges; and
- control-sequence lookup or creation policy.

Catcode lookup happens per delivered character. A catcode assignment between
tokens is immediately observable. Source lines are never pre-tokenized beyond
the next command.

`SourceToken` remains owned for tokenizer and CLI consumers that need the
semantic name itself. Production source tokenization uses the same state
machine with a destination projection. An untransformed multi-character
control word scans byte boundaries in the current contiguous source backing
and passes that `&str` slice directly to the creating or non-creating interner
boundary. If `^^` reduction or exact-byte character encoding makes the
semantic name differ from the raw bytes, one owned `ControlSequenceName`
fallback accumulates the logical character codes instead. The boundary
resolves either call-local spelling to a packed `TokenWord`, and only that
compact identity plus direct-source provenance crosses into canonical command
delivery. Delivery performs only
`TokenWord`/control-sequence identity to current eqtb meaning resolution; it
does not reconstruct a name, choose creation policy, or retain a fallback.
Neither the borrowed name nor the owned fallback therefore enters an input
level, current command, snapshot, or suspension. `ControlSequenceName` keeps up to 24
semantic character codes inline and spills longer names to an unbounded vector. A
repository fixture census measured 9,770 control-word occurrences at median
5, p95 15, p99 20, and maximum 31 characters; all registered primitive-name
literals were at most 17, so the bound covers more than 99% of measured source
names and every primitive without imposing a semantic length limit. The common
borrowed path performs no name construction or encoding. A fallback name still
encodes inline character codes into a fixed stack UTF-8 buffer for lookup or
interning; an already-spilled pathological fallback may allocate that
temporary conversion.

`get_token` temporarily enables creation only at the source-tokenization seam;
ordinary `get_next` leaves that seam in TeX82's non-creating
`no_new_control_sequence` state. TeX82 §257
sets that flag, §365 clears it only around `get_token`, and §374 clears it only
around `\csname`'s own `id_lookup`, so a raw scan may not enter a name in the
hash table at all — the interner is that hash table, and interning during raw
delivery is not a diagnostic convenience but a semantic mutation, because the
name becomes findable by every later scan.

Section 356 sends every control word to the hash, including a one-letter word;
§354 resolves a control symbol to `single_base+c` and an escape at normalized
line end to `null_cs`, and §351 gives a blank line's `\par` `par_loc`; those
are permanent `eqtb` locations that exist before any scan, so the creation
policy never applies to them. For a hash name the table has never held, §259's
`id_lookup` returns §222's single dummy `undefined_control_sequence` location.
Umber models that location as an inaccessible frozen token rather than an
interned spelling, exactly because it is not a hash entry: it carries §222's
permanent `undefined_cs` meaning, cannot be assigned, and every such name
shares the one identity. Its observed spelling is `^^@`, which is what §263's
`sprint_cs` prints for the slot — web2c sizes `hash` through it and clears it
with `hash[hash_base]`, whose `text` §257 sets to 0, and §48 builds string 0 as
the printable form of character 0.

Because the policy is a property of the reader, every raw-delivery caller must
match the section it implements: §442's alphabetic character constant and
§477's unexpanded `scan_toks` read with `get_token`, while §380's
`get_x_token`, §478's expanded `scan_toks`, §494's `pass_text`, and §507's
`\ifx` operands read with `get_next`.

Invalid input emits the canonical recoverable diagnostic and restarts raw
delivery after consuming the offending character.

`UnicodeExtended` reuses the M/N/S transition machine but is an explicit Umber
extension, not pdfTeX input compatibility. Registration rejects malformed
UTF-8 before allocating a source identity. Delivery emits only Unicode-domain
`CharacterCode` values, including synthetic spaces, `\par` spelling, and
superscript-reduction results. The aggregate sparse code table supplies every
catcode (and therefore its defaults); tokenization performs a fresh query for
each classified scalar and does not embed a Unicode-category heuristic.

The Unicode superscript policy accepts a repeated current-catcode superscript
scalar followed by either two ASCII hexadecimal digits, or two further copies
of that scalar and four ASCII hexadecimal digits. ASCII hexadecimal digits are
case-insensitive. Otherwise the following scalar is transformed by adding 64
below U+0040 or subtracting 64 at and above U+0040, provided the result remains
a valid Unicode scalar. A transformed superscript scalar is reprocessed. The
complete notation contributes to both the exact UTF-8 byte range and decoded
scalar range of the resulting token or invalid-character recovery step.

## 14. Canonical `get_next`

There is one semantic raw-command operation. In conceptual order it:

1. selects the current input level;
2. refills or retires an exhausted level;
3. reads one source token or stored token;
4. substitutes `OutParameter` input by pushing the correct parameter range;
5. resolves character command codes or the current control-sequence meaning;
6. processes one-shot backed-up suppression;
7. applies `scanner_status` outer-validity and EOF rules;
8. updates `align_state` for literal character braces and records that exact
   adjustment on the final current command;
9. detects an alignment delimiter at the current base depth;
10. pushes or retires the canonical template input required by that delimiter;
11. records the semantic read and diagnostic provenance; and
12. leaves the resolved command in the active request's destination and returns
    the compact `Command` status.

Steps may restart without returning a command, exactly as TeX restarts after
ignored characters, exhausted input, parameter insertion, and template
insertion.

For direct source, creation or lookup consumes the transient tokenizer name and
returns an escaped control-sequence token that already contains its compact
session-local symbol coordinate. Stored tokens carry the same coordinate. Raw
delivery uses it directly for the current meaning lookup; it does not resolve
the retained name bytes merely to validate an identity already admitted when
the token was created or restored. Active character spellings remain character
tokens and resolve through their distinct active-character namespace before
the same compact meaning lookup.

The implemented scalar loop owns source and token-cursor selection, exact
retirement/restart, stored token/origin reconstruction, `OutParameter` replay,
one-delivery backed-up suppression, current-meaning resolution, and literal
brace `align_state` accounting. `get_token` is routed through that same loop.
Scanner-status outer recovery and alignment-template interception have their
sole entry points in this loop. `check_outer_validity` captures the live typed
status and warning identity, records a forbidden-control-sequence diagnostic
before backing up an offending outer macro through exact delivery identity,
substitutes the current recovery space, clears the live scanner episode, and
pushes bounded ordinary recovery input. This follows the pinned TeX82 §23
instrumentation boundary; terminal EOF reports its separate diagnostic before
the same recovery insertion. The recovered delivery projects its effective
`spacer` command and character code even though the original outer
control-sequence spelling remains in the exact backup input for rereading.
Terminal runaway
recovery follows the same path: definitions and absorbed text receive `}`;
macro matching receives frozen `\par`; alignment preambles receive frozen
`\cr` then `}`; and skipped conditional text receives frozen `\fi`. The
insertions restart canonical `get_next`; no scanner caller chooses semantic
recovery. For TeX82 §§379 and 510, terminal skipped text publishes
`outer_validity_eof(skipping)` followed by `conditional_incomplete` before
the recovery input push and frozen-`\fi` observation; this remains
command-owned so `pass_text` restores its prior scanner status normally.

The input-delivery audit drives the focused committed TeX82
`input-recovery.tex` source through both public raw entry points for every
exact TeX82/e-TeX/pdfTeX profile. It also binds the required raw-delivery,
retirement, and terminal-stop observations to all three canonical semantic
matrices, and literal-brace observations to each engine's focused alignment
case. The recovery audit additionally requires the three matrices to retain
exact backup, outer-command, runaway-EOF, and every non-normal scanner-status
observation; TeX82's focused EOF children bind each status to its canonical
inserted recovery token. A rollback test captures a live matching episode,
proves recovery is bounded, then restores and replays it. A crate boundary test mechanically preserves one
shared scalar loop, keeps profile selection beneath it, and rejects semantic
condition/cache/scanner/expansion/paragraph state on input levels or replay
trace-controlled delivery.

Control-sequence aliases of brace meanings do not change `align_state`;
literal catcode-1 and catcode-2 character tokens do.

Every raw semantic consumer uses `get_next` or `get_token`, including:

- macro argument matching;
- conditional `pass_text`;
- `\ifx` operands;
- unexpanded `scan_toks`;
- definition parameter scanning;
- `\string` and `\meaning` operands;
- alignment lookahead; and
- scanner lookahead that may back up its exact command.

Lower-level lexical reads are private and cannot be called by scanners or
`tex-exec`.

The scalar expansion implementation produces `\number`, `\romannumeral`,
`\string`, `\meaning`, and `\fontname` as bounded transient character input
with synthesized value-rendering provenance. `\string` uses the delivered
token spelling, while `\meaning` uses its separately retained effective
meaning, including TeX82's `macro`, `\long macro`, `\outer macro`, and
`\long\outer macro` command spelling before its immutable definition text.
Token-register and token-parameter `\the` results are instead
inserted as immutable stored-token input: producing that splice consumes only
the quantity target and never expands or reads beyond its contents. A later
ordinary expansion loop remains responsible if those tokens subsequently
become normal input.

## 15. `get_token`, backup, and exact delivery

`get_token_into` temporarily grants its next source-tokenization step TeX82
§365's creation permission, then invokes the same canonical raw driver and
leaves the same `CurrentCommand` with its packed token spelling in the caller's
destination. Stored, transient, and backed-up words never observe that
permission. The value-returning `get_token` wrapper exists for compatibility
outside the hot delivery chain.

`back_input` implements TeX82 §325 in that section's order:

1. validates that the nonce-bearing delivery stamp still identifies the most
   recent raw transition in the live processor episode (not merely an equal
   token at an equal cursor position);
2. runs §325's stack-conservation loop (§15.1 below) so every depleted
   token-list level retires _before_ the backup exists;
3. undoes exactly one literal-brace alignment adjustment made by that
   delivery, which is §325's `align_state` correction;
4. pushes a backed-up token level carrying the exact spelling, origin, raw
   source span, and typed canonical location.

Every backup is an explicit level: §325 has no in-place rewind, and a
one-delivery treatment such as `\\noexpand` therefore needs no special case.

Semantic equality to a previously delivered token is not proof that the token
can be backed up.

`back_error` performs the same backup and then queues the canonical recoverable
diagnostic. Inserted recovery tokens acquire explicit inserted origins and
ordinary future delivery semantics.

### 15.1 Stack conservation before a new token list

TeX82 §§325 and 390 spell the identical loop before pushing a new token list:

```text
while (state=token_list)and(loc=null)and(token_type<>v_template) do
  end_token_list; {conserve stack space}
```

§390 runs it before `macro_call` installs a replacement text; §325 runs it as
`back_input`'s first act. The command core has exactly one implementation,
`CommandProcessor::conserve_input_stack`, and both callers use it.

The loop's condition is `loc=null` alone. It is therefore total over
token-list kind -- depleted macro bodies, replayed parameters, backups,
recovery insertions, u-templates, and stored replay episodes all drain here --
it iterates over a whole depleted run rather than one level, and it does not
consult which level made the last delivery. Narrowing it to a particular kind
or to the delivering level reorders the resulting `input retire` transitions
after the new level's push, which is observable. `v_template` is the sole
exception in both sections: an exhausted v-part stays live until `do_endv`
retires it (§13's alignment cell completion). A retirement that completes an
executor-owned stored replay episode records that completion in persistent
command state for the next `get_next` and keeps draining. TeX82 §390 can then
push the replacement text of the episode's final macro token above that retired
level; the completion fence survives processor borrows and waits for input
levels newer than the episode before it permits delivery to resume from an
older enclosing level.

## 16. Scanner status and outer validity

`check_outer_validity` is one command-core operation driven by
`ScannerStatus`. It decides whether:

- a file or subfile may end;
- an `\outer` command may be observed;
- the offending control sequence is backed up;
- the current command becomes a space;
- a definition or absorbed text receives a right brace;
- a macro call receives `\par`;
- an alignment preamble receives frozen `\cr` and a right brace; or
- skipped conditional text receives frozen `\fi`.

This check runs after every source-level retirement, including a nested
`\input`; skipped text cannot return to the parent source before its frozen
recovery input is installed. For skipped conditional EOF, the observer records
the EOF and incomplete-condition diagnostics, then the recovery input push and
frozen `\fi`, before `pass_text` records its `skipping -> normal` restoration.

The diagnostic captures the status's typed warning identity and partial
builder. Presentation is deferred, but the recovery token sequence and future
state match the canonical batch engine.

Observer recovery kind records the TeX82 insertion operation, separately from
the inserted token identity, and TeX82 reports that operation at two distinct
seams. §325's `back_input`, reached through §327's `ins_error`, reports one
inserted operation whatever it inserts: §336's frozen `\fi` and §379's frozen
`\relax` are both `InsertedToken` even though each token is an inaccessible
control sequence, so consumers must not infer the operation kind from the
token's spelling. §323's `begin_token_list(p, inserted)` reports instead which
side of §289's `cs_token_flag` split the inserted list's leading token falls
on. §339's runaway recovery therefore classifies its frozen `\cr` and `\par`
as `InsertedControlSequence` while a `}` stays `InsertedToken`, and §470's
conversion text and §467's `\the` copy are classified from that one token
rather than by a constant chosen per call site.

The supported transcript-parity target is deterministic batch/nonstop
operation. Interactive deletion and insertion prompts are host UI and are not
implemented as a semantic execution mode.

## 17. Expanded command delivery

There is one ordinary loop:

```rust
fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
    loop {
        let Some(command) = self.get_next()? else {
            return Ok(None);
        };
        if !command.meaning.command.is_expandable() {
            return Ok(Some(command));
        }
        self.expand(command)?;
    }
}
```

The promoted scalar implementation routes macro calls through the canonical
`macro_call` activation path and implements `\noexpand` and `\expandafter`
by mutating backed-up input levels directly. `\noexpand` stores its treatment
only on the one replay level, while `\expandafter` explicitly replays its
first token after expanding or backing up its second token. Remaining
expandable primitive families extend this same dispatch; they do not add a
second expanded-delivery interpreter.

The real loop also:

- shares one expansion budget with nested expansion;
- records meaning and value reads;
- converts frozen end-template commands;
- applies deterministic undefined-control recovery;
- respects the selected protected-macro policy; and
- captures diagnostic context on failure.

`get_x_or_protected` invokes the same internal loop with:

```rust
enum ProtectionPolicy {
    ExpandProtected,
    StopAtProtected,
}
```

There is no `ExpansionMode` trait and no second restricted interpreter.
Input-opening authority, read recording, and undefined-command recovery are
capabilities or policy fields on the active processor.

## 18. Expansion dispatch

`expand` consumes an expandable `CurrentCommand` and mutates the active
command state directly:

```rust
fn expand(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
    match command.meaning.command {
        CommandCode::Macro => self.macro_call(command),
        CommandCode::Expandable(opcode) => self.expand_primitive(opcode, command),
        _ => unreachable!(),
    }
}
```

Expansion may:

- push a macro-body input level;
- push converted or queried text;
- manipulate backup input for `\expandafter` or `\noexpand`;
- evaluate or skip a conditional;
- create and push a control-sequence token;
- start or end an input source;
- insert marks or token parameters; or
- produce a typed resource need.

There are no `Dispatch::Push` or `Dispatch::PushTransient` values that mirror
an input level and are immediately translated back into mutation.

Expandable primitive dispatch is a statically compiled match over a closed
opcode enum. Pure heavyweight helpers such as regex, MD5, or numeric formatting
remain isolated functions outside the token-delivery loop.

Command-text rendering is append-oriented. Numeric, glue, token,
control-sequence, meaning, and command renderers append into one caller-owned
`String`; nested renderers and static literal arms do not construct owned
intermediate strings. Thin allocating wrappers remain only at boundaries that
must hand ownership to input replay, diagnostics, or retained error state.
Token-list rendering delegates the shared TeX82 token spelling rules to
`tex-state`'s append helpers, so `\escapechar`, control-word delimiters, and
parameter-character doubling remain one implementation.

## 19. Canonical scalar `macro_call`

The semantic implementation follows TeX's scalar algorithm before any
optimization is considered.

Macro definitions retain canonical parameter and replacement token lists.
TeX82 §27's `\\meaning` renders those immutable lists directly after the
effective macro command spelling (`macro`, `\long macro`, `\outer macro`, or
`\long\outer macro`), followed by
`:<parameter-text>-><replacement-text>`; it never consults a live macro
activation or macro-body input level, because §392 retires those call-local
owners before later recovery input can be read.
`macro_call`:

1. installs `ScannerStatus::Matching` only when its parameter text is not
   empty;
2. matches compulsory leading tokens with raw `get_token`;
3. scans up to nine arguments in definition order;
4. uses canonical undelimited or delimited matching;
5. preserves overlapping delimiter-prefix recovery;
6. tracks literal brace depth;
7. enforces the narrow paragraph exception and `\long`;
8. applies the `#{` delimiter/brace rule;
9. strips one complete outer group when required;
10. stores every completed argument once in one compact command-arena span;
11. retires any exhausted macro-body level before placing the replacement
    list above its caller, while preserving the independently observable
    backup and recovery lifecycles;
12. creates one macro invocation provenance node and pushes the replacement
    as a macro-body input level owning the argument ranges; and
13. restores the matching scanner status, when one was installed.

The initial promoted implementation does not use a compiled delimiter
automaton, macro bytecode, or alternate fast matcher. Such acceleration may be
added only through the optimization policy in section 32, with the canonical
scalar matcher retained as the specification fallback and test oracle inside
the new subsystem.

TeX82 §392 skips the parameter matcher entirely for a definition beginning
with `end_match`: a parameterless macro pushes its replacement body directly,
without a `Matching` observation. Otherwise the implementation installs
`Matching` around compulsory-prefix and argument delivery. It uses raw `get_token`, strips precisely one outer argument group,
retains nested literal braces, and lets the existing outer-validity operation
perform all inserted-token recovery. TeX82 §394 backs up a forbidden `\par`
while `Matching` is still live, before the failed call restores its enclosing
scanner status; this preserves the recoverable input and scanner-transition
order for a non-`\long` macro. An expanded definition scan then discards that
failed expansion and collects the backed-up paragraph while `Defining` remains
live. By contrast, §23 EOF recovery inserts frozen `\par` while matching;
that terminator is consumed by the failed call, so it must not acquire a
visible §394 backup replay after the matching-status exit. Direct delimited
matching retains the
maximal overlapping delimiter prefix after a partial mismatch, commits each
unreusable leading token literally with its command-owned
`macro_delimiter_recovery` observation, ignores delimiters below literal brace
depth, and cancels the raw brace accounting for a matched `#{` delimiter. A
successful call freezes every range once, creates one invocation origin, and
installs exactly one activation/body pair over the canonical replacement list;
replay resolves its compact `OutParameter` tokens through that activation.
Before this replacement hand-off, TeX82 §392 cleans the exhausted calling
macro body before the new body is pushed. The typed input stack preserves its
separate raw-delivery lifecycles for transient recovery and §25 backups, while
the v-template remains live for its `end_template`/`endv` hand-off. This
preserves recursive-macro stack bounds and the committed macro-retirement
order after recovered paragraph input.

The committed TeX82 `expansion-macros.tex` fixture is the focused canonical
audit for compulsory, undelimited, delimited, overlapping-prefix,
nested-brace, paragraph, `#{`, nine-parameter, and nested-replay behavior.
The shared e-TeX and pdfTeX transition fixtures independently bind matching
entry/restoration, completed arguments, activation, delimiter completion, and
overlap recovery to their canonical observers. Crate tests additionally prove
that rollback before a successful call replays the same activation and
argument ranges while appending exactly one compact invocation record per replay;
an architecture gate preserves one raw scalar fallback and rejects alternate
matcher types.

## 20. Canonical `scan_toks`

`scan_toks` is not implemented by blindly calling generic `get_x_token`.

Each semantic `ScanToksMode` constructor is decoded once at entry into a
private typed configuration. Distinct enums retain the grammar, opening
strategy, expansion policy, scanner warning owner or macro target, completed
token-list purpose, and scanner-status visibility. The collector consumes
that configuration without reclassifying the original mode or exposing a bag
of boolean switches. In particular, `GeneralAfterOpening` remains the typed
§1227 prevalidated-opener path and `GeneralText` retains its extension-specific
observation suppression.

TeX82 §482 `read_toks` uses the shared scanner-episode lifetime but remains a
separate whole-line collector, not another `ScanToksMode`: it holds
`align_state=1000000`, continues across imbalanced lines, and performs §486's
runaway recovery without inventing balancing braces.

It owns:

- scanner-status installation;
- a token builder;
- parameter-text scanning;
- replacement-text brace depth;
- highest valid macro parameter;
- possible `#{` handling; and
- expanded versus unexpanded collection.

For the `#{` parameter-text case, TeX82 §476 stores the left brace in the
parameter text, terminates that text with `end_match`, and appends the same
left-brace token after the completed replacement body. The parameter character
only introduces this special case; it is not retained by either list.

For an unexpanded scan it repeatedly calls `get_token`.

Before a general-text collector installs `ScannerStatus::Absorbing`, its
structured-scanner wrapper finds the mandatory opening brace through
`get_x_token` and backs up that exact delivery. The collector then installs
the status and redelivers the brace through the same expanded path before
collecting its unexpanded body. This preserves TeX82's normal-status recovery
boundary and keeps the backed-up brace's diagnostic origin without reporting
it as a second physical-source location.

The same command-owned scanner distinguishes the two TeX82 assignment
representations: `\toks` registers receive the collected body, while token
parameters such as `\output` receive one frozen list retaining the enclosing
braces. Replay applies either completed list without inspecting input tokens.

For an expanded scan it follows the canonical structure:

1. call `get_next`;
2. store an unexpandable command;
3. expand an ordinary expandable command once and continue;
4. splice the token-list result of `\the` directly without re-expanding it;
5. splice e-TeX `\unexpanded` directly through the same family;
6. stop protected macros when the active e-TeX policy requires;
7. preserve compact `OutParameter` and escaped-parameter rules; and
8. stop at the inaccessible collector boundary without reading caller input.

The replacement loop supplies one caller-owned `Option<CurrentCommand<G>>`
as the delivery destination. Classification, observation, and spelling borrow
the resident command, then successful progress clears the option in place.
Only TeX's real backup path or typed resource suspension consumes it. This
keeps ordinary collection destination-directed without a returned-command
handoff, a heap indirection, or generation-long retention.

Tokens returned by `\unexpanded` or token-list `\the` have no permanent
suppression metadata. If they later re-enter ordinary input, ordinary
`get_x_token` expands them.

The collector freezes its traced result through the narrow aggregate command
context capability. It receives no host or input-opening capability, so a
direct splice remains a local immutable-store operation.

The scalar-expansion audit binds these rules to the committed TeX82
`expansion-macros.tex` observations for macro expansion, `\noexpand`,
`\expandafter`, `\csname`, conversions, ordinary and expanded collection, and
direct `\the` splices. The shared e-TeX and pdfTeX matrices retain the common
collection/status observations. Focused command tests prove completed ordinary
expansion and completed direct-splice collection replay identically after an
executor-step rollback. A crate-boundary test mechanically preserves the one
step-at-a-time collector, direct immutable splice, and absence of a second
ordinary expansion loop.

## 21. Conditionals

Condition processing maps TeX's independent condition stack directly:

```rust
struct ConditionFrame {
    identity: ConditionId,
    limit: IfLimit,
    kind: ConditionalKind,
    source_line: u32,
    branch: ConditionalBranch,
    inverted: bool,
}
```

Beginning a condition pushes a frame with the evaluating limit before scanning
operands. Completing evaluation updates that exact frame identity even if
recursive operand expansion pushed a nested condition.

The implemented stack uses distinct `ConditionalKind`, `IfLimit`, and
`ConditionalDelimiter` values rather than sharing TeX's integer command-code
space. `ConditionId` is monotonic within the command state and
`change_if_limit` searches by that identity from the stack top, so an operand
that expands a nested condition cannot retarget its outer frame. A delimiter
observed while its frame remains `Evaluating` produces typed incomplete-if
recovery context; recovery insertion and conditional evaluation consume that
context at their own command transitions. A delimiter exceeding `if_limit`
publishes its extra-delimiter diagnostic at that raw delimiter delivery
(TeX82 §509), before the following input command.

`pass_text`:

1. installs `ScannerStatus::Skipping`;
2. repeatedly calls canonical `get_next`;
3. counts nested conditional commands;
4. stops at the next outer `\or`, `\else`, or `\fi`, leaving frame-limit
   validation and extra-delimiter recovery to the caller;
5. preserves literal-brace alignment accounting and template interception;
6. applies outer-validity recovery at each exhausted source boundary through
   the shared mechanism, before parent input resumes;
7. observes the live `skipping` to prior-status restoration, then restores
   the prior scanner status; and
8. records the delimiter it stopped at as one condition `branch` observation
   against the live top-of-stack frame.

Step 8 is TeX82 §494's `done:` label, which every skip reaches, so no caller
records a branch of its own: §498's false boolean limb, §509's `\ifcase` limb
count, and §510's skip to `\fi` all publish exactly one branch per `pass_text`
invocation, under whatever limit that frame currently carries. The observed
frame is the stack top — TeX's `cur_if`/`if_limit` — which is not always the
frame the skip was started for, because §500's `\if\iftrue abc\else d\fi`
leaves an inner frame above it.

The TeX82 predicate dispatcher selects `get_x_token` for character/category
tests and `get_next` specifically for `\ifx`; the latter preserves raw meanings
and must not expand either operand. The two caller-local command slots remain
the operand owners through comparison: `\ifx` borrows their meanings directly,
and macros compare their flags plus raw parameter and replacement token
sequences through the borrowed definition coordinates rather than cloning
those owners or comparing immutable-store allocation identities. The
comparison remains exact when a bounded candidate index rolls over and equal
live token sequences receive distinct timeline-local coordinates.
Character/category tests normalize non-character operands to TeX's common
relax sentinel before comparing them.
Boolean false limbs and selected `\ifcase` limbs re-enter the single
`pass_text` machine, while `\else`, `\or`, and `\fi` change or pop only the
live frame selected by its stable identity. Premature delimiters enqueue an
incomplete-if diagnostic and insert inaccessible frozen relax; delimiters that
exceed the frame limit enqueue a deterministic extra-delimiter diagnostic and
are ignored. Numeric and dimension comparison operands are delegated at the
typed scanner boundary; execution-mode and box predicates consult the
executor-owned mode nest and aggregate box state when those boundaries are
active.

For a false boolean limb, its frame remains `evaluating` while `pass_text`
delivers the skipped raw tokens, so the branch `pass_text` records is under
that pre-change limit. TeX82 §498's shared `common_ending` then either pops
the frame for `\fi` or changes it to `fi` for `\else`, and §509's exhausted
`\ifcase` limb count reaches the same `common_ending` rather than a
duplicate of it. This keeps the observable TeX82 transition order separate
from the stack's typed state transitions.

An `\ifcase` frame likewise remains `evaluating` while `pass_text` skips each
non-selected limb, so each traversed `\or` is recorded under that pre-change
limit; only after the last one does the command core change the frame to `or`
and record the selected `case` branch. A negative case index is not a separate
path: §509's `while n<>0 do ... decr(n)` loop never reaches zero for one, so
the same loop skips to `\else` or `\fi`.

`\or` and `\else` reaching §510 as accepted delimiters do not change
`if_limit`. TeX82 §510 is only `while cur_chr<>fi_code do pass_text` followed
by a pop, testing the delimiter already in hand so `\fi` skips nothing; the
frame keeps the limit it had, which is the limit each remaining skipped limb
is recorded under.

The command host installs an ephemeral `ConditionalState` projection for each
executor operation. It contains only the three-way mode family and the
`\ifinner` fact, so it is neither part of `CommandState` nor captured by a
command snapshot. `tex-exec::ModeNest` is the sole producer of this projection.
The typed dimension scanner reads registers, parameters, and page dimensions
through `CommandContext`; box predicates use one aggregate box-kind query.
Condition frames therefore remain independent of input levels, executor mode
ownership, and node-store representation.

Frozen relax and frozen fi recovery use inaccessible primitive identities
rather than live re-definable control sequences.

e-TeX `\unless` is one inversion bit on supported predicates, not a duplicated
conditional interpreter.

The command-boundary audit is executable. The canonical TeX82 fixture matrix
pins nested evaluation, `\ifx`, `\ifcase`, skipped braces, delimiter and EOF
recovery, `\unless`, and expanded definitions; `boundaries.rs` additionally
requires `pass_text` to consume only `get_next`, `\ifx` to consume only
`get_token`, and condition frames to remain outside input levels. Snapshot
tests capture nested live alignment delivery and prove that it restores as an
executor-step state while durable summaries reject it as nonquiescent.

## 22. Alignment delivery

The command core owns raw token delivery for alignments; `tex-exec` owns row,
column, span, grouping, packing, and node construction.

`AlignmentDeliveryState` implements:

- preamble `-1_000_000`;
- between-entry/template `1_000_000`;
- cell-body base `0`;
- literal brace increments and decrements;
- u-template completion;
- top-level tab, `\span`, and `\cr` interception;
- v-template insertion;
- exact exhausted-v-template retention;
- frozen end-template conversion;
- backup correction;
- nested alignment suspension and restoration; and
- template retirement after successful `do_endv`.

The executor calls narrow lifecycle methods such as:

```rust
begin_alignment
set_preamble_phase
begin_cell
finish_cell
suspend_alignment
resume_alignment
finish_alignment
```

It cannot assign arbitrary `align_state`, classify commands, or inspect token
cursor internals. Recovery APIs express canonical operations such as backing a
delimiter and inserting the required group closer; §1127's `align_error` is
one such API (`recover_align_error`), reached from main control's §1126 arms
rather than from raw delivery.

The authoritative semantic mapping in `docs/alignment_brace_semantics.md`
remains applicable and should be updated to name the new owners when migration
completes.

The same audit binds the canonical `alignment-delivery.tex` events for
preamble sentinels, delimiter interception, u/v template retirement, backup,
and nested suspension. The executor's focused `off_save` tests retain its
bounded replay/drop recovery gate; canonical trace coverage is tracked
separately by `umber2-johp.7.8`. The architecture gate permits the executor
only typed lifecycle requests and an opaque end-template event: raw delimiter
classification and the integer `align_state` remain in command-core delivery.

## 23. Value scanners

Scanners are modules over `&mut CommandProcessor`; they are not generic over
input, state, host, or expansion mode.

They choose from a small canonical vocabulary:

- `get_next` for raw semantic input;
- `get_token` when exact backup may be required;
- `get_x_token` for ordinary expanded input;
- `get_x_or_protected` only where e-TeX requires it; and
- direct pure parsers after input has been reduced to owned values.

Shared helpers implement spaces, optional signs, keywords, filler, register
indexes, relations, and internal values. Helpers never open input sources,
dispatch arbitrary expandable commands, or mutate input levels directly.

### 23.1 `scan_keyword` restores two levels, not one

TeX82 §407's failed keyword match is not a single undo. It runs §325's
`back_input` on the offending token and then §323's `back_list` on the prefix
that had already matched, so the prefix is pushed _above_ the offender and is
reread first. The two are different operations: `back_input` undoes one
delivery -- stack conservation, that delivery's literal-brace `align_state`
correction, and a recovery record naming the token -- while `back_list` is a
plain `begin_token_list(p,backed_up)` of a list the scanner assembled, with
none of those. Both pushes are observable and only the first carries a
recovery record, so collapsing them into one backed-up level (or pushing the
combined list without observing it) loses transitions the pinned oracle
records for every partially matched keyword, such as `scan_keyword("em")`
rejecting the `x` of `\lower.5ex`.

Two further §407 rules are properties of the same loop rather than special
cases. A spacer read while nothing has matched yet satisfies neither branch:
it is consumed, discarded permanently, and the keyword index does not advance,
so leading spaces are skipped and never restored, while a spacer after a
partial match is an ordinary mismatch. And `cur_cs=0` restricts a match to a
character token, so a control sequence `\let` to a keyword letter -- same
`cur_cmd`, same `cur_chr` -- still cannot spell one. The comparison itself is
on `cur_chr` alone, so a keyword letter matches under any category code.

Keyword text is traversed directly rather than copied into a temporary letter
list. The matched prefix uses 13 inline `BackedUpToken` slots containing only
the spelling and optional source provenance needed by mismatch replay; it does
not retain resolved meanings or definition owners across a suspension. The
bound covers the complete current TeX82, e-TeX 2.6, and pdfTeX 1.40.29
production vocabulary. The public scanner retains an explicit heap spill for
longer caller-supplied keywords, so the storage optimization introduces no
semantic keyword-length limit.

Integer, dimension, glue, muglue, and expression arithmetic use shared exact
types from `tex-arith` and `tex-state`. The integer scanner owns decimal,
octal (`'`), and hexadecimal (`"`) digit delivery: radix introducers and every
accepted digit are expanded-command deliveries before the scalar observer
publishes its completed value, while its one trailing space is absorbed. This
keeps the canonical `scan_int` input/observer ordering within the command core
rather than making the replay seam synthesize it. Internal register values
also scan their register index through that same command-owned scalar path,
then publish the nested integer, internal-value, and outer integer results in
canonical order. Overflow, rounding, radix, unit, and recovery behavior cite
canonical sections and compare against reference fixtures.

Register arithmetic preserves TeX82 §104's intentional distinction between
unchecked dimension addition and the checked multiplication routines. In
particular, §1238's plain `cur_val+eqtb[l].int` sum may commit the
machine-representable `-max_dimen-1sp` boundary; it does not take §1236's
`arith_error` return merely because the result lies outside the scanner's
input range.

TeX82 §452 retains at most the first 17 decimal-fraction digits for §453's
rounding. The scanner stores those digits in a fixed byte array while still
delivering and consuming every later decimal digit through the canonical
terminator; a non-space terminator is backed up and a space terminator is
absorbed.

TeX82 §433-§437 declare five "restricted classes of integers" --
`scan_eight_bit_int`, `scan_char_num`, `scan_four_bit_int`,
`scan_fifteen_bit_int`, and `scan_twenty_seven_bit_int` -- and all five have
one shape: run `scan_int`, then, if the result is negative or above the
class's maximum, report `print_err`/`help2`/`int_error` and set `cur_val:=0`.
The command core carries that enumeration exactly once, as a
`RestrictedIntegerClass` and a single `scan_restricted_integer`; every
restricted scan selects a class rather than open-coding a range test. Only
§434's maximum is profile-dependent: TeX82's character domain is `0..=255`,
which the Unicode profile widens to the Unicode scalar values (§6.1).

The bound is part of the _scan_, never of the command that consumes it. TeX82
recovers `cur_val` before `shorthand_def` (§1224), `def_code` (§1232),
`def_family` (§1234), a math noad (§1151/§1154), or `\ifeof` (§501) ever reads
it, so the value the assignment commits -- and therefore every observation
derived from that assignment -- is already the recovered zero. Splitting the
two, so that a scanner returns the raw `scan_int` result and the assignment
clamps it afterwards, leaves the observation reporting a value the engine
never stored: `\chardef\x=256` recorded `character:256` against the reference
engine's `character:0` while the meaning itself was correctly `char_given 0`
(`umber2-johp.166`). The unrecovered value survives only where TeX82 prints
it: `int_error`'s parenthesized operand, and the `scan_int` observation that
precedes the bound and legitimately reports its own unclamped result.

`scan_something_internal` applies §433's `scan_eight_bit_int` bound to every
primitive register family (`\count`, `\dimen`, `\skip`, `\muskip`, and
`\toks`): the
index is scanned through ordinary expanded command delivery, and a negative
or greater-than-255 index recovers as register zero. A dimension-valued
register bypasses the numeric dimension backup/replay path and reaches the
internal result directly; a complete glue-valued register likewise bypasses
the trailing `plus`/`minus` scan. The observer records the resulting typed
internal integer, scaled, or glue value without leaking register storage.

`scan_something_internal` takes the requested level, not just the token, and
owns §429's `while cur_val_level>level` lowering cascade and §430's negation
itself. §413 has exactly one exit, reached after both, so the level it commits
-- and therefore the level the observer records -- is the level the caller
asked for, never the level the quantity happens to be stored at: `\fam\z@`
asks at `int_val`, and the `dimen_val` register `\z@` is lowered to an integer
holding the identical scaled representation before anything observes it.
Fetching in one function and coercing in each caller puts §429 on the far side
of the observation boundary and reports a scaled dimension where TeX82 reports
an integer (`umber2-johp.163`). A caller therefore names its level -- `int_val`
for §440, `dimen_val`/`mu_val` for §448 and §455, `glue_val`/`mu_val` for §461,
`tok_val` for §465's `the_toks` -- and never coerces afterwards. The three
outcomes §413 distinguishes stay distinct in the return type: the command is
not internal at all, §416's "Missing number" case (a font identifier or token
list requested below `tok_val`), or a committed value.

That bypass is not a register-family privilege. TeX82 §208/§209 make every
command code in `min_internal..=max_internal` an internal quantity, and
`scan_int` (§440), `scan_dimen` (§448), the internal-unit probe (§455), and
`scan_glue` (§461) all branch on exactly that range: the internal branch keeps
the token it already has, and only the other branch runs `back_input`. So a
`\dimendef` name, a named parameter, `\wd`/`\ht`/`\dp`, `\fontdimen`, a
`\chardef` constant, a page quantity, or any other internal quantity reaches
`scan_something_internal` with no backup level and no second delivery of its
own command. Restricting the retained-token branch to one primitive family
instead makes every other internal quantity push a backup, emit a recovery
record, and redeliver a command TeX82 delivers once (`umber2-johp.135`). The
scalar scanners therefore decide the branch by asking the shared internal-value
classifier itself, never by matching a per-primitive list: whatever that
classifier recognizes is retained, so wiring a new internal quantity into it
also puts it on the retained branch.

The classifier is exhaustive over `Meaning`: `None` means _only_ that a
named non-internal meaning lies outside §413's range. Host-unavailable
`\spacefactor`, `\prevdepth`, and `\prevgraf` commit their §418/§422 zero
values without entering missing-number recovery. Read-only internal integers
likewise remain internal: `\inputlineno` reads the live enclosing file line,
and e-TeX's current-group/current-if enquiries read the group and condition
stacks. e-TeX 2.6 `etex.ch` [17.4750--4790] negates `\currentiftype` for an
active `\unless`, while `\currentifbranch` remains a function of `if_limit`
alone. Adding a `Meaning` variant therefore requires an explicit scanner
classification.

`scan_dimen` delegates its integral prefix to that same integer scanner. Its
backed-up decimal point is then consumed raw, while a backed-up unit remains
available to the canonical keyword retry sequence.

That delegation is conditional, and the condition is a token `scan_dimen`
already holds rather than one it fetches. §448's non-internal branch reads

```text
back_input;
if cur_tok=continental_point_token then cur_tok:=point_token;
if cur_tok<>point_token then scan_int
else begin radix:=10; cur_val:=0; end;
if cur_tok=continental_point_token then cur_tok:=point_token;
if (radix=10)and(cur_tok=point_token) then <Scan decimal fraction>;
```

`back_input` does not disturb `cur_tok`, so a _leading_ decimal point never
enters `scan_int` at all, and §452's `get_token` -- unexpanded, "point_token
is being re-scanned" -- is the single delivery that re-scans it. Re-reading
the backed-up token through `get_x_token` and handing it to `scan_int` anyway
costs six semantic events per leading-point dimension: an expanded
redelivery, §444's `vacuous` scan, §446's second `back_error` backup and its
recovery record, and an integer result TeX never computes (`umber2-johp.177`,
`\vskip .5cm`).

Two consequences of the same shape are easy to reintroduce as hand-narrowed
tests. First, §448 has no digit test of its own: whether a number was there
is `scan_int`'s answer (§444's `vacuous`), which is why §444's `'` and `"`
radix introducers and §442's alphabetic constant are legal dimension prefixes
-- §448's own example is `-'77 pt`. Second, the fraction test is
`(radix=10)and(cur_tok=point_token)`, not the point test alone: §440
initializes `radix:=0` and only §444's decimal branch sets it to 10, so
`'77.5pt` has no fractional part. Both `point_token` and
`continental_point_token` are §445 token constants (`other_token` plus a
character), so recognizing a point is a category-code-12 test, never a bare
character comparison.

`scan_glue` first probes a
complete internal glue value; an ordinary numeric width is backed up and only
then passed to `scan_dimen`. These retry frames retire before their replacement
backup, preserving the TeX82 input lifecycle for whole-number dimensions,
physical units, and `fil` orders. An accepted internal dimension (or mu-glue
width for a mu dimension) is a unit: §455 scales both its integral and rounded
fractional parts through `nx_plus_y`/`xn_over_d`, rather than replaying it as
an unknown physical unit. In particular, after a fractional value, TeX82
§455's internal-unit, `em`, `ex`, `true`, and `pt` probes each own their failed
replay before `scan_keyword("in")` consumes `i` and `n` directly. The replay
adapter receives only the final `Scaled` or `GlueSpec` and never reconstructs
these transitions.

## 24. Static e-TeX and pdfTeX extension seams

Extensions are not runtime plugins. A profile installs a static primitive
catalog and a closed command enum.

The intended seams are:

| Extension             | Canonical seam                                      |
| --------------------- | --------------------------------------------------- |
| `\protected`          | macro flags and protected expansion policy          |
| `\unexpanded`         | `the_toks` family and expanded `scan_toks` splice   |
| `\detokenize`         | token-list conversion                               |
| `\expanded`           | balanced expanded token-list construction           |
| `\unless`             | conditional inversion                               |
| `\scantokens`         | generated registered source input                   |
| `\readline`           | virtualized raw-line token construction             |
| `\everyeof`           | source retirement                                   |
| expression scanners   | typed value scanner family                          |
| extended conditionals | condition predicate dispatch                        |
| extended registers    | `tex-state` value enquiries                         |
| environment enquiries | typed group, condition, node, font, and shape reads |
| e-TeX integer state   | named parameter and optional-feature writes         |
| pdfTeX conversions    | conversion dispatch                                 |
| pdfTeX file queries   | typed World enquiries                               |
| pdfTeX regex and MD5  | pure bounded services                               |
| pdfTeX random values  | deterministic World randomness                      |
| PDF object enquiries  | typed PDF ledger reads                              |
| PDF font enquiries    | typed font/PDF state reads                          |

Extension implementation may add opcodes and helpers but does not replace
`get_next`, `get_token`, `get_x_token`, `macro_call`, `pass_text`, or
`scan_toks`.

Compatibility mode is tested: enabling the e-TeX or pdfTeX build machinery
without entering its extended profile must not change TeX82 semantic traces.

## 25. Host capabilities and resource suspension

The command machine never performs native filesystem, network, clock,
terminal, or process access.

`CommandHostContext` exposes typed capabilities for:

- opening an input request into registered immutable backing;
- reading stream state through `World`;
- deterministic random values;
- PDF and font enquiries owned by aggregate state;
- staged terminal/log diagnostics; and
- optional dependency observation.

The immutable fixed job clock is already owned by `World`, so canonical
conversions read it through `CommandContext`'s precise tracked dependency
rather than duplicating it in the transient host capability set. In
particular, pdftex.web §1590's `\pdfcreationdate` and the LaTeX-compatible
`\creationdate` alias read the current job's clock after a format load and
return it through the ordinary conversion-token path.

A missing resource returns a typed `NeedResource`. No async future or callback
is stored in `CommandState`. The unfinished operation moves exactly once into
`PendingCommandAttempt<G, R>`, the sole in-session suspension package: it owns
the complete `AttemptArena`, one coarse `GenerationOwner<G>`, a non-`Copy`
operation capability, the typed request, and integer-only command, scanner,
expansion, and subordinate resume cursors. Resume consumes that package,
validates the generation, reinstalls the same attempt, and returns the same
operation capability without rescanning the request operands. A rejected
suspension likewise returns the still-live capability to its caller.
Cancellation drops the package wholesale. Before a suspension leaves the
session it detaches to the handle-free command-continuation schema; the
detached form contains logical recipes and DTO-local indices rather than the
arena, owner, or runtime coordinates.

For canonical `\input`, that negative response is evidence, not presentation:
the resumed opener reaches `open_registered_input` with the retained filename,
live input stack, and `CommandContext`. Only there can TeX82 §530 render the missing
name and `show_context`, acquire an interactive replacement, or enter §93's
fatal-history transition in batch/nonstop mode. A replacement that is not yet
registered suspends under its own ordinary `ResourceNeed::Input` identity; the
host does not receive a diagnostic callback or a second kind of absence
request.

The promoted scalar input/state slice keeps the same split concretely:
`CommandHostCapabilities` maps a logical `\input` name to already-acquired
immutable backing and supplies the immutable job name for the bounded
processor call. `\input` registers that backing before opening a source level.
`\endinput` sets TeX82 §362's process-global `force_eof`; only §360's next
real-file (`name>17`) refill consumes and clears it before retiring that file.
Intervening §483 terminal or `\read`/`\readline` pseudo-sources neither consume
nor clear the pending flag. TeX82 `end_file_reading` (§338) observes only that
retirement; the
test-only canonical adapter attaches the still-active logical source name
before removing its parallel trace frame, so source retirement preserves the
same identity as the corresponding push without putting host naming into
command state. Mark enquiries read the aggregate page-mark slots through
`CommandContext` and replay their immutable token lists. Scanner helpers do
not receive either host capability, so they cannot acquire sources or widen
the processor's authority.

Input streams follow the same transaction boundary. `\openin`, `\closein`,
`\read`, and `\readline` scan their stream number, optional equals, filename,
`to` keyword, and definition target into an `InputStreamRequest`; no raw
command or source cursor crosses to replay. The bounded replay borrows the
registered immutable bytes only to pin an input record in `World`, whose stream
state owns nested targets and line cursors. A missing `\openin` registration is
an input suspension, so unpublished semantic destinations remain unchanged
while the retained command attempt waits for the stable result.

At ordinary EOF, TeX82 §343 calls `end_file_reading`, checks outer validity,
and restarts raw delivery at the caller. Thus a nested `\input` retirement and
the parent source's normalized line-ending space can each occupy later
main-control operations before the parent delivers `\end`. The composed
canonical bridge must delegate that sequence to `MainControl`'s
command loop; it must neither consume legacy input nor impose a host-side
post-input step count.

Resource request order is deterministic and checkpointed where it can affect
future request identity. A failed attempt cannot leak input levels,
diagnostics, stream effects, random consumption, generated files, or
provenance.

## 26. Provenance

Provenance is always available and never semantic.

### 26.1 Token representation

The common `TracedTokenWord` remains compact. `OriginId` may encode a direct
source position or an index into a persistent provenance arena. Unknown origin
is a valid graceful fallback.

### 26.2 Direct source origins

Ordinary physical source tokens derive ranges from immutable registered bytes
without an arena write. Line normalization never claims removed terminators,
stripped spaces, or synthetic `endlinechar` bytes existed in the source.

### 26.3 Derived origins

Inserted, converted, substituted, or macro-expanded tokens may refer to:

```rust
struct DerivedOrigin {
    primary: OriginId,
    invocation: Option<InvocationOriginId>,
    operation: OriginOperation,
}
```

One macro call creates one invocation node containing call-site origin, macro
definition identity, macro name, and its parent invocation. Token derivations
share that node. Span operations allocate or intern origins in batches.

### 26.4 Backup identity

`DeliveryStamp` is not provenance. It proves an execution-local delivery and
is invalid after unrelated input movement or snapshot restoration.

### 26.5 Rollback and incremental history

Source registration and provenance use aggregate watermarks or persistent
roots. Step rollback discards uncommitted records. Accepted incremental
history shares sealed chunks and resolves editor fragments against the current
layout lazily. Deleted fragments resolve as deleted, never as a different live
position.

Packed arena origin keys are process-global diagnostic identities. Ordinary
rollback truncates records and retires their keys. A local atomic-step retry
leases the discarded registration and record sequences: it reuses a key only
when replay reaches the same allocation with the exactly equal source backing
and origin record, and abandons the remaining lease at the first divergence.
Thus redelivery of one rolled-back command retains diagnostic identity while a
detached `OriginId` can never alias different provenance. Restoring a retained
aggregate snapshot preserves every origin and record committed before its
watermark exactly. This diagnostic identity is excluded from all semantic
equality listed below. Repeated failed retries retain no live records and reuse
bounded arena capacity.

### 26.6 Semantic independence

These must remain identical with complete, degraded, or unknown origins:

- command traces;
- meanings;
- macro matching;
- condition evaluation;
- scanner values;
- state hashes;
- effects; and
- DVI/PDF output.

## 27. Dependency observation

Incremental read recording occurs at aggregate state-access boundaries rather
than at scattered primitive call sites.

The authoritative supported-region boundary, exhaustive family disposition,
fail-closed rule, and command/execution implementation split are defined by
[tracked_region_coverage.md](tracked_region_coverage.md). This section describes
the command-side access shape; it does not expand that region or authorize
replay.

`tex_state::CommandContext` observes typed reads such as:

- control-sequence meaning;
- catcode and other code-table values;
- integer, dimension, glue, and token parameters;
- registers;
- font metrics and selectors;
- box and page enquiries;
- condition and group enquiries;
- stream state;
- World file facts; and
- PDF ledger facts.

When recording is dormant, the check is a predictable inactive branch and
allocates nothing. When active, an observation records its typed key,
changed-at stamp, and semantic value. Validation uses the stamp fast path and
typed value fallback already established by incremental memoization.

Input transition recording occurs inside `InputState`, where exact cursor
movement and source identities are authoritative. Provenance does not stand in
for dependency identity.

One processor borrow admitted to an active outer region records versioned
`InputLine`, `InputStack`, and condition projections through `CommandContext`.
The projection hashes immutable source bytes, cursor scalars, semantic tokens,
control-sequence spellings, and stack order; it never hashes runtime source,
level, token-list, or provenance handles. Unsupported live continuations and
borrow-scoped host file lookups poison the outer region before their fact can
influence semantics. With no outer recorder, processor construction skips the
projection and barrier policy entirely.

## 28. Snapshots and durable summaries

There are two snapshot forms.

### 28.1 Direct operation cursor

Ordinary execution owns no `CommandStateSnapshot`. `Universe` exposes a
fixed-size, non-restoring direct-operation cursor for journal retirement and
private-allocation rejection; command state retains typed blocked
continuations at resource boundaries. Named incremental checkpoints remain the
only aggregate restoration authority. They preserve the exact committed
command and provenance prefix, while durable command summaries serialize no
second provenance arena or raw provenance watermark.

The command component of such a retained boundary is generation-generic. A
`CommandStateSnapshot<G>` and a live `CommandSummary<G>` each retain exactly
one coarse `CommandGenerationOwner<G>` beside a fixed `CommandSnapshotCursor`
of command-journal, arena-watermark, stack-length, and ordered-ledger
positions. Named-boundary publication explicitly clones the aggregate command
root once into that owner and binds it to the admitted state generation. The
live root remains exclusively mutable before and after publication. The
command timeline supplies only a monotonic identity serial; it owns no root or
per-checkpoint row. Cloning a retained value shares its private thread-confined
`Rc` owner and copies the cursor tuple; it does not clone the root again. The
generation and timeline capabilities keep their independent atomic owners.
Warmed delivery therefore performs no root admission, reference-count branch,
or aggregate clone. Restore validates the complete aggregate without mutation,
then explicitly clones the retained root into the live machine before
truncation, following the owner-before-roots-before-truncation ordering in
`runtime_storage_lifetimes.md`.

### 28.2 Durable summary

The generation-owned restartable rows selected by `CommandSummary` contain
only validated command data:

- source cursors and stable backing identities;
- token cursors and immutable payload identities;
- macro activations and live argument ranges;
- condition frames;
- quiescent alignment brace state, without active or suspended templates;
- the portable profile fingerprint and source-allocation high-water marks;
- persistent expansion counters; and
- required provenance/source roots.

The live summary reaches those rows only through its retained coarse owner and
fixed cursor. Detachment walks the selected roots once and replaces every live
name, source, token list, macro, glue value, provenance record, input frame,
and suspended-attempt root with an owned logical recipe connected by a dense
DTO-local index. The resulting continuation carries no live identifier, owner,
arena position, journal cursor, or borrow.

Host capabilities, `CurrentCommand`, delivery stamps, cache contents, spare
buffer capacity, timers, and profiling counters are absent.

Command resource fuel is absent as well. `CommandFuelLedger` is the distinct
monotonic run owner and lends its singular `CommandFuel` directly to every
`CommandProcessor` episode. The processor stores only that mutable borrow; it
has no owned-ledger alternative or per-charge ownership dispatch. The ledger charges
before each central raw-delivery attempt, including attempts nested beneath
expanded delivery, macro matching, and scanners. The same ledger counts raw
token-frame steps, completed expanded deliveries, live meaning lookups,
non-normal scanner-status tokens, and expandable commands executed inside a
deferred write. A failed semantic step may restore `CommandState`, but that
operation does not restore the fuel or work vector. These counters are host
telemetry and never enter snapshot, checkpoint, format, or semantic identity.
Canonical sessions default to 100,000,000 actions and accept only
`1..=1,000,000,000`; zero, larger values, and `u64::MAX` are typed
configuration errors rather than unlimited sentinels.

Summary restoration likewise validates the summary's profile fingerprint
before mutation. The same typed mismatch reports format and checkpoint
identity failures, so no persistent boundary can silently change a job's
dialect or character mode.

Durable named checkpoints are emitted only at executor-owned boundaries. They
do not capture a Rust stack continuation. Conditions and completed macro
activations may remain open when the boundary contract allows them. Summary
publication rejects conditional skipping, macro matching, definition or token
absorption, expansion episodes, alignment scanning or template delivery,
suspended alignment delivery, live semantic builders, rollback roots, and
stale scanner warning context. Scanner and transient domains are omitted from
the summary and reconstructed only in their unique quiescent forms.

### 28.3 Substrate acceptance gates

The initial command-state substrate is guarded by executable architecture
tests, not by a parallel compatibility facade:

| Invariant                                                                                  | Executable boundary                                                                 |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| `tex-state <- tex-command`, with no dependency on the retired command crates or `tex-exec` | `crates/tex-command/tests/it/boundaries.rs` manifest-direction test                 |
| crate-private state machines and opaque ownership fields                                   | compile-fail fixtures under `crates/tex-command/tests/ui/`                          |
| one explicitly classified field for each semantic ownership domain                         | exhaustive destructuring in `crates/tex-command/src/state/tests.rs`                 |
| host capabilities and call-local command values cannot enter owned serialized boundaries   | host and ephemeral compile-fail fixtures                                            |
| snapshots preserve all live semantic fields without runtime or host access                 | nonquiescent snapshot roundtrip in `crates/tex-command/src/snapshot/tests.rs`       |
| durable summaries reconstruct exact quiescent state and reject every nonquiescent class    | summary roundtrip and rejection tests in `crates/tex-command/src/snapshot/tests.rs` |

These gates audit the ownership substrate only. They do not supply command
semantics or an alternate API while the canonical state machines are still
being implemented.

## 29. Formats

Umber formats remain portable, validated semantic images rather than TeX
binary-memory dumps.

The format compatibility fingerprint includes:

- command-format schema;
- command dialect;
- character mode;
- primitive catalog version;
- token encoding version;
- meaning encoding version;
- code-table representation;
- frozen-command inventory; and
- dependent store schema versions.

`tex-command` exports its deterministic profile-fingerprint component and a
format-boundary validator. The aggregate format container combines this
component with the remaining schema and store identities above; a profile
mismatch is rejected before command state is installed.

Format loading installs immutable canonical macro, token, meaning, code-table,
font, and hyphenation bases, then creates fresh job-local `CommandState`.
Discardable caches are built lazily or in validated bulk and are never
serialized.

Reference formats are built independently in reference engines for behavioral
comparison. Umber does not claim binary compatibility with their `.fmt` files.

## 30. Errors and diagnostics

Semantic recovery and diagnostic presentation are separate:

Printable diagnostic values also preserve TeX's selector boundary. TeX82
§§59/262/296 makes `\meaning` collect `show_token_list` under
`new_string`, while `\show` prints a macro or mark list through the active
terminal/log selector; only the latter turns a character equal to the live
`\newlinechar` into a line break. The command-owned display freezes that
already selector-aware result for execution instead of rendering a
context-free `^^` spelling or rescanning the completed diagnostic text.

TeX82 macro tracing follows that same split. The command processor formats
§389's invocation and §400's completed arguments while it owns their live token
buffers, then queues non-error diagnostic values. At a completed processor
episode, `tex-exec` claims that existing `Vec` allocation wholesale and
consumes its values in order through §245's diagnostic scope; command state is
left with one fresh empty queue. No element collector or second diagnostic
representation crosses the boundary, so `\tracingonline` routing is selected
at the committed call rather than by the expansion layer.

- the command core chooses canonical recovery tokens and state transitions;
- typed diagnostics capture primary origin, related origins, macro invocation
  head, scanner status, command identity, and canonical diagnostic kind;
- the host formatter resolves paths, lines, columns, excerpts, and macro
  traces lazily.

Nested command episodes retain the same boundary. In particular, TeX82
§1370 expands deferred writes during shipout: scanner recovery is recorded in
command state with its live §82 context, then rendered into the artifact
transaction before the resulting write payload. Thus §367 expansion traces,
§418 internal-quantity diagnostics, §1372's unbalanced-write recovery, and
`token_show(def_ref)` retain their live `write_out` order while the whole
sequence stays rollback-safe. The expanded write bytes remain at the whatsit's
exact list position, so §§1373--1374 open and close effects cannot commit
around an absent write or materialize an empty numbered-stream artifact.
Deferred special and PDF-literal diagnostics retain their separate
post-transaction command-owned publication path. TeX82 §1043
extension whatsits in outer vertical mode enter
the page contribution list directly; leaving them on the mode nest delays
their write expansion past the page that canonically owns it.

Batch/nonstop transcript fixtures compare canonical wording and order where
the project claims transcript parity. Host-specific paths, banners, terminal
interaction, and display widths are normalized only through explicit fixture
rules.

TeX82 §310 error-context selection is part of the command input-stack walk,
not a post-projection filter. The walk pseudoprints the current level and the
nonnegative `\errorcontextlines` budget immediately, remembers only the newest
remaining level as the possible `bottom_line`, and projects that level after
the walk. It emits §310's ellipsis only when at least one level lies between
the immediate prefix and the bottom. Thus §§312--315 never construct owned
strings for an omitted level; `tex-state` retains only §§316--318's shared
two-line renderer for each selected projection.

The canonical hundred-error termination and explicit expansion/resource
budgets prevent unbounded recovery. Limits are versioned engine policy and
must not create an alternative successful result inside the reference domain.

## 31. Reference oracle and conformance

Correctness uses pinned TeX82, e-TeX, and pdfTeX engines, never the retired
implementation.

The reference transport writes TeX's `cur_chr` for every command. For
`call`, `long_call`, `outer_call`, and `long_outer_call`, that field is the
definition-head reference (`def_ref`). `tex-state` retains the corresponding
definition-owned observation identity beside its immutable macro definition:
`\let` aliases therefore expose the same operand, while separately scanned
definitions expose distinct operands. The offline command-stream comparator
retains its established reference-address projection because its isolated
replay does not model unrelated TeX allocator traffic. The integrated TRIP
observer does not apply that projection and compares the definition identity
exactly. Runtime handles and snapshots remain independent of the detached
observation value.

The fixture stream adapter translates executor-committed meaning mutations
using the assigned control-sequence key captured at that seam. It never
reconstructs the key from a generic mutation category, so `\let` aliases of
explicit grouping primitives remain comparable while command state stays the
sole owner of delivery and operand scanning.

For TeX82 `handle_right_brace` §1132, replay alone selects the structural
`align_group` branch. The command processor then owns `back_input`, its
literal-brace backup correction, and insertion of immutable frozen `\cr`;
the executor neither manufactures nor replays those raw tokens. This preserves
the §1127/§1132 recovery ordering before v-template delivery.

Expanded balanced general-text collection enters the command-owned absorbing
scanner episode before it delivers its required opening brace. The brace is
therefore observed once under the live scanner status; ordinary token-list
assignment scanning retains its canonical initial validation and replay.

Executor-owned `\message` application captures its completed text before the
processor borrow ends and emits the terminal-byte effect only after the write
commits. The stream adapter keeps that effect as terminal bytes, not a
fixture-derived textual reconstruction.

Command-observer identities preserve TeX's command/`cur_chr` pairs rather
than Rust storage discriminants. In particular, the installed `\par` primitive
observes as `par_end` with `cur_chr = 256`, while explicit `\begingroup` and
`\endgroup` observe as `begin_group` and `end_group`, each with `cur_chr = 0`.
Likewise `\setbox` observes as `set_box` with selector `0`.
Their internal primitive-enum operands are not part of the canonical trace.
This keeps paragraph and group delivery comparable without exposing
command-state representation in snapshots or fixtures.

Incomplete conditional recovery follows TeX82's `back_error` ordering: it
backs up the encountered delimiter, pushes an inaccessible frozen `\relax`,
records the stable `conditional_limit_recovery` diagnostic, then resumes the
operand scanner. The frozen token remains immutable command state, while the
detached observer projects its canonical spelling as `\relax`. Scalar scanner
replay retires an exhausted inserted-recovery frame before backing that token
up again. Filename scanning likewise replays its first non-space token through
the sole input path before consuming the name. These transitions are
snapshot-owned input state, never fixture-adapter reconstruction.

The same ownership applies when a tab, `\span`, `\cr`, or `\crcr` reaches main
control instead of a v-template. `get_next` (§342) only diverts a delimiter
into a template when `align_state = 0`, so every other occurrence -- inside an
alignment cell whose braces are unbalanced, and outside any alignment at all
-- is §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`. Main
control routes all four spellings, plus a category-4 or category-5 character
token, through the single `CommandProcessor::recover_align_error` entry point,
which implements §1127 in full: `abs(align_state) > 2` selects §1128's report
and drop, and anything nearer base depth backs up that exact delimiter and
uses `ins_error` to place the balancing brace above it. `\noalign` and `\omit`
take §1126's other two lines, whose §1129 routines report and ignore.

`tex-command` publishes raw and expanded delimiter delivery before this typed
recovery, owns both backup corrections and the inserted recovery level, and
replays the brace before it re-intercepts the original delimiter. `tex-exec`
receives only whether the inserted brace opens a recovery simple group, and
cannot inspect or alter the underlying token/input ordering. Raw delivery
never intercepts an out-of-template delimiter on main control's behalf:
tex.web reaches `align_error` from `main_control`, and a delivery-boundary
interception would have to re-derive §1127's `align_state` test in a second
place.

### 31.1 Instrumented engines

Dedicated reference executables are built from pinned canonical WEB and
upstream change files plus a final Umber-owned tracing change file. Canonical
sources are not edited.

For TeX82, `scripts/regen-fixtures.sh --oracle tex82 --profile
initex-eight-bit` owns acquisition
and delegates to the reproducible workflow documented in
[`tex82_oracle.md`](tex82_oracle.md). It emits separately identified clean and
instrumented Web2C executables plus a build record. The pinned final change
file emits TeX82 raw/expanded delivery, input lifecycle, backup, scanner
status, outer-validity recovery, completed macro arguments and activations,
delimiter matching and overlap recovery, parameter conversion, macro
replacement completion, `scan_toks` completion and direct `\the` splices,
committed integer, dimension, glue, and internal scanner results, condition
push/limit/branch/pop transitions, skipped-delimiter and `\ifcase` progress,
conditional recovery, alignment preamble and nesting lifecycle, `align_state`
and backup correction, delimiter interception, template push/retirement,
alignment recovery, terminal-stop, and termination events. Its focused live
run is schema-validated against the executable semantic-event matrix,
deterministic, and byte-transparent for terminal, normalized log, status, and
DVI output. The final TeX82 change also observes assignment-scoped committed
meaning, catcode, code-table, parameter, and register writes, plus committed
message, expanded-write, stream-open/close, and successful-shipout effects.
Transparency includes exact generated write-file bytes.

A stream-open effect carries the opened file name, because §1374's open branch
assigns `cur_name`, `cur_area`, and `cur_ext` from the whatsit before packing
it. A stream-close effect carries no value at all: §1374 closes `write_file[j]`
without naming it, and TeX keeps no name for an open stream, so §1378 closes
the survivors at job end the same way. Instrumentation must not read the
file-name globals there -- they still hold whatever was packed last, such as
the most recent `\input` or the job name §529's `pack_job_name` installs for
the log or DVI file -- and must not shadow the open target in engine state TeX
does not have. The stream number in the effect's channel is the whole of a
close's committed identity.
Cargo correctness tests never acquire or execute this live oracle.

For e-TeX 2.6, `scripts/regen-fixtures.sh --oracle etex26 --profile
compatibility+extended-eight-bit` owns
acquisition and delegates to the reproducible workflow documented in
[`etex26_oracle.md`](etex26_oracle.md). It emits separately named clean and
instrumented executables for the canonical compatibility and
extended INITEX profiles, plus a complete build record. The profile distinction
is the canonical leading-`*` INITEX input contract, not a compile-time fork.
The final repository change ports the complete TeX82-applicable schema-v1
semantic matrix through stable e-TeX seams. Its extension matrix additionally
observes committed token construction and suppression, expression results,
extended predicates and inversion, group/condition/value enquiries, canonical
sparse-register writes, and named tracing/state parameters. Sparse storage
nodes and allocation identities are excluded. A machine-readable audit
classifies every primitive declared by the pinned canonical `etex.ch` as
command-core-owned or executor-owned; every command-core entry names an exact
extension-matrix boundary, while executor entries name their existing focused
parity gate. Both profiles are smoke- and
matrix-gated; their traces are schema-valid and deterministic, and clean versus
instrumented terminal, normalized log, status, DVI, and generated-effect bytes
must match. Compatibility mode retains the TeX82 shared-domain contract while
the extended profile exercises the same base boundaries under e-TeX's
canonical primitive installation. Cargo correctness tests never acquire or
execute this live oracle.

For pdfTeX 1.40.29, `scripts/regen-fixtures.sh --oracle pdftex14029 --profile
initex-etex-eight-bit`
owns acquisition and delegates to the reproducible workflow documented in
[`pdftex14029_oracle.md`](pdftex14029_oracle.md). It emits separately named
clean and instrumented exact-eight-bit executables from the pinned
canonical WEB program, ordered Web2C/SyncTeX stack, and archive-owned library
inputs. Ordered repository-owned final changes port the complete shared
TeX82/e-TeX schema-v1 command matrix and observe pdfTeX command-core
expansion/scanner extensions, named state mutations, object-independent
enquiries, and committed PDF-facing effects through stable seams without
editing upstream files. A machine audit accounts for all 549 canonical
primitive declarations: 391 are shared TeX/e-TeX entries and 158 pdfTeX
additions are classified as command-core or executor/backend. Every
command-core addition is exercised by the expansion or state matrix; the state
trace excludes backend allocation identity, and records the host-dependent
timer enquiry as a stable semantic boundary. The build record captures source,
change, profile, translator, toolchain, library, platform, executable,
smoke-output, transition-output, extension-output, state-output, and trace
identities. Font-independent DVI and deterministic PDF smoke programs plus
focused shared, extension, and state programs must be byte-transparent between
variants; all instrumented traces are schema-validated, matrix-gated, and
repeatable. Expansion-matrix rows name their owning primitive and are checked
bidirectionally against the complete primitive audit. Deterministic smoke and
state PDFs are also parsed through the independent bounded Hayro probe; their
allocation-insensitive semantic projections must agree between clean and
instrumented runs, and the repeated state run must reproduce its projection,
while PDF structure remains absent from command events. Cargo correctness
tests never acquire or execute this live oracle.

Instrumentation writes a versioned semantic event stream. It must not use
TeX's semantic `mem`, string pool, selector, transcript state, or command
input.

Every instrumented executable is proven transparent by comparing its ordinary
output with the corresponding uninstrumented executable.

### 31.2 Semantic events

Events describe committed semantic transitions, not Pascal helper entry or raw
pointer movement. The schema includes:

- command delivered by `get_next`;
- expanded command delivered to a caller;
- input level push, retirement, or stop;
- backup and recovery insertion;
- scanner-status transition;
- completed macro argument and macro activation;
- condition push, limit change, branch, and pop;
- scanner result;
- `scan_toks` splice and completion;
- alignment-state and template transition;
- typed state mutation relevant to the command machine; and
- final diagnostic/effect ordering.

Values use control-sequence spelling, character code and catcode, canonical
command/operand names, input reason, source-relative location where stable,
and condition/alignment state. They exclude reference allocation identities.

Schema version 1 is implemented by the dependency-light `tex-oracle` crate.
It is shared by all three reference harnesses and the detached `tex-command`
observer; it does not depend on either command engine. The
canonical event union is:

| Event            | Committed semantic boundary                                        |
| ---------------- | ------------------------------------------------------------------ |
| `command`        | raw `get_next` or expanded caller delivery                         |
| `input`          | logical-level push, retirement, or terminal stop                   |
| `recovery`       | backup or error-recovery insertion                                 |
| `scanner_status` | scanner-status transition                                          |
| `macro`          | completed argument or macro activation                             |
| `condition`      | condition push, limit change, selected branch, or pop              |
| `scanner`        | completed typed scanner result                                     |
| `token_list`     | `scan_toks` splice or completion                                   |
| `alignment`      | `align_state`, template, or delimiter transition                   |
| `mutation`       | typed command-relevant meaning, code, parameter, or register write |
| `diagnostic`     | ordered typed diagnostic                                           |
| `effect`         | ordered externally visible message, I/O, shipout, or final effect  |

Tokens contain character code, canonical catcode name, optional
control-sequence spelling, and an optional manifest-source-relative line and
byte. Commands contain canonical command and operand names rather than numeric
WEB command codes. Typed values cover integers, character codes, scaled
values, glue including orders, tokens, names, and exact bytes. No event field
can name a Pascal address, `mem` node, string-pool index, selector, transcript
position, input-stack index, or physical path.

The deterministic encoding is compact UTF-8 JSON Lines. The header contains
the schema number and fixture-manifest identity. Each later line contains a
zero-based sequence number and one normalized semantic event. Schema field
order is fixed, maps are key ordered, and line endings inside semantic strings
are normalized from CRLF or CR to LF. Normalization does not reorder events,
renumber engine identities, rewrite source names, or hide semantic values.
The stream identity is SHA-256 over a domain tag, schema version, and the exact
encoded bytes.

The fixture manifest identity uses a separate SHA-256 domain and covers the
schema; engine dialect and banner; canonical WEB source, ordered upstream
change files, and final instrumentation change file; stable logical input
names and exact hashes; environment, epoch, clock, and random seed;
distribution hash; and hashes of each ordinary-output channel. Manifest input
names must be logical names rather than absolute or traversal-bearing host
paths.

Reference instrumentation calls `EventObserver::committed` only after the
described semantic transition commits and passes a fully owned `Event`.
`JsonLinesObserver` owns a dedicated writer that is never a TeX selector,
transcript, or ordinary output. Ordinary builds use the zero-sized
`DisabledObserver`; build integration may compile calls out entirely. Neither
transport is allowed to query engine storage.

The command core owns its observer record surface rather than
depending on `tex-oracle`. `CommandObservation` carries command delivery,
logical input, recovery, scanner-status, macro, condition, typed scanner,
token-list, alignment, mutation, and effect records. Command deliveries carry
an opaque origin identity and optional exact registered physical source range
plus its typed canonical location, alongside input-level, cursor-slot, and
processor-delivery provenance. Raw source
delivery installs its backing through the aggregate source-map boundary before
spelling construction; expanded and replayed delivery retain that traced
origin without fixture-derived locations. All other records retain only command-owned
typed values and stable identities. The fixture adapter maps those
owned records to a deterministic replay capture outside the production command
dependency graph. It validates the locked TeX82 suite, scans the terminal root
filename through the command processor, opens only that selected root, and
drives command/executor execution through `MainControl` with a
source-byte-derived bound. Remaining manifest sources are registered only as
immutable `\\input` capabilities, so nested input remains command-owned;
ordered schema comparison remains separate.
TeX82 diagnostic consumers retain their own command-demand boundary within
that replay: §46's `\\show` obtains its displayed token through raw
`get_token`, so a shown macro is observed but never enters macro matching or
replacement replay. The executor receives only the completed diagnostic
operation, not a second input path.
Observers are non-fallible, receive records only after the transition commits,
and are neither retained in `CommandState` nor captured by snapshots.
The observation vocabulary and sites compile unconditionally; there is no
observation Cargo feature. Each site first tests the processor episode's
runtime predicate, which is true exactly when an external observer is attached,
and does not construct its record when the predicate is false. Main control
selects that predicate once for an atomic operation and gives every processor
episode it creates, including nested episodes, the same operation-local buffer.
It offers the buffered records to the external observer in order only after the
operation commits; rollback and resource suspension discard them before retry.
Optimized span paths decompose into the same scalar events when an observer is
attached.

### 31.3 Fixture tiers

The conformance hierarchy is:

1. focused command semantic traces from INITEX;
2. reference-visible `\message`, `\meaning`, `\show`, transcript, and
   recoverable-error fixtures;
3. TRIP and e-TRIP transcript/log/DVI gates;
4. primitive-family pdfTeX fixtures;
5. independently built plain/e-TeX/LaTeX/pdfLaTeX formats;
6. normalized DVI byte parity;
7. canonical PDF structure, text extraction, and rendering parity; and
8. real-document corpus parity.

Committed fixtures record the version-1 manifest and event stream described
above. Repository fixture contract v1 additionally binds the exact profile,
generation tools, mandatory canonical citations, focused INITEX source files,
and ordinary artifact observations without changing the schema-v1 manifest
preimage. The contract and first TeX82 fixture are documented in
[`command_semantic_fixtures.md`](command_semantic_fixtures.md). Live
regeneration goes only through supported `scripts/regen-fixtures.sh` modes.

For a composed Story, Gentle, TRIP, or e-TRIP failure, the committed canonical
semantic trace is the primary convergence oracle whenever that profile and
source are available: compare it in sequence and repair the first divergent
event against the pinned TeX82, e-TeX, or pdfTeX trace. Final DVI, transcript,
and provenance checks remain mandatory acceptance gates, but are too late and
too aggregate to identify a command-core ownership error. The retired Umber
pipeline is never an event or behavioral oracle.

### 31.4 Incremental and provenance proof

Reference engines cannot validate incremental machinery directly. The proof
chain is:

```text
reference TeX/e-TeX/pdfTeX
        ==
cold new Umber
        ==
restored or incremental new Umber
```

Cold and incremental Umber additionally compare semantic state, ordered
effects, diagnostics, artifacts, dependencies, provenance queries, and
checkpoint schedule.

Unicode-only behavior uses explicit Umber specifications and property tests;
it is not compared to an 8-bit engine outside their shared domain.

The character/input audit binds all three exact profiles to the committed
TeX82 source-token fixture's shared-domain M/N/S, ignored/comment, end-line,
blank-line, and control-sequence expectations. Exhaustive byte and Unicode
scalar encoding tests, explicit Unicode-extension token/range fixtures, all
profile-pair mismatch checks, and architecture gates for immutable profile
selection and host-free registered-source delivery complete the executable
coverage. Unicode fixture expectations remain Umber specifications and are
never presented as pdfTeX behavior.

## 32. Performance architecture

The canonical implementation uses bounded mutable semantic episodes as its
ordinary internal unit of work. The measured architecture decision in
[Canonical Engine Architecture Decision](engine_architecture_decision.md)
does not authorize a runtime engine selector, duplicate state, or omitted
observable semantics; its typed barriers and promotion gates refine the
optimization rules below.

The first promoted implementation is the simplest canonical scalar machine.
Its performance foundations are:

- one raw command loop;
- one ordinary expanded loop;
- dense input levels;
- compact token and meaning words;
- direct source origins;
- contiguous shared macro-argument buffers;
- no hot-path trait objects;
- no push-result round trip;
- no parallel traced/untraced interpreter;
- no per-scanner generic state facade;
- cold typed error and resource paths;
- immutable stored token lists; and
- the measured bounded process-local traced-token scratch pool described in
  section 9.

The raw and expanded loops are separately compiled specializations beneath
one typed delivery-policy entry point. Their input mutation and restart rules
remain canonical, but the expanded hot loop does not branch on a raw-mode
option for every token and the raw loop does not carry expanded-only policy.

### 32.1 Optimization promotion

An optimization may be introduced only after the scalar implementation passes
the relevant canonical parity gate. Examples include:

- compiled macro delimiter plans;
- macro-body meaning-site caches;
- source or macro text spans;
- scanner keyword fast paths;
- specialized template replay;
- batched provenance derivation; and
- top-input register layouts.

Promotion requires all of:

1. identical semantic oracle events with an external observer attached;
2. identical diagnostics, effects, DVI, and PDF fixtures;
3. a mechanically obvious equivalence boundary inside the canonical machine;
4. no new semantic state in a discardable cache;
5. focused microbench evidence explaining the expected win;
6. controlled whole-workload profiling under `docs/profiling.md`;
7. a statistically credible improvement in the targeted workload;
8. no material regression in snapshot capture, rollback, retained memory, or
   WASM behavior; and
9. removal of the prototype if the measured ceiling is negligible.

Performance counters and observer hooks do not write to semantic state.
Observer hooks retain only a predictable inactive runtime branch and construct
no records unless an external observer is attached; their vocabulary remains
compiled in every build.

## 33. Main-control integration

`tex-exec` owns main control. Each bounded episode repeatedly gives
`CommandProcessor` its operation-local `Option<CurrentCommand>` destination,
dispatches the unexpandable command written there, and performs stomach
semantics under one aggregate rollback root. Raw preflight, ordinary expanded
delivery, main-loop lookahead, alignment delivery, prefix scanning, leader
handoff, and in-place reswitch all use that same caller-owned destination
shape; the returned status carries no command.

Execution may call narrow processor APIs to:

- push `\every...` token input;
- begin or finish alignment lifecycle transitions;
- scan assignment values;
- construct definitions;
- read balanced general text;
- start input requested by an unexpandable command where canonical;
- publish or restore command summaries; and
- query diagnostic input context.

Execution cannot:

- read lexical tokens below `get_next`;
- inspect or mutate input cursor indexes;
- resolve `OutParameter`;
- store condition frames on input;
- classify alignment delimiters;
- implement backup independently;
- choose an expansion mode trait;
- attach sticky suppression to tokens; or
- access raw provenance stores.

An explicit `CurrentCommand` is delivered once. If its operand scanner
suspends, main control moves the command, delivery cursor, non-`Copy` scanner
child, and non-`Copy` operation capability into one typed retry destination.
Alignment dispatch records the substantive command destination before calling
its scanner; alignment itself remains the destination only when suspension
occurred while alignment still owned delivery. Retry therefore resumes the
exact caller rather than fetching past a settled command or reconstructing an
owner coordinate from command state.

The call-local destination is empty on entry, is cleared only while a
canonical filler loop continues, and never crosses a resource barrier.
Preflight settlement consumes the command already in that slot and writes its
settled replacement back into the same slot; it does not back up or redeliver
the command. Replay completion and alignment events remain compact status
variants and leave no command-bearing return envelope.

Each bounded `MainControl` episode settles commands through the sole live
command, mode, Universe, output, and World owners. A command-core
`MissingInput` becomes a typed suspension carrying its prepared continuation;
observer records remain buffered until structural application commits. Retry
resumes that continuation without reconstructing a delivered command or
rolling back an already committed ordinary prefix. Host capabilities remain
borrow-scoped, so supplying a resource changes only the resumed operation's
capability set.

There is no coverage fallback. Every TeX82, e-TeX, and pdfTeX meaning enters
this loop, including definitions, assignments, groups, alignments, mode and
node construction, page/PDF output, resources, effects, diagnostics, and
retry. Required semantic barriers and typed rollback outcomes use this same
state machine; group entry and exit are not episode stops, and no boundary
selects another dispatcher.

### 33.1 Canonical main-control coverage matrix

This is the closure inventory for Beads
`umber2-johp.8.5.1.1.1.4.1.7`. TeX82 is the semantic authority: §§24--25 own
delivery and expansion, while §1030 and its mode tables own the consumer.
The pinned pdfTeX 1.40.29 source boundary is
[`pdftex_primitives.md`](pdftex_primitives.md), not the former executor. A
retired executor/input-stack occurrence is therefore a regression; it is never
a behavioral oracle. Routing
direct, CLI, virtual, editor, and incremental callers is deliberately outside
this matrix and remains Beads `.4.2`.

| Canonical family                                                                                   | WEB authority                                                                                                        | Canonical owner and committed test inventory                                                                                                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Delivery, expansion, recovery, and scanner state                                                   | TeX82 §§24--25; §1030                                                                                                | `CommandProcessor::{get_next,get_x_token}` is the only source path; `tex-command` raw/expanded delivery, macro, conditional, and structured-scanner tests plus `tex-exec` architecture gate.                                                                                                                |
| Mode selection, groups, prefixes, definitions, registers, parameters, code tables, and diagnostics | TeX82 §1030+ mode tables; §§1064, 1066, 1095, 1131; §1257 (`new_font`)                                               | Typed `ColdOperation` application; `assignments_*`, `canonical_definition_*`, `canonical_font_definition_*`, `canonical_grouping_*`, and `canonical_display_diagnostics_*`.                                                                                                                                 |
| Paragraph and horizontal material                                                                  | TeX82 §1030+ horizontal/vertical branches                                                                            | Typed paragraph start/backup, `\\everypar`, characters, spaces, ligatures, accents, discretionary, kern/glue, parshape, line breaking, and page contribution; `production_driver_*paragraph*`, `production_driver_*horizontal*`, and `paragraph_page_builder_is_observer_neutral`.                          |
| Boxes, leaders, packing, splitting, and explicit shipment                                          | TeX82 §1071 (`box_end`/`\\shipout`) and §1030+ box branches                                                          | Typed box construction/selection/unboxing/leaders/vsplit; `canonical_box_*`, `canonical_vsplit_*`, `canonical_initex_replay_scans_setbox_*`, and `shipout_box_completion_*`.                                                                                                                                |
| Math and display material                                                                          | TeX82 §§691--734 and §1030+ math branches                                                                            | Typed `MathRequest`/`MathDelimiterBoundary`; `canonical_math_*`, `canonical_math_replay_observer_does_not_change_frozen_mlist`, and math scanner recovery tests.                                                                                                                                            |
| Alignment preamble, cells, templates, recovery, and final packing                                  | TeX82 §§760--780; §1131 recovery                                                                                     | Command-owned `AlignmentDelivery`; `canonical_alignment_*`, `command_owned_endv_*`, `nested_alignment_*`, `noalign_*`, and committed TeX82 alignment fixtures.                                                                                                                                              |
| Input, font, and image resources                                                                   | TeX82 §529 (`start_input`/filename scan), §1254; pdfTeX `scan_image`/`scan_pdf_box_spec`                             | Immutable typed suspension only; `missing_canonical_input_*`, `canonical_missing_font_*`, `canonical_openin_*`, `canonical_pdfximage_*`, and `canonical_pdf_resource_retry_*`. Host-byte fulfillment is a deliberate modern envelope, not a TeX82 semantic substitute.                                      |
| Page building, output routines, DVI/PDF lowering, streams, effects, and artifacts                  | TeX82 §46, §§608--642, §§1006--1028, §1071, §§1337--1338; pdfTeX `pdf_ship_out`, `hlist_out`/`vlist_out`, `out_what` | Typed page/output lifecycle and detached receipts; `canonical_*shipout*`, `canonical_stream_effects_*`, `canonical_pdf_whatsits_*`, and `tex-out` artifact/DVI tests.                                                                                                                                       |
| pdfTeX extension registry and PDF semantic families                                                | pdfTeX 1.40.29 registration blocks and routines pinned by `pdftex_primitives.md` (158 exact names)                   | One static profile dispatch over the same command machine; source-derived 158-name inventory, `pdftex::*`, `pdf_output::*`, graphics/navigation/font fixtures, and typed image retry coverage. Engine-neutral aliases are documented there; no new extension boundary is introduced here.                   |
| Fresh/format-loaded session and observer equivalence                                               | TeX82 INITEX/format model; §§24--25 and §1030+                                                                       | `format_and_checkpoint_profile_components_reject_mismatch`, `retained_observer_captures_fresh_and_format_loaded_production_runs`, and fresh/loaded scanner and execution cases. Observers buffer only committed records from the same processor episode; they do not replay or synthesize command delivery. |

Paragraph hyphenation preserves TeX82 §929's `new_character(hf,hyf_char)`
diagnostic as typed evidence alongside the detached hyphenated node sequence.
The line-break trace renderer publishes that evidence when traversal reaches
the corresponding automatic discretionary. This retains §581's exact warning
order even though Umber constructs the hyphenated sequence before its pure
line-break pass; an out-of-range §923 hyphen character still produces neither
a node nor a warning.

The architecture test in `crates/tex-exec/tests/it.rs` rejects retired command
crates, alternate input-stack delivery, executor dispatch, and compatibility
fronts across the tex-exec tree. It requires the typed
`CurrentCommand::meaning()` dispatch boundary and fixes one
aggregate snapshot/rollback implementation. The rollback snapshot includes
command state, mode nest, Universe, active
box/alignment/output state, artifacts, and effect roots. `ObservationBuffer`
is a transaction-local commit buffer, not cached command state: a failed
operation rolls it back without publication, then retries with a fresh
processor episode.

### 33.2 Dispatch-completeness invariant

`main_control.rs`'s `scan_command` is the sole place an
`UnexpandablePrimitive` reaches stomach dispatch. Every variant must be either
routed by a named arm (directly, or through an explicit generic path such as
`scan_math_request` for the math-noad family) or must fail loudly
and name itself. A silent catch-all that treats "no dispatch arm" as
"succeeded and consumed nothing" is a standing defect, not a placeholder: main
control has already consumed the primitive's own token, so if its scanner
never runs, any mandatory operand (an integer, dimension, glue, or braced
group) is left in the input stream and gets typeset as literal document text
arbitrarily far downstream of the real gap. `\patterns`/`\hyphenation`
(Beads `umber2-johp.67`), `\penalty` (`umber2-johp.68`), `\␣` (control space),
and `\prevdepth` (both `umber2-johp.73`) were each found only after silently
corrupting output this way; `umber2-johp.69` replaced the wildcard with the
mechanism below after a survey found roughly half of
`tex_state::meaning::UnexpandablePrimitive`'s ~266 variants had no named arm.

**Mechanism.** `scan_command`'s final match arm no longer reads
`_ => Ok(ColdOperation::Continue)` for `Meaning::UnexpandablePrimitive`. Instead
it delegates to `scan_unclassified_primitive(primitive, mode, origin)`, which
is written as an **exhaustive match over the complete `UnexpandablePrimitive`
enum** (not just the variants that currently lack a dispatch arm), split into
two buckets:

- `unreachable!()` for every primitive that already has an explicit,
  mode-complete dispatch arm earlier in `scan_command` (including the early
  math-family gates before the main match). Listing these here, rather than
  relying on the earlier arms alone, is what makes the match exhaustive: if a
  later edit narrows or removes one of those arms without updating this
  classifier, the primitive falls through to its `unreachable!()` arm and
  panics instead of silently reverting to swallowed-primitive behavior.
- `Err(ExecError::UnimplementedPrimitive { primitive, mode, origin })` for
  every primitive with no dispatch yet, or dispatched only conditionally
  elsewhere (the math-noad family, or `\left`/`\right`/`\middle`'s
  math-delimiter gate) and reached outside that context.

Because the match is exhaustive over the enum itself, **adding a new
`UnexpandablePrimitive` variant fails to compile** in
`scan_unclassified_primitive` until it is deliberately placed in one of the
two buckets -- this compile-time property, not just the runtime error, is the
main deliverable: it prevents this exact defect from being reintroduced by a
future primitive addition. `ExecError::UnimplementedPrimitive` follows the
`umber2-johp.59` precedent of `ExecError::Command(CommandError)`: a named,
typed failure that carries the offending variant (via `{primitive:?}`) rather
than a generic message, so the run stops at its true site.

Reaching an `UnimplementedPrimitive` failure is expected and correct for
primitives that have not been routed yet; it is a precise, individually
fixable divergence, not a regression to soften. Beads `umber2-johp.69`'s
closing comment inventories which variants fall into the loud bucket, grouped
by whether Story, Gentle, or Plain reach them, for follow-on work.

**The same invariant applies one level up the meaning word.**
`umber2-johp.69` scoped itself to `UnexpandablePrimitive`, leaving every other
`Meaning` variant (`CharToken`, register/parameter accessors, `Relax`,
`Undefined`, `PageDimension`, and so on) to an ordinary
`_ => Ok(ColdOperation::Continue)` fallback -- which then became the active
hiding place, since it turns "not implemented" into "succeeded and consumed
nothing" just as effectively (`\pagegoal=100pt` silently typeset `=100pt` as
document text; Beads `umber2-johp.106`). `umber2-johp.108` removed it:
`scan_command`'s final arm now delegates to `scan_unclassified_meaning`, an
**exhaustive match over `Meaning`** which in turn delegates its `CharToken`
case to `scan_unclassified_char_token`, an **exhaustive match over
`Catcode`**. Both use the same buckets as `scan_unclassified_primitive`, plus
one more:

- `Ok(...)` for meanings tex.web routes somewhere in main control
  already implements generically, cited per arm. This bucket is where
  "consume nothing and proceed" is now allowed to appear -- but only as a
  named, tex.web-cited decision (`any_mode(relax): do_nothing`, §1045), never
  as an anonymous wildcard.
- `unreachable!()` and
  `Err(ExecError::UnimplementedMeaning { meaning, mode, origin })` exactly as
  above.

Adding a new `Meaning` variant, or a new `Catcode`, therefore fails to compile
until it is deliberately placed in a bucket. `scan_command` has no wildcard
arm that resolves to a `ColdOperation` any more, at either level.

### 33.3 Canonical observation vocabulary

The same invariant applies to _naming_, not just dispatch. Every string an
observation payload carries for a concept -- a category code, a character
command, a scanner status, a glue order, a token's catcode or spelling, a
meaning's command name -- is spelled once, in
`tex_command::canonical_names`, and is re-exported so that producers in
`tex-exec` and the differential tracer use the same table instead of each
keeping their own.

Scalar, mutation, and effect payload structure is typed before this naming
boundary: mutations carry their state domain, key, value, and scope
separately, while effects carry kind, channel, and value. Portable observers
map those fields directly and never decode numeric text, prefixes, or embedded
separators.

This exists because three consecutive `umber2-johp` root causes
(`.134` parameter names, `.140` catcode names, part of `.135`) were naming
defects rather than engine defects: Umber behaved correctly and only the
emitted name disagreed with tex.web's. Each cost a full agent run to find one
at a time, and each masked real divergences behind it. `umber2-johp.141`
enumerated the whole vocabulary in one pass and found five families wrong.

The durable rules, with rationale, live in `crates/tex-command/AGENTS.md`
under "Canonical Observation Vocabulary". The two that are easiest to
reintroduce:

- A Rust `Debug` rendering must never reach an observation payload, and must
  never round-trip through one either. `Debug` spells Umber's variant names
  and the oracle spells tex.web's, so any agreement between them is
  accidental and silently breaks when a variant is renamed.
- A transport must never re-derive a name the producer already computed.
  A second table always drifts, and while it drifts it hides exactly the
  engine divergences the producer's name would have exposed.

### 33.4 Main-control fetch labels

TeX82 §1030 gives `main_control` two places to fetch its next command, not
one, and which one is live is engine state that survives across Umber's
`step_once` boundary.

- `big_switch` calls `get_x_token`. This is where all but four cases of the
  big `case` statement resume, because each of them ends at
  `goto big_switch`.
- §1034's inner character loop (`main_loop`) instead resumes at §1038's
  `main_loop_lookahead`, which starts with a bare `get_next` -- "set only
  `cur_cmd` and `cur_chr`, for speed" -- and jumps straight back into the
  loop when that raw command is `letter`, `other_char`, or `char_given`.
  Only a raw command outside those three reaches `x_token`.

So a run of adjacent ordinary characters produces exactly one raw delivery
per character and no expanded delivery at all, while the first character of
a run -- fetched at `big_switch` -- produces both. A canonical engine that
fetches every command through `get_x_token` is not merely noisy in the trace:
it has collapsed two distinct labels into one, and the extra expanded
deliveries desynchronize the semantic stream from the first word of body text
onward.

`MainControl::main_loop_active` carries the live label. It is set
only by §1030's four `main_loop` entries -- `hmode+letter`,
`hmode+other_char`, `hmode+char_given`, `hmode+char_num` -- and only when
both of the tests those entries then pass hold:

- the mode the step left behind is horizontal or restricted horizontal
  (§1090's `vmode+letter` starts a paragraph first and arrives there;
  §1154's `mmode+letter` appends a math char and never enters the loop); and
- the current font contains the character. §1036's `main_loop_move+2`
  answers a missing character with `char_warning`, frees the would-be node,
  and jumps to `big_switch` rather than the lookahead. Under `\nullfont`
  -- §552 gives it `font_bc=1`, `font_ec=0`, so it contains nothing -- that
  is _every_ character, which is why a font-free fixture legitimately shows
  a raw and an expanded delivery for each of its letters.

Both of main control's fetch sites honor the label: ordinary `scan_step` and
the alignment cell's `get_x_alignment_delivery`. An alignment cell body is
ordinary `main_control` material, and neither of that path's recovery
predicates can fire for the three commands §1038 accepts raw.

The label is not parked across command steps while an alignment scanner is
active. TeX82 §1038's inner loop calls scalar `get_next`, whose §341 delivery
performs alignment interception and brace accounting. Umber's horizontal
macro-text acceleration can bypass that scalar boundary, so command-owned
alignment activity deoptimizes the parked fast path just as it deoptimizes a
macro literal span; the executor must not infer this state from its mode nest.

Executor-owned replay episodes (a discretionary part, an `\afterassignment`
token) clear the label on entry and exit. TeX reaches those
lists through `scan_left_brace`/`push_nest` and leaves them through
`handle_right_brace`, never mid-character-run, so an episode's own last
character must not park the enclosing context at the lookahead.

### 33.5 Host-applied step routing

Most `ColdOperation`s are applied by the cold module's `apply_cold_operation`,
which owns no `MainControl`. A few cannot be: TeX82 §1137's
`init_math`, §1193's `after_math`, §1190's `math_left_right`, §1116's
`append_discretionary`, the math-noad family, and the end of a replay episode
all need the mode nest, the save stack, and the command processor's
token-list scheduling at once, plus the ability to run nested episodes. Those
steps are applied by `MainControl` itself, and `apply_cold_operation`
carries an `unreachable!()` arm for each.

There are three step-delivery entry points -- unobserved, observed, and the
alignment cell's. The host-applied set is named exactly once, in
`MainControl::apply_host_owned_step`, and all three route through
it. It used to be an `if let` chain copied into each entry point, and the
observed copy omitted `ColdOperation::MathShift`: an observed `$` fell through
to `apply_cold_operation`'s `unreachable!()` and panicked while the identical
unobserved `$` executed correctly (`umber2-johp.118`). Add a host-applied
step in that one function, never at a call site.

The rule that failure exposed is the general one, and it is not specific to
math: **observation is an instrumentation boundary, not an alternate
execution mode.** Any behavior that differs between an observed and an
unobserved step is a defect on its own terms, before any trace is compared
against the oracle -- the traced run is then not the run being shipped. Two
independent copies of a routing decision are the mechanism that lets that
happen, so the fix is always to derive both paths from one shared commit
rather than to repair the copy that drifted.

A host-applied step is also where an operation stops being one command
processor episode. `init_math`, the math-noad family, and `append_discretionary`
each run **nested** command-processor episodes while they execute -- a math
field's scan (§1151 `scan_math`, reached from §1176's `sub_sup` for a
script; see §33.8 for why the field itself is never a replay level), a
`\mathchoice` branch, a discretionary part -- and each of those is a fresh
`CommandProcessor`. Which is the same duplication one level down:
every construction site got to decide for itself whether the operation's
observer was installed, and the three nested math constructions never did.
An observed `^{\the\footnotenum}` then consumed its entire braced field with
zero observations, and the backup level holding the backed-up `^` was never
seen retiring, while the unobserved run consumed it identically
(`umber2-johp.195`). That is the same defect as `umber2-johp.118` with the
opposite symptom: not a step that behaves differently when observed, but a
step whose observation silently stops partway through. Both make the traced
run a different artifact from the shipped one.

So the construction is named once too. `command_processor` is the only
`CommandProcessor::new` call in `tex-exec`, and it takes the operation's
commit slot as a parameter, so an episode cannot be constructed without
stating which buffer it publishes into. `MainControl` holds that slot
as engine state (`operation_observations`) rather than threading an observer
argument, precisely because a nested episode is several frames below the entry
point that knows whether the operation is observed. Deliberate silence -- the
startup terminal line's exhaustion retirement -- vacates the slot for that one
episode and says so; it is never expressed by omitting the observer at a
construction site. `crates/tex-exec/tests/it.rs` pins the single constructor
and the single `.with_observer` call.

### 33.6 The two math-shift probes are not one probe

A `$` delivered to main control is followed by a lookahead in three of the
six modes, and the lookahead is _not_ the same routine in all three. Keeping
one shared "is the next token another `$`" helper is what let the opener
inherit the closer's expansion policy (`umber2-johp.192`).

- §1138 `init_math` -- `hmode+math_shift`, for either sign of `hmode`. The
  probe is `get_token`, and tex.web says why on that line: "`get_x_token`
  would fail on `\ifmmode`". The probe runs before the math nest is pushed,
  so expanding a mode-sensitive conditional there answers for the mode the
  `$` is leaving. It pairs only under `(cur_cmd=math_shift)and(mode>0)`: in
  restricted horizontal mode `$$` is not a display opener, and the second
  `$` is backed up and reread as the end of an empty inline formula. Every
  other outcome runs §325 `back_input`, so exactly one raw delivery is ever
  consumed without a backup level.
- §1197's `@<Check that another \.\$ follows@>` -- reached from §1194
  `after_math` when a display closes (`m>=0` with `a=null`) or when a
  display's equation number closes (`mode=-m`). This one _is_
  `get_x_token`, so the peeked token is expanded and observed as an expanded
  delivery, and a non-shift reaches §327 `back_error` with the
  `Display math should end with $$` diagnostic.
- §1194's `m<0` closes inline math through `@<Finish math in text@>` and
  probes nothing at all.

The `mode>0` test belongs to §1138's probe alone, beside the decision to
consume or back up the second `$`. A second copy of it in the step's
application can disagree with the backup that already happened, and then the
consumed token is simply lost.

### 33.7 `goto reswitch` is not `back_input`

tex.web puts `reswitch:` _above_ `main_control`'s big case and below
`big_switch`'s `get_x_token` (§1030). A case that has already fetched its own
replacement command therefore dispatches that command in place: no input level
is pushed, no recovery record is written, and the command is delivered exactly
once. `back_input` is a different operation with a different observable
signature, and the two are not interchangeable.

`dispatch_main_control_command` owns that label, and every main-control step
runs through it whatever fetched the command: `scan_step`'s own `get_x_token`,
§1038's `main_loop_lookahead`, or an alignment cell's
`get_x_alignment_delivery`. It collects §1211's prefixes and loops; a case that
reswitches assigns the command it fetched and continues, re-entering prefix
collection exactly as tex.web's case re-enters through `prefix`.

That one owner is load-bearing, not tidiness. tex.web has no second dispatcher
for an alignment body -- §785's `align_peek` and §1130's
`vmode+endv,hmode+endv: do_endv` only bound a cell, and §1210 files
`any_mode(prefix)` under the modeless cases -- so a caller that reaches the big
case without passing through here is running a _narrowed_ main control that
silently drops whatever the label consumes. When the alignment paths did
exactly that, `\global` inside a cell reached the exhaustive fallback as an
unimplemented primitive and `\ignorespaces` reached an `unreachable!()`
(`umber2-johp.208`).

The canonical scan owns one such case: TeX82 §1045's
`any_mode(ignore_spaces)`, whose body is `<Get the next non-blank non-call
token>; goto reswitch`. §406 spells that helper as
`repeat get_x_token until cur_cmd<>spacer`, which is `next_non_space`, so
`\ignorespaces` consumes the spaces and nothing else.

Substituting `back_input` for the jump costs five semantic events per
`\ignorespaces` -- a backup push, a recovery record, a duplicate raw and
expanded delivery, and a backup retirement -- and re-derives the redelivered
command's provenance from the backup level rather than from its source
(`umber2-johp.196`, plain.tex's `\textindent` reaching `\footstrut`'s
`\vbox␣to\splittopskip{}`). §1045 is not the only `goto reswitch`: §1030's
`hmode+no_boundary` is a second one inside `main_control`, and §1151's
`scan_math` carries a `reswitch` label of its own for `char_num`. Neither may
grow a backup, for the same reason.

### 33.8 A math field is classified in place or is a live group; neither is an episode

TeX82 §1151's `scan_math` splits on what the next non-blank non-relax token
is, and neither outcome is an episode.

An unbraced field is resolved **in place**, by the procedure that already
fetched the command. Its six scalar cases -- `letter`, `other_char`,
`char_given`, `char_num`, `math_char_num`, `math_given`, `delim_num` -- each
end by assigning one math code `c`, and §1151 then stores
`math_type(p):=math_char; character(p):=qi(c mod 256)` with its own `fam`
rule. No input level is pushed, no token is backed up, and the command that
selected the case is never delivered a second time. The only `back_input` in
the whole procedure belongs to §1152's active-character restart and to
§1153's braced field.

Freezing that single spelling into a replay episode instead is a different
engine in three ways: the command is delivered twice, a level tex.web has no
§307 `token_type` for is pushed and retired around the second delivery, and
the field is reconstructed as a nested mlist -- which also loses `c`'s class
bits, because §1151 stores a math _character_ and drops the class a noad
would have carried, so `^\mathchar"3161` became a one-noad sublist instead
of a math char. gentle.tex reached this 147 times (`umber2-johp.265`).

§1151's `othercases` is the entire rest of the vocabulary, not just a left
brace: `\hbox` in a script field is `back_input; scan_left_brace` too, so
§403 reports `Missing { inserted`, backs the rejected command up, and
behaves as though a brace had been read. The `math_group` opens either way
and the rejected command becomes the first token of its body.

A braced field is §1153, and it is not command-owned material at all:

```
@<Scan a subformula...@>=
begin back_input; scan_left_brace;@/
saved(0):=p; incr(save_ptr); push_math(math_group); return;
end
```

`push_math` (§1136) is `push_nest; mode:=-mmode; incompleat_noad:=null;
new_save_level(c)`, so the pair of braces really does bracket a save-stack
level, and §1153 _returns_ -- the subformula's body is read by ordinary main
control, and §1186's `math_group` arm of `handle_right_brace` closes it on
the matching `}`. Nothing is absorbed and nothing is replayed.

Absorbing the body instead (`scan_left_brace`, then `back_input` and
`scan_toks`, then replay the frozen list on a stored level) is observably a
different engine, in four ways at once: the opening brace is delivered a
second time behind a second backup level, an absorbing scanner-status
transition is announced, an extra input level is pushed and retired around
the body, and the closing brace is consumed by the scanner instead of being
delivered as the command §1186 dispatches. gentle.tex's footnote marker --
plain.tex's `\footnote` reaching `$^{\the\footnotenum}$` -- showed all four
(`umber2-johp.199`).

`MathFieldBody::OpenGroup` is therefore the braced outcome: `tex-command`
consumes only the mandatory brace, and `MainControl::accept_math_field` opens
`push_math`'s save and mode levels, retains §1153's typed parent-field
destination, and returns to the ordinary production loop. Each body command
is then its own normal main-control operation. In particular, a file enquiry
inside a superscript uses the same generation-owned typed resource
suspension as one at top level; the opener and already committed body
commands are neither copied nor replayed. The delivered `}` is
`EndMathGroup`, which performs §1186's `unsave`/`fin_mlist`, pops that exact
destination, and fills the reserved nucleus or script field.

This destination stack is executor structural state, not a command scanner
or a caller-order mailbox. The command attempt and any suspended scanner
frames remain wholly owned by the existing command-generation lifecycle;
the math destination records only where §1186 writes after the ordinary
resource continuation resumes.

`\mathchoice` (§1172) is the same mechanism, not an exception to it.
`append_choices` is `tail_append(new_choice); ... push_math(math_choice_group);
scan_left_brace`, and §1174's `build_choices` -- the `math_choice_group` arm
of `handle_right_brace` -- `unsave`s, stores the finished mlist in the choice
node's display/text/script/scriptscript field, and repeats
`push_math(math_choice_group); scan_left_brace` for the next branch. So all
four branches are live `push_math` groups read by ordinary main control, one
at a time, exactly like §1153's; the only difference is which group code is
pushed and where the finished mlist is stored.

Absorbing the four branches instead added a fifth observable difference on top
of the four above: the absorbed branches are replayed from stored levels in
_reverse_ order, so the oracle's `\displaystyle` branch arrived as
`\scriptscriptstyle` (`umber2-johp.220`). `execute_live_math_choice_group`
therefore retains the same live input order for `math_choice_group`; ordinary
`math_group` fields close through `EndMathGroup` in the production loop as
described above.

### 33.9 `\aftergroup` is one backup per token, not one replay level

`\aftergroup` never builds a token list. §280's `save_for_after` pushes an
`insert_token` entry per token onto the save stack, and §282's
`@<Clear off top level from save_stack@>` -- `unsave`'s body -- reaches
`@<Insert token p into TeX's input@>` (§326) for each one:

```
begin t:=cur_tok; cur_tok:=p; back_input; cur_tok:=t;
end
```

So every saved token gets its own complete §325 `back_input`: its own
stack-conservation loop, its own one-token `backed_up` level, and its own
recovery record. Packing the payload into a single stored replay level is a
different object and is observed differently -- a macro body that ended with
the group's closing brace stays on the stack instead of retiring first, and
none of the backup pushes, recovery records, or backup retirements happen at
all (`umber2-johp.198`; gentle.tex's `\f@@t`, whose
`{\bgroup\aftergroup\@foot\let\next}` makes `\@foot` exactly such a token).

§282 clears the level from the top down while `\aftergroup` saved from the
bottom up, so the last-saved token is backed up first and ends up deepest.
Rereading therefore restores save order, and the layer that hands the payload
over in save order must back it up in reverse.

Two `back_input` entry points exist because §325 says `cur_tok` is the token,
not the delivery. `back_input_saved` is for a caller that still holds the
`CurrentCommand`, because §342's alignment interception records transitions
that set `align_state` outright rather than stepping it, and those must be
reversed as recorded. `back_input_token` is for every caller whose token is
not the live delivery -- §282's saved tokens, and §372's `\csname`, whose
`cur_tok:=cur_cs+cs_token_flag` was never delivered at all -- and it derives
the `align_state` change from the token's own category, exactly as §325 does.

### 33.10 Runtime command-entry audit

The 2026-08-04 cutover audit found one production command transition machine.
`MainControl` explicitly owns `CommandState`, the shared command-fuel ledger,
and `CommandHostCapabilities`. Its private `command_processor` helper is the only
`CommandProcessor::new` site in `tex-exec`; each bounded episode borrows those
roots and creates a borrow-scoped `CommandHostContext`.

Every supported runtime reaches that boundary without a command-path selector:

- direct and retained native jobs use `EngineSession`;
- the CLI composes `VirtualCompileSession`, while `expand-dump` uses the same
  retained session and `lex-dump` uses `CommandState` tokenization directly;
- virtual compilation retains a `tex-incr::RevisionCandidate` and canonical
  resource host across suspension;
- fresh INITEX construction and format-loaded execution create
  `EngineSession` with the selected immutable `CommandProfile`;
- editor and fixed-point sessions compose `VirtualCompileSession` and
  `tex-incr` candidates; and
- the WebAssembly `CompilerSession` is a representation adapter over
  `VirtualCompileSession`.

The `umber2-swm9` cutover deleted the retired crates and uncompiled legacy
tex-exec source. Those files are not a runtime fallback and must not be used as
an oracle or reintroduced as a selectable path.

## 34. End-state invariants

The replacement is complete only when all of these hold:

1. `tex-command` is the sole owner of source tokenization, input levels,
   semantic raw delivery, expansion, macro calls, conditions, and scanners.
2. Production has exactly one `get_next` semantic path.
3. Production has exactly one ordinary `get_x_token` loop.
4. Scanner status is explicit and owns outer-validity recovery.
5. Condition frames are not input frames.
6. The command core owns the only `align_state`.
7. Literal braces, not brace aliases, update `align_state`.
8. Backup uses exact delivery identity and corrects alignment once.
9. `\noexpand` suppression lasts exactly one backed-up delivery.
10. `\unexpanded` and token-list `\the` splice only in expanded token-list
    collection.
11. Macro arguments are stored once and replayed by range.
12. The canonical scalar macro matcher remains the semantic baseline.
13. TeX82, e-TeX, and pdfTeX extensions share the same base command machine.
14. Exact compatibility profiles are byte-oriented.
15. Unicode mode has a distinct engine and format identity.
16. Host capabilities do not enter command snapshots.
17. Resource suspension rolls back the aggregate executor step.
18. Provenance and caches cannot affect semantic behavior.
19. Incremental execution equals cold execution, which equals the canonical
    reference in the shared domain.
20. Reference fixtures come from pinned canonical engines, never the retired
    implementation.
21. Optimizations are parity-locked and measurement-promoted.
22. `tex-lex`, `tex-expand`, compatibility adapters, duplicate delivery loops,
    and behavioral replay kinds are removed.

## 35. Rejected alternatives

### 35.1 Preserve `tex-lex -> tex-expand`

Rejected because `get_next` crosses tokenization, replay, meaning resolution,
scanner status, parameter substitution, and alignment delivery. A public crate
boundary through that operation creates duplicate drivers and semantic
metadata leakage.

### 35.2 Make `InputStack` the whole command engine

Rejected because condition state, meaning caches, paragraph transitions,
profiling, and command dispatch are not input. Snapshot coupling is not
ownership.

### 35.3 Keep condition frames interleaved with input

Rejected because TeX's condition stack is independent, condition identity must
survive recursive operand expansion, and input retirement must not inspect
conditional metadata.

### 35.4 Use runtime primitive traits

Rejected because supported dialects are closed, extension seams are known,
dynamic dispatch would enter the hot path, and a generic registry obscures
canonical command families.

### 35.5 Compile macros before parity

Rejected because a compiled matcher can change raw token consumption,
delimiter-prefix recovery, paragraph exceptions, and error state. Compilation
is an optional post-parity optimization, not the semantic foundation.

### 35.6 Use exact Pascal call traces as the oracle

Rejected because Rust may express an equivalent state transition without the
same helper nesting. Oracle events describe semantic deliveries and committed
state transitions.

### 35.7 Use the retired Umber engine as an oracle

Rejected because it would preserve accidental behavior and the architecture
being replaced. It may remain buildable during migration but supplies no
expected result.

### 35.8 Make scanners resumable

Rejected because executor-step rollback already gives deterministic resource
retry, arbitrary continuations greatly enlarge state, and named incremental
boundaries are intentionally quiescent.

### 35.9 Make provenance part of token semantics

Rejected because provenance is diagnostic metadata, would poison equality and
caches, and would make editing or rollback change TeX behavior.

### 35.10 Claim Unicode behavior is pdfTeX behavior

Rejected because pdfTeX is an 8-bit engine. Unicode support is valuable but
must remain an explicitly identified Umber extension.

## List commit invariant

Horizontal construction may retain a pending shaped character run, but a mode
level may never be popped while it does. `tex-exec` routes list closure through
`assignments::commit_current_list`, which materializes that run before the
level can be packaged, frozen, or supplied to an output path. `ModeNest::pop`
rejects an uncommitted run as a backstop, so a new list-finalization path cannot
silently recreate an empty box or lose geometry by omitting a caller-side
flush.

## Direct retained-session host boundary

Direct fresh and format-loaded jobs acquire their root through the active
`World`, transfer that exact `SourceRegistration` into
`EngineSession`, and then drive bounded `MainControl`
operations. Resource policy sees only typed input, font, and image needs plus a
borrow-scoped `World` capability. It returns immutable matching
registrations; aggregate rollback owns retry. The host never receives command
state, a raw input cursor, or an expanded-token delivery API.

Completion publishes one authoritative `RunResult` from committed receipts:
effects from the session's effect cursor, artifact hashes and bytes from its
artifact cursor, and prepared DVI pages aligned one-for-one with those
artifacts. It also retains the initial mode and every distinct mode reached
after a committed main-control step, plus TeX82 §93's exact fatal terminal
state when §81 ends the job through `jump_out`. That fatal state is successful
semantic completion, not a runner error. After all §1335 and pdfTeX
close-files diagnostics, the result derives `TexRunStatus` from §76's final
history: spotless and warning histories are successful, a recovered error is
completed-with-errors, and fatal history is fatal. TeX82 §1335's effective INITEX
`\dump` is also a committed main-control receipt. The host may serialize a
format only when that receipt is present; it must not infer dump intent by
examining source bytes or retired executor statistics.

Native virtual compilation and editor/fixed-point checkpoint persistence use
the same command-owned contract. Their `EngineCheckpoint` values contain a
required `CommandSummary`; they have no compatibility adapter or alternate
lexer/expander restart representation.
