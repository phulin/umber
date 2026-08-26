# `umber2-7asg.5.8`: Destination-directed structured scanners

## Boundary and implementation

Commits `2ee66015f` and `922b56f35` route every material raw and expanded
command fetch in `scanners/structured.rs` through an operation-local
`Option<CurrentCommand<G>>` destination. The converted callers cover
definition targets, general text and filenames, show/write stoppers,
immediate-extension and box/alignment lookahead, alignment-preamble `\span`
expansion, math-field and accent classification, assignment handoff, and raw
`\let`/`\futurelet` probes. Protected output replay uses the processor-owned
replay-aware destination API and consumes its compact completion status
without a command-bearing return envelope.

Each loop owns its command slot only for the live delivery request. Commands
move from that slot directly into classification, backup, execution handoff,
or the existing typed suspension owner. The alignment-preamble continuation
still retains only the exact command whose one-step expansion suspended; no
call-local delivery slot crosses the resource boundary.

Raw versus expanded fetch selection, scanner status, provenance, observation
order, replay completion, and alignment interception are unchanged.
`\futurelet` still backs up its second probe before the saved first probe.
Math and accent assignment commands remain delivered and move to the executor
without backup or duplicate delivery. The migration adds no cache, mailbox,
result tape, heap indirection, compatibility copy, or alternate delivery loop.

## Validation evidence

The authenticated pinned arXiv control intentionally exhausted its 20M fuel
limit and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` exactly. The current
frame-pointer profiling binary has SHA-256
`82a5c1277eea2e8a4cc53187583c61e877ced0530524f3099438e453902f5ca3`.
The host run was serialized with `flock /tmp/umber-perf-host.lock`, reused the
immutable source, format, distribution, and 123-key closure from the accepted
control, and wrote only issue-private cache and output under
`target/umber2-7asg.5.8/`.

The mixed stored-span, destination-delivery, and complete packed cutover gates
all report zero warmed allocations and zero requested bytes and end with
`packed token/macro cutover gate: PASS`. Their receipt SHA-256 values are
`50b234ac6c27a562a0315073baff6928b1a13efa7266245c86e803fdca47be71`,
`48a903010429b079272bc4b750e1dd2e5bfc6595e558531621153000fb44c43b`,
and `43e7082c1f70fa8cbff95bec0ce96cd25c94d58472ae0d57e46ebb6e07367870`.

The focused `tex-command` suite passes 238 unit and 17 boundary tests; the
focused `tex-exec` suite passes 694 unit, 4 boundary, and 22 integration tests.
The complete `cargo test -q --tests` routine suite passes, and
`scripts/check.sh` reports all four gates passed.
