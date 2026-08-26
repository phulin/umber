# `umber2-7asg.5.15`: destination-directed caller integration

## Integrated boundary

The final cross-crate architecture assertion walks production Rust sources in
`tex-command` and `tex-exec` and rejects every value-returning raw, token,
expanded, replay-aware, nonblank, settlement, and alignment delivery call used
by the migrated caller set. Every ordinary production request now provides its
final `Option<CurrentCommand<G>>` directly. The one remaining
`get_x_token_preserving_undefined` convenience is pinned to
`diagnostic_expand_step`, the cold host-facing diagnostic boundary whose
consumer is selected only after undefined-command classification.

The same boundary suite proves that `get_next_canonical` and
`deliver_raw_input_into` each have exactly one implementation, that the retired
`take_input_token`, `ActiveInput`, and `DeliveredToken` envelope is absent, and
that input levels remain semantic-free. Typed suspension destinations remain
explicit child edges; there is no command mailbox, result tape, destination
search, or redispatch fallback. Main control's `reswitch` and §1270 paths
continue to dispatch their already delivered command in place.

The standalone destination-directed row now warms and measures 8,192 calls for
each of `get_next_into`, `get_token_into`, and `get_x_token_into`, reusing one
caller-owned command slot. This covers raw non-creating, raw creating, and
expanded policy entry points instead of measuring only the raw creating
wrapper.

## Semantic and allocation evidence

The focused `tex-command`, `tex-exec`, and `tex-command-stream` suites pass.
Together they retain the command-semantic and corpus fixtures, rollback and
replay completion, typed resource suspension, alignment and recovery,
tracing, and provenance ordering exercised by the migrated subsystems. The
complete routine suite and repository quality gates are recorded at issue
close.

All three standalone packed-cutover invocations ran under
`flock /tmp/umber-perf-host.lock`. `warmed_mixed_stored_cursor` and
`destination_directed_warm_delivery` report zero allocations and zero
requested bytes, and the complete gate ends with
`packed token/macro cutover gate: PASS`. The complete row includes ordinary
source delivery, backup and replay, every stored owner, long macro arguments,
control-sequence delivery, macro matching and expansion, keyword mismatch,
and all three direct delivery policies.

## Authenticated exact 20M profile

The issue-private force-frame-pointer ELF has SHA-256
`cb8b5bffe3124300ed5ca1aaca538b5df5d001f9ce0a8cfc63eca6c0f4b57a40`.
Both the warmed control and profile used the immutable arXiv `2606.12566`
source, schema-12 `pdflatex.fmt`, packed distribution root
`721e833071d92bba`, authenticated 123-key closure, offline policy, fixed clock,
45-second and 1,536 MiB guards, one issue-private cache, and the required host
lock. Both intentionally exited at exact fuel exhaustion with work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`.

The `cycles:u`, 199 Hz caller/callee capture contains 2,367 samples, zero lost
samples, and exactly 25,834,083,890 weighted cycles. Its `perf.data` SHA-256 is
`223fbbc35a740983a5de5ce8f82563666e15836c02ea5bcc6bbfefa923e31214`.
The table compares the final integration only with `.5.4`, the accepted
destination-directed raw-delivery foundation. It does not attribute `.5.3`'s
borrowed-row change to caller migration.

| Measure                                    | `.5.4` foundation | Final integration | Absolute change | Relative change |
| ------------------------------------------ | ----------------: | ----------------: | --------------: | --------------: |
| `get_next_canonical` self cycles           |     1,659,371,328 |     2,355,030,342 |    +695,659,014 |         +41.92% |
| `get_next_canonical` inclusive cycles      |     4,933,173,540 |     5,778,957,793 |    +845,784,253 |         +17.14% |
| `CurrentCommand::resolve_into` self cycles |     1,147,951,035 |     1,091,057,792 |     -56,893,243 |          -4.96% |
| `CurrentCommand::resolve_into` inclusive   |     1,336,658,709 |     1,300,108,153 |     -36,550,556 |          -2.73% |

These are absolute sampled period sums, not percentages multiplied by the
capture total. `get_next_canonical` inclusive already contains resolver
descendants, so the two symbols must not be added. The measured canonical
increase is retained as a regression rather than hidden by the direct-caller
migration; conversely, the separately sampled resolver decrease is not claimed
as a caller-migration saving. Issue-private raw events and derived self,
inclusive, and exact symbol-period reports are under
`target/umber2-7asg.5.15/perf-20m/`.
