# TeX Command Core

Status: authoritative target architecture for Beads epic `umber2-johp`.

## 1. Purpose

Umber will replace the current `tex-lex` and `tex-expand` pipeline with one
command-processing subsystem whose semantic structure follows TeX82, e-TeX
2.6, and pdfTeX 1.40.27 directly.

The central architectural fact is that TeX does not have a clean semantic
boundary between lexical input and expansion. TeX's `get_next` reads physical
characters or stored token lists, substitutes macro parameters, resolves
control-sequence meanings, enforces scanner status, updates `align_state`, and
intercepts alignment delimiters. Its real downstream boundary is the delivery
of an unexpandable current command to main control.

`tex-exec::CanonicalMainControl` is the production main-control seam for that boundary:
it accepts no `InputStack`, obtains each `CurrentCommand` and every assignment
operand through `CommandProcessor`, then applies only the completed typed
structural mutation after the processor borrow ends. Macro calls and
registered `\\input` nesting therefore remain command-core operations; the
executor never rereads their source text.

`umber::EngineSession` constructs that command machine at startup and registers
its retained root plus already-acquired nested `World` resources through typed
capabilities. A session with a registered root executes its bounded operations
through `CanonicalMainControl`; resource acquisition, transaction rollback,
and final effect/artifact commit remain host-owned. The compatibility adapter
is retained only for callers not yet able to provide immutable root bytes.

When main control starts a paragraph from vertical mode, replay makes the
mode decision at the executor seam but asks the still-live command processor
to perform TeX82's `back_input` on the triggering ordinary character. The
typed paragraph-start result then changes the executor mode after that borrow
ends; the first character is reconsidered only through the command core's
backed-up input level.

Canonical `\font` definitions scan their target, optional equals, expanded
filename, and `at`/`scaled` clause into an immutable `FontLoadRequest`. After
that processor borrow ends, replay resolves the request through its transient
registered-font capability and installs the loaded meaning atomically. An
absent capability is a typed font suspension, so the enclosing aggregate rolls
back before a fresh processor episode retries; a completed unavailable lookup
instead recovers the target to `nullfont`. Font capabilities and loaded
resources never enter command snapshots or durable summaries.

The replay seam also retains the executor-side mode projection and obtains
observable general-text effects (currently `\\message`) through the typed
structured scanner. Alignment lifecycle state crosses it only as
`AlignmentRequest`; active-cell delivery uses
`CommandProcessor::get_x_alignment_delivery`, and an intercepted delimiter is
returned to that same processor episode for v-template installation. Thus
replay does not turn alignment, mode changes, or effects into a second source
consumer.

Text `\\accent` and `\\discretionary` use the same completed-scanner boundary:
the command processor owns the accent number, expanded base-character lookup,
and non-character replay, and freezes each discretionary group as traced,
immutable material. `CanonicalMainControl` replays each frozen part as its own
stored command level inside a `disc_group` restricted-horizontal episode,
flushes and freezes the completed node list, then applies the typed `Disc`
node. Group-local definitions and recovery remain live command/Universe state;
the group's `\aftergroup` payload is returned to a command-owned replay level
before the next part. This aggregate operation remains under one rollback
snapshot: it must not recreate an `InputStack` or expose raw group delimiters
to the executor.

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

## 2. Canonical authorities

Compatibility behavior is derived in this order:

1. Knuth's TeX82 `tex.web`;
2. e-TeX 2.6 and its canonical change files;
3. pdfTeX 1.40.27 and its canonical change files;
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

For TeX82 §§1064, 1066, and 1131 `off_save`, the executor chooses only the
typed structural closer. `tex-command` reports `off_save_replay` before it
backs up the offending command and inserts that closer, so raw delivery cannot
overtake the diagnostic; at bottom level it reports the drop before the backup
retires. This applies equally to ordinary `\endgroup` recovery and alignment
`endv` replay.

