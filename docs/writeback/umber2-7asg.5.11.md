# `umber2-7asg.5.11`: Destination-directed cold executor callers

## Boundary and implementation

Commit `22982619b` routes the three residual material command fetches in
`tex-exec`'s cold scanner through caller-owned
`Option<CurrentCommand<G>>` destinations. TeX82 §§280 and 1269's raw
`\aftergroup` and `\afterassignment` operands use `get_token_into`; the
§§1238--1240 arithmetic target probe uses `get_x_token_into`. Each completed
command moves immediately into its existing cold operation, spelling, target
classification, backup, or diagnostic consumer.

The arithmetic phase remains installed until expanded delivery has produced
its target, so a typed resource suspension resumes at the same operation-owned
phase. End of input retains the prior `MissingToken` or
`UnsupportedAssignmentTarget` result. `off_save` and `align_error` continue to
own their existing command backup and recovery, and their source-line,
provenance, and diagnostic context is still detached while the command
processor borrow is live.

The local destination never crosses suspension and is not command-state
storage. The change adds no raw input loop, delivery adapter, cache, mailbox,
destination inference, command clone, or executor input ownership. Hot main
control, `tex-command`, packed inputs, and benchmark sources are unchanged.

## Validation evidence

The focused `tex-exec` suite passes 694 unit, 4 boundary, and 22 integration
tests. The complete `cargo test -q --tests` routine suite passes, and
`scripts/check.sh` reports all four repository gates passed.

All three standalone packed-cutover invocations ran under
`flock /tmp/umber-perf-host.lock` and report zero warmed allocations and zero
requested bytes, ending with `packed token/macro cutover gate: PASS`. This
includes mixed stored spans, ordinary source delivery, packed backup/replay,
stored control-sequence delivery, macro matching/replay/expansion, keyword
mismatch, and destination-directed delivery.

The authenticated control reused the immutable source, schema-12 format,
packed distribution root `721e833071d92bba`, offline resource closure, fixed
clock, and 20M fuel boundary. The force-frame-pointer binary has SHA-256
`7cac7a4f87b35e3abc62dd82179dcd7df862adee6262151f696a9e21c53ec2e7`.
The serialized row intentionally exited 1 at exact fuel exhaustion and
reproduced `(20000000,19913119,2218327,6020965,16785710,4011)`. Issue-private
evidence is under `target/umber2-7asg.5.11/`; the serial integration issue
remains the owner of comparative profiling.
