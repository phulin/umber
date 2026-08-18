# umber2-awgc.4.3: Resource, Effect, PDF, and Checkpoint Retry Cutover

## Outcome

Production main control now preflights expandable delivery, operand scans, and
immutable resource acquisition before semantic apply. Ordinary, resource,
effect, PDF/page, ErrorStop, observed, tracked, private-revision, named-boundary,
and output-capable box-closing commands do not construct an outer
`StepSnapshot` or `LocalRetrySnapshot`. The aggregate retry adapter is confined
to active-alignment delivery and the diagnostic-expansion host API; their
deletion is tracked separately by `umber2-awgc.4.5`. Artificial deferred-output
expansion still uses its isolated `CommandStateSnapshot` to restore the
surrounding source cursor while preserving TeX82 §1370 conditional changes;
the final residual-snapshot audit remains `umber2-awgc.4.4`.

## Exact continuations

Missing fonts, `\openin` probes, PDF images, and input files retain their fully
scanned typed request. `\input` retains its completed `ScannedFileName`, and
`\immediate` retains the nested PDF primitive already consumed by recursive
lookahead. A retry resolves that value and enters semantic apply without
redelivering the command or rescanning operands. The production resource
fixture records zero replayed deliveries and zero replayed dispatches.

Expandable preflight settles in the same command-processor borrow that emitted
the raw delivery. This preserves macro nesting, conditional tracing, ordered
raw/expanded observations, and the packed input owner's canonical cursor.
Nested expanded collectors retain their accumulated words and exact special
splice route. TeX82 §368 `\expandafter` additionally retains both operands,
and §372 `\csname` retains its accumulated name, so a file enquiry inside
either frame resumes after the blocked command instead of rescanning consumed
input. The executor continuation also retains the post-operand delivery cursor.
ErrorStop deletion and insertion requests are applied at this preflight seam.
Observed suspension additionally moves the sole unpublished evidence buffer
and its opaque delivery-order cursor. Retry therefore preserves exact delivery
sequence and §1038's raw character-lookahead boundary without cloning an
observer or provenance aggregate.

## Narrow operation ownership

Semantic apply mutates the canonical owners directly after preflight. Mode,
page, PDF, effect, output, provenance, dependency, and observer publication use
their existing owner journals and append boundaries; failure-prone host
resolution has already completed. Private incremental revisions pair the
operation with `DirectOperationMark`, which contains only the disposable patch
allocation suffix mark. Successful and canonical partial commits close or
discard that suffix without retaining aggregate state roots.

Tracked regions begin before expandable preflight, so scanner reads and
resource failures have the same dependency evidence as the former aggregate
path. Observed execution closes its receipt before direct commit and publishes
only a successful operation. Resource suspension retains the typed request and
unpublished receipt without exposing effects, artifacts, or named
boundaries.

## Validation

Focused resource, immediate-PDF, ErrorStop, observer, tracked-region,
private-revision, PDF retry, conditional-tracing, and exact observed-retry
tests pass. The focused suites pass: 746 `tex-command` tests plus 17 boundaries,
560 `tex-exec` tests plus 4 integration and 21 exact fixtures, and 923
`tex-state` tests plus 11 integration and 2 format tests. The workspace suite
passes with `RUST_MIN_STACK=33554432`; the default test-thread stack exposes an
unrelated recursive-drop overflow in `tex-typeset`'s 20,000-deep replacement
stress test, tracked as `umber2-awgc.14`. `scripts/check.sh` passes all four
repository format and clippy gates.
