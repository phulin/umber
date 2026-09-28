# Positional journal marks

Main control settles every command as its own rollback unit. The admitted
run discards the current unit on a resource suspension and on every
non-fatal error it returns, so a unit must never hold more than one
command. Settling a unit ("rolling" its mark) therefore runs once per
command, and its cost is paid by every command, including `\relax`.

## Measured cost

Settling each lane command costs 535 instructions on `\relax` (575 without a
roll, 1,110 with one) and 17B instructions on the book. Rolling the same mark
twice with nothing in between still costs 251 instructions for the state lane
and 284 for the command-attempt lane, so the cost is bookkeeping, not log
work:

- The state roll ends the operation's transaction and begins a new one: it
  decrements and increments the transaction depth, runs the durable-box
  commit's action and entry drains on empty vectors, and rebuilds a
  `StateOperation` with its durable-box token through fallible accessors.
- The attempt roll's unchanged path captures a thirteen-field arena mark
  through checked conversions, compares it, and validates the operation's
  scope coordinates. That costs about 240 instructions even when nothing
  was allocated.

## Design

A mark is a set of lengths plus the group coordinates it must restore.
Rolling a mark updates it in place. It never allocates, never reconstructs an
ownership token, and costs a handful of comparisons when the unit wrote
nothing.

### State lane

The admitted run opens one state transaction and holds it until the run
ends. Rolling the mark keeps the transaction depth. At depth one it clears
the dead suffix, since every entry belongs to the settled command, and it
records the new position. It then recaptures the group coordinates: group
depth, save-stack projection, save position, and the innermost group's entry
count and sparse start. Writes keep recording exactly as they do now, because
recording depends only on the transaction depth being nonzero.

The durable-box lane keeps its operation, since its outermost commit
performs real work: loan commits and value retirement. Its roll returns
immediately when its entry, action, scalar, and group positions all equal
the operation's. Only commands that touch box registers take the full
commit and begin.

### Command-attempt lane

The attempt arena is per-command scratch. Its roll compares raw table
lengths against the opening mark directly, without checked conversions,
and checks the scope coordinates in the same pass. It updates the stored
mark in place instead of moving it out of and back into the command state.
An operation that allocated takes the existing commit and begin, so every
scratch id it issued is invalidated by the scope serial as before.

### Group transitions

Each unit holds one command, so a mark spans a group close only inside the
command that closes the group. Restoring such a mark is the existing
`InvalidCursor` case, which group-closing commands already avoid by never
failing after the close. Journaling group exits would let marks survive
them, but it would only matter for merged units, which discard-on-error
semantics rule out.

## Rejected alternative

An append-only undo log with group-exit records and a single `u32` mark per
run would let a lane run merge its commands into one unit. Merging is
unsound under the admitted run's error contract, and cheap per-command marks
remove its only benefit.

## Validation

- Routine suite, including the rollback, resource-replay, and pdfTeX
  checkpoint tests.
- Back-to-back roll microbenchmarks and the lane operation table in
  [Command lane](command_lane.md).
- Book output byte-identical apart from timestamps.
