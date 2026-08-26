# `umber2-7asg.5.12`: Destination-directed main control

## Boundary and implementation

Main control now supplies operation-local command destinations for raw
preflight, ordinary expanded fetches, startup filename parsing, optional-space
lookahead, no-align bodies, alignment-aware `\ignorespaces`, prefix scanning,
leader handoff, `\noboundary`, and terminal exhaustion. Compact
`DeliveryStatus` values carry replay completion and alignment events without
returning a command-bearing envelope. `hot_apply.rs` already consumes settled
commands and owns no delivery call of its own.

The executor-local nonblank loops reuse the same destination while command
delivery and expansion remain wholly owned by `tex-command`. Preflight moves
the raw command into in-place settlement and moves its result directly to
`OperationDelivery` or `PendingPreflightCommand`; it neither backs up nor
redelivers the settled command. Main-loop raw lookahead, `goto reswitch`,
prefix and leader ownership, command tracing, raw/expanded/alignment
observation order, command-owned provenance, and typed scanner child/cursor
chains are unchanged. The specialized diagnostic undefined-preserving entry
remains a cold value-returning boundary because this slice does not change the
command API and its destination is selected only after classification.

No `cold/scan.rs`, `tex-command`, benchmark, or boundary file changes are part
of this slice.

## Validation evidence

The focused `tex-exec` suite passes 694 unit, 4 boundary, and 22 integration
tests. The authenticated pinned arXiv control intentionally exhausted its 20M
fuel limit and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` exactly. The run used the
immutable source, schema-12 format, authenticated packed distribution,
offline prefetch closure, fixed clock, issue-private cache/output under
`target/umber2-7asg.5.12/`, and `flock /tmp/umber-perf-host.lock`.

The mixed stored-span, destination-delivery, and complete packed cutover gates
all report zero warmed allocations and zero requested bytes and end with
`packed token/macro cutover gate: PASS`. The complete routine suite and
repository gate results are recorded at issue close.