For TeX82 §1095's `hmode+stop` `head_for_vmode` branch, `tex-command` likewise
owns the two exact backups: it first backs up `\end` (or `\dump`), then backs
up the synthesized primitive `\par` with inserted input ownership. The
executor applies only the typed paragraph transition, after which command
processing retires that inserted recovery level and redelivers the stop in
vertical mode. This precedes §46's `its_all_over` end-game decision; no
executor source-read path may manufacture either token.

At TeX82 §46's `its_all_over` output hand-off, that redelivered vertical stop
first retires its exhausted backup, then receives a fresh command-owned backup
before the `\output` token list is pushed. This preserves the final-stop retry
below the output routine and fixes the canonical input order independently of
the executor's typed page/output effects.

When TeX82 §46 reaches the `max_dead_cycles` escape after completed output
routines, replay preserves that same final-stop backup and then publishes the
single detached forced-shipout effect through the ordinary artifact commit
boundary. The command seam does not reconstruct the repeated dead-cycle page
lists; their only canonical trace consequence is that ordered DVI effect.

Canonical `\shipout` replay likewise crosses the executor only as a completed
box. The artifact kernel receives an already-published detached input summary;
it must not construct a `tex_lex::InputStack` as a publication fallback.
Explicit `\begingroup` is a typed `SemiSimple` entry and `\endgroup` its
matching typed exit. TeX82 §1064 recovery remains command-owned: malformed
macro targets and parameter markers preserve/replay their source spellings in
the command machine, while main control applies only the resulting diagnostic
effect and recovered definition.

Replay models that routine as an explicit output group and internal-vertical
mode. Its required opening brace is consumed by `scan_left_brace`, not as a
nested ordinary group; the matching close ends its paragraph, leaves the
output group, and restores outer vertical mode before output-list retirement.
The final-stop backup below the output list can therefore retire before a later
`its_all_over` retry, instead of spuriously taking `head_for_vmode`'s
horizontal recovery path (TeX82 §§46, 1095, 1131).

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
`\jobname` use selectors 0 through 5. TeX82 §27's `conv_toks` continues to
own their respective operand scans and inserted-token lifecycle; the command
identity is selected at raw delivery, rather than projected later from a
generic expandable primitive. Startup filename scanning installs the selected
area-free, extension-free job name through `CommandHostCapabilities`; it is a
borrow-scoped environment fact, so snapshots retain neither a host path nor a
fixture-derived conversion value.

For TeX82 §§1071 and 1076, `\shipout` begins a typed box-completion episode:
command control delivers the next `make_box` command and owns every scalar
box-register scan, including its one-token terminator backup. On the closing
brace, replay performs §1071's `box_end`/`ship_out(cur_box)` synchronously;
the DVI-page effect is consequently published before that terminator backup
retires on the next raw fetch. The executor receives the completed box node,
not an input capability or a token to reread.

Likewise, `\box` is observed as `make_box` with `cur_chr` 0 before command
control invokes TeX82 §1071's `scan_int` for its box-register operand. That
command-owned scan preserves raw digit delivery and any terminator backup;
TeX82 §15 assigns the shared `make_box` command code, and §35 initializes
`\box` with `box_code` (`0`). The executor consumes only the resulting typed
box semantics.

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
`scan_keyword`, `scan_optional_sign`, `scan_optional_equals`, and
`scan_internal_value` operations consume only the command-owned expanded input
stream. Each returns a typed `ScannedScalar` value carrying the first-token
provenance and any canonical recovery; callers never receive an input cursor,
token frame, or raw-delivery capability. Failed optional keyword scans replay
through the same `get_next` path, so executor code cannot create a second
lexer, expansion loop, or backup mechanism. `CommandState::snapshot` remains
the transaction boundary for the resulting future input state.

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

`scan_file_name` returns a typed filename and its canonical `Group`, `Space`,
`NonCharacter`, or `EndOfInput` termination. `open_registered_input` composes
that scan with the borrow-scoped registered-input capability, then registers
and opens the immutable source through `CommandState`; unavailable backing is
the typed `CommandError::MissingInput` recovery. Neither API exposes a source
cursor, input level, raw token, or host filesystem operation. Snapshot rollback
therefore restores the complete future input state after every structured scan.

