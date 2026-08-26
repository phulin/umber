# `umber2-7asg.5.13`: Destination-directed macro collection

## Boundary and implementation

Commit `1e5179f72` removes the last value-returning command fetch owned by
`scan_toks.rs`. TeX82 §483's unbalanced `read_toks` line-discard loop now
reuses its existing call-local `Option<CurrentCommand<G>>`, classifies the
compact `DeliveryStatus`, and drops each completed command directly. The
focused macro-call and token-collection tests use the same destination
contract; `macro_call.rs` already had direct destinations at every material
fetch and required no production change.

Parameter candidates still restart inside canonical raw delivery before a
final command is constructed. Macro compulsory-prefix matching, delimiter
overlap and ownership, outer-pair stripping, paragraph and outer recovery,
balanced replacement collection, expanded collector suspension, scanner
episode visibility, tracing, provenance, and deepest-first child teardown are
unchanged. In particular, `read_toks` still drains the complete remaining
line before restoring `align_state` to `1000000`; the migration adds no
command clone, return envelope, cache, heap indirection, compatibility copy,
or suspension-owned local slot.

## Validation evidence

The focused `tex-command` suite passes 241 unit tests and 17 external boundary
tests. The complete `cargo test -q --tests` routine suite passes.
`scripts/check.sh` reports all repository gates passed.

The three packed-cutover controls ran under
`flock /tmp/umber-perf-host.lock` with issue-private receipts below
`target/umber2-7asg.5.13/`. Mixed stored spans, destination-directed delivery,
and macro matching/replay/expansion each report zero warmed allocations and
zero requested bytes; every invocation ends with
`packed token/macro cutover gate: PASS`. The scanner's established
publication-allocation baseline is unchanged because this slice changes only
command delivery and introduces no allocation site.

The authenticated schema-8 arXiv control used immutable packed inputs, the
schema-12 `pdflatex.fmt`, the 123-key prefetch closure, fixed clock, offline
policy, 45-second and 1,536 MiB guards, and an issue-private cache and output
directory. It intentionally exited 1 at exact fuel exhaustion and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` exactly. The candidate
profiling binary has SHA-256
`ff30f061a4bb9f3011f0852d714c5f626d39b1cd0d2eb70400928b6b8d345003`.
