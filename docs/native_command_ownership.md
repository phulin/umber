# Native command ownership

The native interpreter fetches, scans and applies a command, then goes directly
to its next fetch label. There is no successful-command settlement or cleanup
stage. This supersedes the boundary-clear scratch and repeated demand-checking
policy described by earlier hot-loop documents. Beads `umber2-i0n5.16` tracks
the replacement and its semantic and performance evidence.

## Scratch ends at its consumer

A scratch collector and its sealed list occupy the same arena row. The sealed
coordinate is a move-only capability; reading borrows it, and publication,
insertion, rendering or splicing consumes it. Consumption returns the actual
chunks and retires the row. Consumed holes below another live collector stay
vacant until the live suffix retires; no separate free-row table is necessary.
Monotone row serials reject stale coordinates after suffix reuse.

A scanner that fails discards only its own unfinished suffix. Child-scope
marks use a serial frontier, not a vector length that consumption can reduce.
Thus consuming a child or an older finished list does not invalidate an active
scanner's scope. Successful consumers release their lists without visiting
unrelated scanner owners. A halted job can retain failing owners until job
destruction or full checkpoint replacement; that is an actual coarse lifetime,
not a successful-command cleanup pass.

## Save statistics belong to stack changes

The native save journal already maintains current word usage and the newest
checked push. It also records the maximum checked usage at actual save pushes.
It retains the classic and e-TeX source-line projections as scalar maxima,
because those profiles can assign different maxima to the same stack history.
Native box entry and exit update the same owner's box-spec contribution; an
executor-side duplicate word counter is unnecessary.

These job maxima never rewind on unsave, checkpoint restore or candidate
rejection. Host statistics read them at their own reporting boundary. A token
read, a macro invocation or an unrelated terminal command does no save-stack
accounting and does not traverse stack owners.

## Trace state belongs to tracing

The processor's observation loan and persistent scanner/alignment regimes have
their own begin/end transitions. Tracing demand is read where a trace or a bulk
read needs it. It is not recomputed by a post-command reset. The actual main
trace entry initializes its prefix, printed flag and count; an untraced command
does not reset absent trace state.

## Semantic events own their continuation

An assignment inserts its pending afterassignment word at its actual
assignment completion. Unsave inserts aftergroup words while saved owners are
live. Input publication consumes its actual token owner. These actions precede
the next fetch and do not pass through a generic operation-commit function.

Reports retain their TeX chronology: scanner reports publish before the
semantic action that consumes the operand; semantic reports publish at the
reporting action. A synchronous error marks its processor loan at the error
producer, so its actual reporter captures the cause before another command.
Ordinary commands do not poll the World error counter. Output construction, shipout, page-region succession and
mode changes retain their real ownership cuts. Observation and dependency
tracking publish their requested evidence at their actual instrumentation
boundary. None authorizes an ordinary post-command sweep over page, World,
input, scratch, trace and save-stack state.

The bounded host driver still counts native commands and returns for its
budget, cancellation, terminal, resource, named-checkpoint or demanded-evidence
boundary. Those host units are not command transactions. Full checkpoint
admission and restore retain the inventory and root-before-payload rules in
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md).

## Verification

Owner tests cover consumption, nested scopes, stale coordinates, failed
collectors and borrowed immutable publication. Alignment preamble collectors retain their actual live owners across host
steps and release them when the preamble finishes. Native save-stack tests cover
mixed group/box words, both source-line profiles and nonrefundable maxima.
Trace, afterassignment, aftergroup, errors, output and observation retain their
semantic tests and ordered canonical comparison. Matched release profiles and
microbenchmarks measure the deleted command-boundary work separately from
the older baseline's already completed refactors.