### 5.3.1 Canonical main-control ownership gate

`tex-exec::CanonicalMainControl` is the only production executor-facing command driver.
It may classify `CurrentCommand::meaning()` and apply completed typed values,
but it must not accept or construct `tex_lex::InputStack`, call raw-token
delivery, or inspect a raw token carried by a delivered command. The legacy
executor remains an independent migration surface and may still depend on
`tex-lex`/`tex-expand`; that temporary dependency does not grant the replay
adapter a second input path. The `tex-exec` architecture test enforces this
source-level boundary, while replay tests exercise typed scanner rollback and
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
the canonical driver; `CanonicalMainControl` applies the captured meaning only after that
processor borrow ends and then publishes the committed meaning mutation.

TeX82 `\hrule` and `\vrule` cross the same gate as completed
`ScannedRuleSpec` values. `CommandProcessor::scan_rule_spec` owns every
expanded `width`, `height`, and `depth` keyword and dimension scan, including
the failed-keyword backup that begins subsequent main control. Replay only
appends the resulting rule node; it neither reads a source token nor rebuilds
rule provenance. This remains true inside alignment cells, where template
delivery and rule scanning share the one command-owned input stream.

TeX82 `\setbox` follows the same split in two phases. `CommandProcessor`
scans the register integer and optional equals sign as a typed
`ScannedSetBoxAssignment`, including the canonical backup of the equals
delivery. The following `\vbox` remains an ordinary command delivery; replay
validates and backs up its required opening brace through the processor, then
opens, packages, and assigns the executor-owned box group.
This keeps box construction from acquiring a raw-input API while retaining the
observable scanner and backup ordering.

For `\hbox`, `\vbox`, and `\vtop`, command processing also owns the optional
`to`/`spread` packing clause and dimension before validating and backing up the
opening brace. Replay enters the typed group and mode, schedules the matching
immutable `\everyhbox`/`\everyvbox` command episode, and applies pure packing
only after the body closes; scoped `\setbox` assignment then occurs at the
same aggregate boundary.

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
        backup.rs
        summary.rs

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
        helpers.rs
        integer.rs
        dimension.rs
        glue.rs
        token_list.rs
        font.rs

    primitives/
        mod.rs
        tex.rs
        etex.rs
        pdftex.rs
        pdf_strings.rs
        pdf_files.rs
        pdf_regex.rs
        pdf_random.rs

    provenance.rs
    observation.rs
    snapshot.rs

    tests/
```

Files should remain organized around canonical state machines. Mechanical
splitting is not a substitute for ownership separation.

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
pdfTeX 1.40.27 + EightBitExact
```

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
position while retaining a zero-width physical byte range. The sole
source-token exception is the `\par` control sequence generated by an
endline in `new_line` state: it retains the triggering physical terminator
range (when present), so its control-sequence source position stays on the
blank line rather than falling back to the preceding line's anchor. An
unterminated final line remains zero-width.

Each direct source delivery also carries a typed canonical location distinct
from its raw span. It is TeX82's post-delivery `loc - start - 1`: ordinary
one-byte spellings therefore locate at their span start, while a decoded
`^^41` retains its full four-byte raw span but locates at the final `1`.
Zero-width synthetic spellings retain their physical anchor. This provenance
pair is ordinary snapshot-owned input state, including backup replay; it is
not reconstructed by fixture observation.

Control-sequence names are semantically sequences of `CharacterCode`. Their
storage may use a compact UTF-8 representation when lossless for the active
profile, but string encoding is an implementation detail. Name identity,
`\string`, `\meaning`, format identity, and diagnostics must operate on
canonical character codes.

Code-table queries use `CharacterCode` and the active profile. A profile
conversion cannot occur during a job.

The implemented immutable `CommandProfile` has exact TeX82, e-TeX 2.6, and
pdfTeX 1.40.27 eight-bit constants plus explicit Unicode-extension
construction. Dialect facilities (e-TeX and pdfTeX families) and Unicode
semantics are derived capabilities, not host capabilities. Its versioned
stable bytes feed a fixed domain-separated FNV-1a-64 profile fingerprint;
format and checkpoint identities compose that fingerprint.

