# `umber2-66p0.8.40.155`: occupied command destination

## Adopted boundary

`raw_delivery_entry` now initializes the caller's `CurrentCommand` slot once
and keeps one direct mutable borrow through resident retry, settlement,
observation, alignment interception, and successful return.
`expanded_delivery_entry` likewise initializes or readmits the exact
`x_token`/suspended command once, then keeps that destination occupied through
fetch, classification, and synchronous expansion. Neither ordinary loop probes
vacancy, reinstalls a placeholder, repeatedly recovers `as_ref`/`as_mut`, or
takes the command on success.

Cold end, replay completion, and failure alone clear a provisional output. A
genuine immutable-resource suspension replaces the resident value once and
parks the prior command owner; resumption restores that same owner before the
single loop continues. Macro, provenance, recovery, alignment, observation,
rollback, and expansion-depth semantics retain their prior owners. No second
representation, policy driver, command path, cache, unsafe alias, or loop was
added. Production and boundary-test source changed by net 12 lines deleted.

## Focused comparison

The exact assigned base `95364dae9` and candidate were built independently in
release/profiling mode. One matched `fused_raw_expanded_delivery` comparison
performed 1,000,000 raw plus 1,000,000 expanded deliveries across replay,
attempt, and durable storage. Both reported the exact 2,000,000 fuel/frame/
meaning counts, 1,000,000 expanded completions, zero relays, zero command
copies, and zero warmed allocations or requested bytes.

| Measure                            |             Base |        Candidate |                Delta |
| ---------------------------------- | ---------------: | ---------------: | -------------------: |
| User instructions                  |    1,041,692,737 |    1,033,691,780 | -8,000,957 (-0.768%) |
| User cycles                        |      702,184,533 |      666,114,613 | -36,069,920 (-5.14%) |
| Whole executable text              |      2,119,848 B |      2,116,180 B |   -3,668 B (-0.173%) |
| Raw/expanded entry code            |      668/1,528 B |      567/1,595 B |           -101/+67 B |
| Combined entry code                |          2,196 B |          2,162 B |       -34 B (-1.55%) |
| Warm allocations / requested bytes |            0 / 0 |            0 / 0 |            unchanged |
| Public `memcpy` calls / bytes      | 137 / 24,338,126 | 137 / 24,338,086 |            0 / -40 B |
| Public `memmove` calls / bytes     |            2 / 0 |            2 / 0 |            unchanged |

The public-copy tables reconcile exactly with zero collision probes, overflow,
or probe-internal calls. Diagnostic elapsed time was 102.97/111.62 ns per raw/
expanded delivery at base and 94.39/106.87 ns at candidate; retired
instructions and code size are the selection evidence.

Baseline/candidate binary SHA-256 values are
`561ea1fdf4c8b735d1ee60c5dd9b9a06a3572091b0c45774d1bb72ff632c7ace` and
`f8081faf5c78e62bc6a9558ef38a3bc65db1740f11e9ecac08370205e0065c73`.
Their perf receipts are `c8aaf6d7ef3ae19aacff759472f6e68aec95d4dcf09a9c224dc889355ffc27f4`
and `5236fb14f95aa0b360b96c899edbf0eeb70c35949b536ba099f28cdc6f5caffa`;
their symbolized copy reports are
`943178980808a5c8260f43b265b9a7c09878a8811eb1555452abd5446b10b7ff`
and `7fb7f9eb89e2bc4c7964226a3904fb321e001380a0899e69014a50085523a277`.
Ignored evidence is under `target/umber2-66p0.8.40.155/`.

## Validation

- `cargo test -q -p tex-command --tests`: 391 unit tests and 23 boundary tests pass.
- `fused_raw_expanded_delivery`: exact semantic/copy census and zero warm allocation pass.
- `scripts/check.sh`: all four default gates pass.
