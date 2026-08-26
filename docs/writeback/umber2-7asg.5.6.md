# `umber2-7asg.5.6`: Destination-directed expansion callers

## Boundary and implementation

Commit `0ab6b13c1` routes every production raw, token, and expanded fetch
owned by `processor/expand.rs` through the caller-provided final
`Option<CurrentCommand<G>>` destination. This includes active-character
settlement, nonblank helpers, `\pdfprimitive`, `\noexpand`, `\expandafter`,
`\csname`, `\string`, and `\meaning`. The internal value-returning
active-character bridge is gone. Public value-returning entry points remain
thin boundary conveniences over the same destination driver.

The protected replay-aware policy now has the matching
`get_x_or_protected_with_replay_completion_into` entry point. It preserves
e-TeX's terminal protected-macro behavior and replay-completion surface while
letting its structured-scanner consumer provide the final command slot.

Meaning resolution, raw and expanded observation order, replay completion,
alignment interception, and active expansion-depth accounting retain their
existing single-driver owners. `PendingExpansion` continues to retain exactly
one completed command and one typed child destination at a resource barrier;
the migration adds no destination search, result tape, command mailbox,
compatibility copy, cache, or raw slot that survives suspension.

## Validation evidence

The authenticated pinned arXiv control intentionally exhausted its 20M fuel
limit and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` exactly. The current
frame-pointer profiling binary has SHA-256
`797f1ea2cc1423c83fd62934504db75728e8fbf247ec34d844315ae5b0a34bd2`.
The host run was serialized with `flock /tmp/umber-perf-host.lock`, used the
immutable source, format, distribution, and 123-key closure from the accepted
control, and wrote only issue-private cache and output under
`target/umber2-7asg.5.6/`.

The mixed stored-span, destination-delivery, and complete packed cutover gates
all report zero warmed allocations and zero requested bytes and end with
`packed token/macro cutover gate: PASS`. Their receipt SHA-256 values are
`64c567184e539d457dff293b74e8e0c173f5548fe7871323a331df212f3a030a`,
`48a903010429b079272bc4b750e1dd2e5bfc6595e558531621153000fb44c43b`,
and `c67c951b74217cd46bebb235b01313567b2e9f4b35b115e0093b5b65096cd22e`.

The focused `tex-command` suite passes 238 unit and 17 boundary tests. The
complete `cargo test -q --tests` routine suite passes, and `scripts/check.sh`
reports all four gates passed.