## 7. State taxonomy

The design distinguishes five state classes:

| Class                       | Examples                                                              | Snapshot rule                                                  |
| --------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------- |
| Semantic command state      | input cursors, macro activations, conditions, `align_state`           | captured and restored                                          |
| Aggregate engine state      | meanings, registers, code tables, fonts, World stream state           | owned and snapshotted by `Universe`                            |
| Call-local semantic state   | `CurrentCommand`, local scanner accumulator, expansion budget scope   | absent at durable boundaries; replayed from the enclosing step |
| Diagnostic/provenance state | source map, origins, macro invocation DAG                             | rollback-coupled but excluded from semantic equality           |
| Discardable acceleration    | meaning cache, normalized-line cache, buffer pool, profiling counters | may be dropped without changing behavior                       |

No type may mix fields from these classes merely because they are used by the
same procedure.

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

This is the command half of an executor savepoint. It is never independently
committed from the paired `Universe`, mode nest, execution state, effects,
generated writes, and output state.

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

`TransientState` owns pooled token buffers, builders referenced by live scanner
status, and rollback roots for temporary command data. Pools and spare
capacity are discardable; the contents of a live builder are semantic until
its enclosing call completes or rolls back.

## 9. Discardable runtime state

Acceleration does not live in `CommandState`:

```rust
pub struct CommandRuntime {
    meaning_cache: MeaningCache,
    normalized_lines: LineNormalizationCache,
    transient_pool: TokenBufferPool,
    profiling: CommandProfiling,
}
```

Every cache entry is guarded by canonical identity and exact generation or
content stamps. Dropping `CommandRuntime` and continuing with a fresh default
must produce identical semantic events, diagnostics, effects, and output.

The input stack does not carry meaning-cache state or expansion-policy bits.

## 10. Command processor

`CommandProcessor` is an ephemeral capability facade:

```rust
pub struct CommandProcessor<'a> {
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    state: tex_state::CommandContext<'a>,
    host: CommandHostContext<'a>,
    observer: Option<&'a mut dyn CommandObserver>,
}
```

It does not own state and cannot outlive one bounded executor operation.
`CommandHostContext` contains only the capabilities installed for that
operation, such as input resolution and optional read recording. Host
capabilities never enter snapshots or formats.

Production methods include:

```rust
impl CommandProcessor<'_> {
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
    Source(SourceCursor),
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
decodes or rewrites bytes. Opening a registered source only clones retained
backing and therefore cannot invoke host policy.

Physical refill distinguishes LF, CR, CRLF, and a missing final terminator.
A final terminator does not manufacture another empty line. Normalization
removes trailing byte `0x20` values, captures the current profile-valid
`endlinechar`, and delivers that synthetic character at the zero-width anchor
after the retained prefix. Unicode scalar delivery advances the same canonical
byte cursor by the UTF-8 width and retains a scalar delivery offset; exact
eight-bit delivery advances it by one. In both modes ordinary character ranges
address only immutable physical bytes, and terminator and stripped-space ranges
remain available as physical metadata without being claimed by tokens.

### 12.2 Token cursor

Payload, semantic delivery behavior, retirement, and trace explanation are
orthogonal:

```rust
struct TokenCursor {
    payload: TokenPayload,
    behavior: TokenBehavior,
    retirement: RetirementBehavior,
    trace: ReplayTrace,
    index: usize,
    identity: InputLevelId,
}

enum TokenPayload {
    Stored {
        tokens: TokenListId,
        origins: OriginListId,
    },
    Transient(SharedTokenBuffer),
    ArgumentRange {
        buffer: SharedTokenBuffer,
        range: MacroArgumentRange,
    },
}

