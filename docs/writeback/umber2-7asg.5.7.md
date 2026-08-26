# `umber2-7asg.5.7`: Destination-directed leaf scanners

## Boundary and implementation

Commit `21160f964` converts the command-delivery callers owned by
`scanners/scalar.rs`, `expression.rs`, `font.rs`, `hyphenation.rs`, and
`token_list.rs` to explicit caller-owned destinations. Their co-located scalar
and expression delivery assertions use the same direct interface. No processor
delivery file, structured scanner, token collector, conditional, or executor
file changes in this slice.

Each scanner supplies a call-local `Option<CurrentCommand<G>>` to
`get_token_into` or `get_x_token_into`, classifies `DeliveryStatus`, and moves a
completed command directly into its existing semantic consumer. Expanded
suspension keeps the exact command in its expansion frame; the enclosing scalar
or structured frame keeps the existing typed child destination and resumes the
same scanner phase deepest-first. The local destination never crosses
suspension or becomes command-state storage.

Raw decimal-point rescans remain raw. TeX82 §442's alphabetic-constant path
continues through its dedicated raw helper, including literal-brace correction.
Optional-space absorption, two-level keyword mismatch replay, radix and
fraction termination, internal-value retention, expression parenthesis
recovery, font backup, hyphenation classification without absorbing status,
and token-list right-hand-side ownership are unchanged. Backup still consumes
the live delivery identity and reverses its recorded alignment adjustment at
most once. The change adds no heap indirection, cache, destination search,
compatibility copy, or alternate delivery loop.

## Validation

The focused `tex-command` suite passes with 237 unit tests and 17 integration
tests. The complete routine suite passes with `cargo test -q --tests`.
`scripts/check.sh` reports all four repository gates passed. The three
standalone packed-cutover invocations pass under
`flock /tmp/umber-perf-host.lock`; ordinary delivery, packed replay, mixed
stored spans, stored control-sequence delivery, macro replay/expansion,
keyword mismatch, and the destination-directed row all report zero warmed
allocations and zero requested bytes.

One issue-private authenticated control reused the immutable source,
distribution, schema-12 format, 124-resource offline closure, fixed clock, and
20M fuel boundary under the same host lock. The candidate binary has SHA-256
`34a633893d6a91e75a2a9d0ecdbb88e1e8221d933ce2c530474c50ec75cf051b`.
It intentionally exited 1 at exact fuel exhaustion and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)`. The serial integration
issue remains the owner of comparative profiling; this slice changes no packed
input, distribution, format, corpus, or shared measurement artifact.
