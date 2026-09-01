# `umber2-66p0.8.40.130`: hand classified expansion directly to dispatch

## Selection authority

The integrated `.127` authenticated 20,000,000-command capture remains the
sole broad authority. At exact work vector
`(20000000,19907047,2216876,6018541,16781945,4011)`, `expand_into` ranked at
2.95% application self after `.127` removed duplicate delivery census writes,
`.128` removed the universal resident-input carrier, and `.129` removed the
ordinary freshness-coordinate publication. No new corpus execution or broad
profile was run.

The expanded loop had already matched the resolved meaning once and selected
one exact `ExpansionDispatch`. It then wrapped that scalar in
`Some(dispatch)` for `expand_into`, whose first ordinary-path action was to
discriminate the option and recover the same scalar. The option represented
neither TeX expansion semantics nor suspension ownership.

## Architectural simplification

The expanded loop now hands its selected dispatch directly to
`expand_classified_into`. The shared dispatch body receives a non-optional
`ExpansionDispatch` and never rereads the command meaning. Exact TeX `expand`
callers retain `expand_into` as the classification wrapper; it restores parked
work only when a real expansion suspension is present, classifies the restored
command once, and enters the same shared body.

This removes exactly one nonsemantic result carrier and its repeated
discrimination from ordinary expansion. The caller-owned command remains in
its final destination. Provenance, macro argument ownership, scanner child
capture, resource suspension/resumption, trace suppression, backup, rollback,
retirement, and raw/expanded delivery semantics are unchanged.

## Focused before/after gate

The exact baseline is current main `7c8ffc8c1` before this change. Both release
binaries ran the production-profiling `mixed_macro_resident_pipeline` once
under `perf stat` and once under the checked public-copy interposer. Both
report 2,000,000 macro-body transitions, 1,000,000 parameter deliveries,
1,000,004 replay words, 2,000,004 raw frame steps, 1,000,000 expanded
deliveries, 1,000,001 macro expansions, zero suspension moves, zero command
copies, and zero warmed allocations or requested bytes.

| Counter                              |      Baseline |         Final |                  Delta |
| ------------------------------------ | ------------: | ------------: | ---------------------: |
| User instructions                    | 2,413,740,346 | 2,402,739,856 |   -11,000,490 (-0.46%) |
| User cycles                          | 1,602,124,134 |   916,926,096 | -685,198,038 (-42.77%) |
| Internal elapsed nanoseconds         |   632,604,125 |   353,516,157 | -279,087,968 (-44.11%) |
| Classified dispatch function bytes   |        21,327 |        20,538 |          -789 (-3.70%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                  0 / 0 |
| Public `memcpy` calls / bytes        | 132 / 344,553 | 132 / 344,550 |                 0 / -3 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                  0 / 0 |

The exact instruction reduction is the primary CPU result, about 11
instructions per macro expansion. Cycles and elapsed time moved in the same
direction but remain supporting diagnostics under concurrent host load. The
hot classified-dispatch symbol shrank by 789 bytes; the uncommon exact-expand
wrapper is 1,312 bytes and is absent from the ordinary classified handoff.
Exact public-copy attribution reconciles both APIs with zero collision
overflow or probe-internal calls. No copy is attributed to the expansion
handoff.

## Evidence and validation

Ignored evidence is under `target/umber2-66p0.8.40.130/focused-gate/`.
Baseline and final binary SHA-256 values are
`125d974964247f2c44ca53fa050d75d9661b06e748db92beb41b36ccc735d108` and
`d2de2261f167de6f7b81814110785a2a2e6694b277f73d0592db07835237cb6e`.
Their `perf stat` receipts are
`6e518bc211cc69d1a24a29d3a0d7266614ce9e665d164c63befd516728b9b783` and
`ca69f6b1fd9fa1b4f9ddd8d39f5c8937fb2fab95514479e47b4d83ec14be0696`;
their symbolized copy reports are
`b481b97a1ad85b35dae8bedcfc999dafe3cc455208a75c4e5554aba220aa5898` and
`e48bdf014512293a9eaf6b34d29eb8160aa5b5dccc8d83351fe325608232ca00`.

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests.
`cargo test -q --tests -p tex-exec` passes 759 unit tests with two ignored, four
main-control integration tests, and 24 external boundary tests.