enum TokenBehavior {
    Ordinary,
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

The implemented ownership model keeps `MacroActivation` values in the
`ParameterState` activation chain and stores a typed activation identity in
`TokenBehavior::MacroBody`. This preserves one owner for each activation while
letting its `MacroArguments` and any live `ArgumentRange` payloads retain the
same reference-counted contiguous traced-token allocation. `InputLevelId` is
typed separately from source identity and is present on both source and token
levels. Exact-byte and Unicode source cursors use this identical enum.

`EveryPar`, `EveryHBox`, `EveryVBox`, `EveryJob`, `EveryCr`, `Mark`,
`OutputRoutine`, and similar explanations belong in `ReplayTrace` unless they
demonstrably change retirement behavior. Trace reasons never select expansion
semantics.

Exhaustion commits against the exact `InputLevelId`. Ordinary, terminal-stop,
and `\scantokens` levels pop once; popping releases only the cursor's ownership
of transient or stored backing. An exhausted v-template instead transitions
once to `AwaitingVTemplateRetirement`, remains the exact top level through
end-template delivery, and is popped only after successful `do_endv`.
Macro-body retirement atomically removes the activation matching that level's
typed `param_start`; a mismatched activation chain is rejected before either
owner is mutated. The committed lifecycle record may copy `ReplayTrace` for
observation, but neither its action nor activation cleanup consults that trace.
Its detached observation also retains the exhausted level's immutable class
(source, backup, macro, parameter, alignment template, recovery, or token
list), so host-side canonical translation preserves lifecycle ordering without
letting diagnostic explanation select retirement behavior.

### 12.3 Macro parameters

A macro activation owns one shared argument buffer and at most nine ranges:

```rust
struct MacroActivation {
    definition: MacroDefinitionId,
    arguments: MacroArguments,
    invocation: InvocationOriginId,
}

struct MacroArguments {
    buffer: SharedTokenBuffer,
    ranges: [Option<MacroArgumentRange>; 9],
}
```

The scalar matcher accumulates completed arguments in definition order through
one `MacroArgumentBuilder`, then freezes that builder into the activation's
single shared buffer. Empty arguments retain empty half-open ranges. A compact
`OutParameter(u8)` remains distinct from a literal parameter character emitted
by the canonical `##` escape, so replay can substitute only the former without
rewriting immutable macro definition token lists. The processor allocates the
invocation provenance node using the active activation's invocation as parent,
then installs the activation owner before exposing its immutable replacement
body level.

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

`get_token` temporarily enables the canonical control-sequence creation policy.
Ordinary `get_next` preserves the reference engine's
`no_new_control_sequence` behavior. The modern interner may allocate a spelling
for diagnostics, but doing so cannot define a meaning or change future
semantic lookup.

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
8. updates `align_state` for literal character braces;
9. detects an alignment delimiter at the current base depth;
10. pushes or retires the canonical template input required by that delimiter;
11. records the semantic read and diagnostic provenance; and
12. returns `CurrentCommand`.

Steps may restart without returning a command, exactly as TeX restarts after
ignored characters, exhausted input, parameter insertion, and template
insertion.

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

`get_token` invokes `get_next` under canonical control-sequence creation policy
and returns the same `CurrentCommand` with its packed token spelling.

`back_input`:

1. validates that the nonce-bearing delivery stamp still identifies the most
   recent raw transition in the live processor episode (not merely an equal
   token at an equal cursor position);
2. undoes exactly one literal-brace alignment adjustment made by that
   delivery;
3. rewinds the current level without allocation when the exact level and
   cursor remain current and the backup treatment is ordinary; otherwise
4. pushes a backed-up token level carrying the exact spelling, origin, raw
   source span, and typed canonical location.

One-delivery treatments such as `\\noexpand` always use the backed-up level,
including when the original token-list cursor remains rewindable.

Semantic equality to a previously delivered token is not proof that the token
can be rewound.

`back_error` performs the same backup and then queues the canonical recoverable
diagnostic. Inserted recovery tokens acquire explicit inserted origins and
ordinary future delivery semantics.

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
the inserted token identity. Thus frozen-`\cr` recovery preserves the
inaccessible control-sequence spelling while classifying the TeX82 recovery
operation as `InsertedToken`; consumers must not infer the operation kind from
the token's spelling.

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
10. stores every completed argument once in one shared packed buffer;
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
argument ranges while allocating exactly one new invocation origin per replay;
an architecture gate preserves one raw scalar fallback and rejects alternate
matcher types.

## 20. Canonical `scan_toks`

`scan_toks` is not implemented by blindly calling generic `get_x_token`.

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
   the shared mechanism, before parent input resumes; and
7. observes the live `skipping` to prior-status restoration, then restores
   the prior scanner status.

The TeX82 predicate dispatcher selects `get_x_token` for character/category
tests and `get_token` specifically for `\ifx`; the latter preserves raw
meanings and must not expand either operand. `\ifx` compares non-macro
meanings directly, while macros compare their flags plus raw parameter and
replacement token sequences rather than their immutable-store allocation
identities. Character/category tests normalize non-character operands to
TeX's common relax sentinel before comparing them.
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
delivers the skipped raw tokens. Once the delimiter has been delivered and the
scanner status restored, the command core records that delimiter as the branch
under the pre-change limit, then changes the frame to `fi` for `\else` (or
records and pops it for `\fi`). This keeps the observable TeX82 transition
order separate from the stack's typed state transitions.

An `\ifcase` frame likewise remains `evaluating` while `pass_text` skips each
non-selected limb. Each traversed `\or` is recorded under that pre-change
limit; only after the selected delimiter restores normal scanner status does
the command core change the frame to `or` and record the selected `case`
branch. This projection ordering does not alter condition-stack evaluation.

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
delimiter and inserting the required group closer.

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

`scan_something_internal` applies TeX82's `scan_eight_bit_int` bound to every
primitive register family (`\count`, `\dimen`, `\skip`, `\muskip`, and
`\toks`): the
index is scanned through ordinary expanded command delivery, and a negative
or greater-than-255 index recovers as register zero. A dimension-valued
register bypasses the numeric dimension backup/replay path and reaches the
internal result directly; a complete glue-valued register likewise bypasses
the trailing `plus`/`minus` scan. The observer records the resulting typed
internal integer, scaled, or glue value without leaking register storage.

`scan_dimen` delegates its integral prefix to that same integer scanner. Its
backed-up decimal point is then consumed raw, while a backed-up unit remains
available to the canonical keyword retry sequence. `scan_glue` first probes a
complete internal glue value; an ordinary numeric width is backed up and only
then passed to `scan_dimen`. These retry frames retire before their replacement
backup, preserving the TeX82 input lifecycle for whole-number dimensions,
physical units, and `fil` orders. In particular, after a fractional value,
TeX82 §455's internal-unit, `em`, `ex`, `true`, and `pt` probes each own their
failed replay before `scan_keyword("in")` consumes `i` and `n` directly. The
replay adapter receives only the final `Scaled` or `GlueSpec` and never
reconstructs these transitions.

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
- fixed job clock;
- deterministic random values;
- PDF and font enquiries owned by aggregate state;
- staged terminal/log diagnostics; and
- optional dependency observation.

A missing resource returns a typed `NeedResource`. No async future, callback,
or scanner continuation is stored in `CommandState`. The enclosing executor
restores its complete pre-step savepoint and retries after the host binds a
stable positive or negative response.

The promoted scalar input/state slice keeps the same split concretely:
`CommandHostCapabilities` maps a logical `\input` name to already-acquired
immutable backing and supplies the immutable job name for the bounded
processor call. `\input` registers that backing before opening a source level;
`\endinput` marks only the active source to retire after its current physical
line. TeX82 `end_file_reading` (§338) observes only that retirement; the
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
an input suspension, so the whole command aggregate rolls back before a fresh
processor episode retries.

At ordinary EOF, TeX82 §343 calls `end_file_reading`, checks outer validity,
and restarts raw delivery at the caller. Thus a nested `\input` retirement and
the parent source's normalized line-ending space can each occupy later
main-control operations before the parent delivers `\end`. The composed
canonical bridge must delegate that sequence to `CanonicalMainControl`'s
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

## 28. Snapshots and durable summaries

There are two snapshot forms.

### 28.1 Executor-step snapshot

`CommandStateSnapshot` owns the exact live command state over already retained
source and token backing. Capture is bounded by live command-state structure,
not total input bytes. Rollback does not reopen sources or consult host policy.
The snapshot is an owned clone of `CommandState`; its trait bounds and private
fields admit no `CommandRuntime`, `CommandProcessor` borrow,
`CommandHostContext`, or `CurrentCommand`.

Rollback first compares the captured profile fingerprint with the fixed job
profile. A mismatch rejects the snapshot without mutating live state.

It is paired atomically with:

- `Universe` snapshot;
- mode and execution roots;
- page/output state;
- staged effects and artifacts;
- generated-file stage;
- statistics; and
- checkpoint publication state.

### 28.2 Durable summary

`CommandSummary` contains only validated restartable data:

- source cursors and stable backing identities;
- token cursors and immutable payload identities;
- macro activations and live argument ranges;
- condition frames;
- quiescent alignment brace state, without active or suspended templates;
- profile and source-id high-water marks;
- persistent expansion counters; and
- required provenance/source roots.

Host capabilities, `CurrentCommand`, delivery stamps, cache contents, spare
buffer capacity, timers, and profiling counters are absent.

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

| Invariant                                                                                   | Executable boundary                                                                 |
| ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `tex-state <- tex-command`, with no dependency on the retired command crates or `tex-exec`  | `crates/tex-command/tests/it/boundaries.rs` manifest-direction test                 |
| crate-private state machines and opaque ownership fields                                    | compile-fail fixtures under `crates/tex-command/tests/ui/`                          |
| one explicitly classified field for each semantic ownership domain                          | exhaustive destructuring in `crates/tex-command/src/state/tests.rs`                 |
| runtime caches remain discardable and outside semantic equality, hashing, and serialization | runtime replacement test plus the runtime-trait compile-fail fixture                |
| host capabilities and call-local command values cannot enter owned serialized boundaries    | host and ephemeral compile-fail fixtures                                            |
| snapshots preserve all live semantic fields without runtime or host access                  | nonquiescent snapshot roundtrip in `crates/tex-command/src/snapshot/tests.rs`       |
| durable summaries reconstruct exact quiescent state and reject every nonquiescent class     | summary roundtrip and rejection tests in `crates/tex-command/src/snapshot/tests.rs` |

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

- the command core chooses canonical recovery tokens and state transitions;
- typed diagnostics capture primary origin, related origins, macro invocation
  head, scanner status, command identity, and canonical diagnostic kind;
- the host formatter resolves paths, lines, columns, excerpts, and macro
  traces lazily.

Batch/nonstop transcript fixtures compare canonical wording and order where
the project claims transcript parity. Host-specific paths, banners, terminal
interaction, and display widths are normalized only through explicit fixture
rules.

The canonical hundred-error termination and explicit expansion/resource
budgets prevent unbounded recovery. Limits are versioned engine policy and
must not create an alternative successful result inside the reference domain.

## 31. Reference oracle and conformance

Correctness uses pinned TeX82, e-TeX, and pdfTeX engines, never the retired
implementation.

The reference transport writes TeX's `cur_chr` for every command. For
`call`, `long_call`, `outer_call`, and `long_outer_call`, that field is the
mutable token-list reference (`def_ref`), not an operand in the macro
meaning. The offline command-stream comparator therefore projects only that
reference address to no operand: it still compares delivery boundary, call
kind, control-sequence spelling, and location exactly. `tex-command` retains
immutable macro definition identity and activation ownership instead of
reintroducing a reference-engine allocation address; snapshots consequently
remain allocation-independent.

The test-only stream adapter translates executor-committed meaning mutations
using the assigned control-sequence key captured at that seam. It never
reconstructs the key from a generic mutation category, so `\let` aliases of
explicit grouping primitives remain comparable while command state stays the
sole owner of delivery and operand scanning.

For TeX82 `handle_right_brace` §1103, replay alone selects the structural
`align_group` branch. The command processor then owns `back_input`, its
literal-brace backup correction, and insertion of immutable frozen `\cr`;
the executor neither manufactures nor replays those raw tokens. This preserves
the §1102/§1103 recovery ordering before v-template delivery.

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

The same ownership applies when a conditional consumes a literal brace in an
alignment cell and a later tab, `\span`, or row terminator reaches main
control with nonzero `align_state`. TeX82 §1102 (`align_error`) first backs up
that exact delimiter, then uses `ins_error` to place the balancing brace above
it. `tex-command` publishes raw and expanded delimiter delivery before this
typed recovery, owns both backup corrections and the inserted recovery level,
and replays the brace before it re-intercepts the original delimiter.
`tex-exec` receives only the typed recovery event and cannot inspect or alter
the underlying token/input ordering.

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

For pdfTeX 1.40.27, `scripts/regen-fixtures.sh --oracle pdftex14027 --profile
initex-etex-eight-bit`
owns acquisition and delegates to the reproducible workflow documented in
[`pdftex14027_oracle.md`](pdftex14027_oracle.md). It emits separately named
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
It is shared by all three reference harnesses and the future test-only
`tex-command` observer; it does not depend on either command engine. The
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

The new command core owns a test-only observer record surface rather than
depending on `tex-oracle`. `CommandObservation` carries command delivery,
logical input, recovery, scanner-status, macro, condition, typed scanner,
token-list, alignment, mutation, and effect records. Command deliveries carry
an opaque origin identity and optional exact registered physical source range
plus its typed canonical location, alongside input-level, cursor-slot, and
processor-delivery provenance. Raw source
delivery installs its backing through the aggregate source-map boundary before
spelling construction; expanded and replayed delivery retain that traced
origin without fixture-derived locations. All other records retain only command-owned
typed values and stable identities. The test-only fixture adapter maps those
owned records to a deterministic replay capture outside the production command
dependency graph. It validates the locked TeX82 suite, scans the terminal root
filename through the command processor, opens only that selected root, and
drives command/executor execution through `CanonicalMainControl` with a
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
Production builds compile this seam out; explicit instrumentation builds enable
it with the `tex-command/instrumentation` feature. Optimized span paths
decompose into the same scalar events when observation is enabled.

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
- discardable generation-guarded caches.

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

1. identical semantic oracle events with observation enabled;
2. identical diagnostics, effects, DVI, and PDF fixtures;
3. a retained scalar fallback or a mechanically obvious equivalence boundary;
4. no new semantic state in a discardable cache;
5. focused microbench evidence explaining the expected win;
6. controlled whole-workload profiling under `docs/profiling.md`;
7. a statistically credible improvement in the targeted workload;
8. no material regression in snapshot capture, rollback, retained memory, or
   WASM behavior; and
9. removal of the prototype if the measured ceiling is negligible.

Performance counters and observer hooks do not write to production hot state
unless their feature is explicitly enabled.

## 33. Main-control integration

`tex-exec` owns main control. For each scalar operation it asks
`CommandProcessor` for an unexpandable `CurrentCommand`, dispatches its
meaning, and performs stomach semantics.

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

An explicit `CurrentCommand` is consumed once. Re-execution after resource
rollback begins from the enclosing executor step, not from a retained command
value.

Each bounded `CanonicalMainControl` operation captures the aggregate command
state, discardable command runtime, mode nest, Universe roots, replay-local
box/alignment/output state, and pending World effects/artifacts before it
creates a processor. A command-core `MissingInput` is translated to a typed
suspension only after that complete rollback; observer records are buffered
until the structural application commits. The next attempt constructs a fresh
processor episode and begins again through `get_next`/`get_x_token`, following
TeX82 §§24--25 rather than retaining a delivered command. Host capabilities
remain borrow-scoped outside this snapshot, so supplying a resource changes
only the next attempt's capability set.

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
