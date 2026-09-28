# Command lane

The command lane is main control's minimal inner loop. It fetches, classifies,
executes, and settles the most frequent commands inside one admitted command
context, with the per-command work reduced to what TeX82's `big_switch`
actually does for them. Every other command, and every context the lane does
not own, leaves through a single hand-off to the existing admitted run and
typed episode, which remain the only owners of uncommon semantics.

The lane is not a second executor. It calls the same scanners, the same
semantic appliers, and the same journals as the admitted run. What it removes
is repeated generic work: per-command context sampling, facade construction,
trace and barrier classification, cold-operation materialization, empty
buffer clearing, and journal re-capture.

## Measured floor

Per-operation user-mode instruction counts (`perf stat`, 100,000 iterations
of ten operations inside one `\iter` macro, the empty-loop run subtracted)
at `9e9fcfb06`:

| Operation                 | Instructions |
| ------------------------- | -----------: |
| `\relax`                  |        2,808 |
| `\let\a\relax`            |        3,896 |
| `\def\a{x}`               |        6,565 |
| `\begingroup\endgroup`    |        7,802 |
| `\count1=5`               |        6,847 |
| empty macro call          |          383 |
| one-argument macro call   |        2,751 |
| `\ifx\a\a\fi`             |        2,561 |
| `\iffalse x\fi`           |        1,902 |
| `\expandafter` (net)      |       ~1,850 |
| `\csname relax\endcsname` |        7,141 |

pdfTeX spends roughly 90 instructions on `\relax`. The `\relax` floor
decomposes into the admitted-loop body (~710), the expanded fetch of one
resident macro-body word (~540), the dispatcher and cold `Relax`
materialization (~400), the four journal rolls (~450), command-processor
construction (~100), clearing an empty page-observation buffer (~100), and
smaller diagnostic, trace, and evidence probes (~500). None of that work is
semantic for `\relax`.

The book census (`--profiling-stats`, 2606.24937) fixes the frequency order:
35.9M main-control operations, led by `\let` (8.6M), `\def`/`\edef`/`\gdef`/
`\xdef` (6.1M), `\relax` (2.3M), `\global` (2.2M), `\futurelet` (1.5M),
`\advance` (1.1M), register and parameter assignments (3.2M), and
`\begingroup`/`\endgroup` (1.6M). Expansion runs 38M macro calls, 16.1M
`\expandafter`, 8.8M `\fi`, 8.3M `\ifx`, 3.0M `\else`, and 2.9M `\csname`.

## Lane eligibility

The lane is entered from the admitted loop only when every lane invariant
holds. Invariants are sampled once at entry. A lane command either cannot
change an invariant, or leaves the lane after settling when it might have.

- The character main loop is inactive, and no pending horizontal character run
  remains to flush.
- The mode is vertical, internal vertical, horizontal, or restricted
  horizontal. Math modes keep their mode-specific scan prelude.
- No alignment is active, and no display alignment is pending.
- No operation observer is attached, and no tracked region is active.
- No page-region succession, paragraph checkpoint cut, pending leader,
  output-routine opening brace, or alignment-recovery brace is pending.
- No diagnostic effect or first recoverable diagnostic is pending.
- `\tracingcommands` is not positive. It is re-read after each lane
  command, since a group transition can restore it.
- The run is below its operation limit.

The first lane generation keeps the existing admitted-loop entry. It enters
the lane only for a context the admitted loop already accepts, so the
episode's rollback, fuel, and slice-limit contracts are unchanged.

## Lane loop

```text
lane:
  loop
    status = processor.preflight(&mut slot)     // ordinary expanded delivery
    if status is not Command or a report is pending -> Delivered
    match lane_family(slot.meaning, innermost group)
      Relax                          -> settle, continue
      Global                         -> fetch §404's next command; fold it
                                        into an assignment or hand it off
      Let | FutureLet                -> scan; commit; afterassignment; settle
      MacroDefinition                -> scan; commit; afterassignment; settle
      CatCode                        -> scan; commit; afterassignment; settle
      Scalar                         -> cold scan; commit; afterassignment;
                                        settle
      SemiSimpleGroup | SimpleGroup  -> enter/leave; aftergroup; settle
      _                              -> Delivered
```

The classification is one match on the resolved meaning. It never consults
tracing, barrier, or `\pdfoutput` tables. No lane family has a transaction
barrier, and a family that could need one is not a lane family. A `}`
closing a group owned by an active box can reach page or shipout work, so it
is a lane family only when its group kind is not an active box's.

A lane command is settled only below the operation limit. Settling it
performs exactly these steps:

1. Re-check the facts a settled command can change: pending reports and
   diagnostic effects, artifact and effect counts, and `\tracingcommands`.
   If any changed, leave through the _Applied_ exit.
2. Fold the checked save-stack words into the run's maximum.
3. Increment the episode's operation count and settle the command's
   rollback unit, as described below.

Only the commands that can change page, box, or host state, which are never
lane families, need the admitted loop's full continuation predicate.

## Hand-off

The lane borrows the admitted run's command processor and applies lane
commands through it, so a lane command never becomes a `ColdOperation`, and
the processor, its sampled facts, and the episode frame are reused across
lane commands. The processor lends its admitted state, command root, and
diagnostic sink to the hot committers between two deliveries; none of its
processor-local facts caches a meaning, category code, or group level.

The lane leaves through exactly one of six exits, each with the slot's
command where ordinary delivery would have left it:

- _Delivered_: the delivery status and any unscanned command, which the
  admitted run dispatches. A command is resumed in place, never backed up or
  re-delivered.
- _Prefixed_: a command that §1211's prefix loop fetched after a `\global`,
  when the lane does not own it. The admitted run's dispatcher continues the
  prefix loop from that command with the accumulated prefix, including
  §1212's error for a command that takes no prefix. When the command needs a
  barrier or a report is pending, it becomes resident with a _prefixed_
  delivery phase, which the resident dispatch continues in the same way and
  which the transaction predicate treats exactly as a resident prefix.
- _Scanned_: a lane command whose delivery or scan left a report. The
  admitted run publishes the report before it applies the scanned operation,
  as it does for its own hot operations.
- _Scanned cold_: a scalar command left in the admitted run's cold slot
  because its delivery or scan left a report, its target is invalid, or its
  arithmetic overflowed. §1236 reports these before writing, so the lane
  declines the write and the admitted run applies the cold operation,
  including its report.
- _Scan failed_: the admitted run's dispatch-error path.
- _Applied_: a lane command applied in place that failed, or left a report,
  an effect, an artifact, or a positive `\tracingcommands`. It settles
  through the admitted run's continuation predicate, which publishes the
  report, counts the operation, and ends or continues the run.

## Rollback units

An operation mark delimits a replayable suffix: discarding it restores every
journal to the mark. The admitted run discards the current unit on a
resource suspension and on every non-fatal error it returns, so the failing
command's partial writes vanish while every earlier command stays. Each
command the lane settles in place therefore settles its own unit, exactly as
the admitted run's roll does, and the unit that ends the lane holds only the
command that ended it. Lane commands never write the mode nest, the page
list, or the active boxes, so the state and command-attempt journals are the
only ones the lane rolls.

The rolls are the lane's largest remaining per-command cost. The
[Positional journal marks](positional_journal_marks.md) design replaces them
with position captures.

## Prefixes

`\global` is §1211's prefix loop run inside the lane: fetch §404's next
non-blank, non-`\relax` command, and fold the prefix into it when it is a
lane assignment (`\let`, `\futurelet`, a macro definition, `\catcode`, or a
scalar assignment).
§1214's `\globaldefs` is resolved once, before the scan, as the admitted
run does. `\long`, `\outer`, and `\protected` apply only to definitions,
are rare, and stay with the admitted run. A scan error keeps the origin of
the first prefix, as the admitted dispatcher's does.

## Fetch

The lane fetch is the ordinary expanded delivery (`get_x_token`), minus the
per-command wrapper work the generic preflight adds:

- It never records a delivery cursor, prepares a command trace, or drains
  semantic diagnostics, because lane eligibility forbids all three.
- It leaves the command compact in a `LaneCommandSlot`, so a lane command
  never becomes a `CurrentCommand` and only a hand-off materializes.

The shared delivery kernel stays the only reader. Fetch-kernel improvements
(dense meaning decode, the frame-local resident reader, fuel batching) apply
equally to lane and generic delivery. See
[Command delivery kernel](command_delivery_kernel.md).

## Expansion fast arms

The census shows expansion events outnumbering main-control commands two to
one. They follow the same principle, applied inside the one expansion loop:

- Macro calls with only undelimited parameters, whose arguments are single
  tokens or balanced groups without `\par` or outer tokens, match inside the
  argument run and publish their argument frame without the delimited-matcher
  setup.
- `\expandafter` reads its two tokens through the resident reader and
  expands the second in place. The first is pushed through the single-token
  backup slot.
- `\ifx`, `\iftrue`, `\iffalse`, `\else`, and `\fi` evaluate and settle
  inside the conditional stack without materializing a command for either
  comparand.

These arms remain owned by `tex-command`'s expansion loop. They are not a
separate expansion engine.

## Semantics and observation

The lane changes no observable TeX behavior. Tracing, observation,
diagnostics, alignment, and math are ineligible contexts, so the full
admitted path still handles them. Fuel is charged per delivered token by the
shared kernel, as before. `\afterassignment` and `\aftergroup` are scheduled
by the same helpers the admitted run uses. Save-stack usage statistics are
captured by the same capture, performed only when a lane command changed the
save stack.

## Validation

- Unit tests compare lane and generic execution on the same input for each
  lane family, with observers attached (generic) and detached (lane). They
  check resulting state, fuel, and produced output.
- The routine suite, command fixtures, trip, and the oracle suites must stay
  unchanged, since lane ineligibility routes every traced and observed run
  through the generic path.
- The per-operation instruction table above is re-measured after each stage.
  Book PDF and aux output must remain byte-identical.

## Stages

1. The lane skeleton: eligibility, a single match, `\relax` settlement and
   merge, and the hand-off.
2. `\let`, `\futurelet`, macro definitions, `\catcode`, and group
   transitions through the existing hot scanners and committers.
3. Prefixes. `\global` folds into a following lane assignment. A prefix
   followed by a command the lane does not own needs a hand-off that carries
   the prefix into the admitted run's transaction path without backing up
   the command it fetched.
4. Rootless scalar assignments and `\advance`/`\multiply`/`\divide`.
5. Per-command generic overhead that the lane still shares: journal roll,
   facade construction, and the fetch wrapper.
6. Expansion fast arms, in census order: macro calls, `\expandafter`, and
   conditionals.
