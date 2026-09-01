# `umber2-66p0.8.40.131`: reuse argument opening-brace classification

## Selection authority

The integrated `.127` authenticated 20,000,000-command capture remains the
sole broad authority. At exact work vector
`(20000000,19907047,2216876,6018541,16781945,4011)`,
`ExecutionScratch::append_argument_token` ranked at 2.24% application self
after `.127` removed duplicate delivery census writes. The `.128` through
`.130` resident, raw-delivery, and expansion-handoff simplifications were also
integrated before this work. No new corpus execution or broad profile was run.

Source and optimized-code inspection found one repeated spelling projection in
the accepted-token transition. `PendingArgumentFacts::settle` classified the
packed token as an opening brace to establish the removable-outer-group fact;
the caller then classified the same packed token as an opening brace again to
advance literal brace depth. That second projection represented neither TeX82
semantics nor a distinct ownership boundary.

## Architectural simplification

First-scan fact settlement now returns its already-computed opening-brace bit
to the same resident append transition. That bit advances brace depth; only a
non-opening token reaches the unchanged closing-brace classification. This
removes exactly one repeated token-spelling classification from every accepted
argument token without introducing a cache, threshold, or alternate matcher.

The classified token still writes its packed word and provenance once before
fact settlement. Paragraph legality, first-token outer-group candidacy,
delimited-prefix commitment, brace saturation, final outer-group stripping,
and publication retain their order and values. The writer's admitted frame,
slot, append coordinate, provenance-run boundary, delimiter rollback
coordinate, and final validation are unchanged. Resource suspension/resume,
backup, macro-stack retirement, candidate rollback/redo, and argument replay
therefore keep their existing owners.

## Focused before/after gate

The exact baseline was current main commit `c5b308baf` (tree
`4c1550feaa3c283151cc9a2226305c3df489abd7`) plus the identical focused gate.
The new `macro_argument_append` row warms the canonical matcher and its
fixed-chunk lane, retires that frame, then scans one 1,000,002-token braced
argument. Both binaries report exactly 1,000,002 accepted tokens, 1,000,004 raw
frame steps, and zero warmed allocations or requested bytes.

| Counter                              |      Baseline |         Final |               Delta |
| ------------------------------------ | ------------: | ------------: | ------------------: |
| User instructions                    | 1,209,927,794 | 1,203,927,662 | -6,000,132 (-0.50%) |
| User cycles                          |   448,565,167 |   441,197,232 | -7,367,935 (-1.64%) |
| Internal elapsed nanoseconds         |    76,998,408 |    75,828,470 | -1,169,938 (-1.52%) |
| Nanoseconds per accepted token       |         77.00 |         75.83 |      -1.17 (-1.52%) |
| Hot append function bytes            |         1,022 |         1,011 |        -11 (-1.08%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |               0 / 0 |
| Public `memcpy` calls / bytes        | 108 / 330,144 | 108 / 330,134 |             0 / -10 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |               0 / 0 |

The exact instruction reduction is the primary CPU result: almost exactly six
instructions per accepted token. Cycles and elapsed time moved in the same
direction but remain supporting diagnostics under concurrent host load. The
ten-byte process-total `memcpy` difference is startup layout noise; call counts
are unchanged and neither report attributes a public copy to argument append.
Both reports have zero overflow and probe-internal calls.

## Evidence and validation

Ignored evidence is under
`target/umber2-66p0.8.40.131/focused-gate/`. Baseline and final binary SHA-256
values are
`b7d7b37030db82299a8d63bb17330a99e18bd1d88c76bfd6f5607a68ed034810`
and
`6ee40767bd645a71edab524ae552140f18a267d4843fb8077cc275cdc7b2b740`.
Their counter receipts are
`bffe61714d815c1f6fa732c0acf0f1eddb03642a5d09a923f251e8953bc42531`
and
`05f11a9a0e9764b3f5785719d0cd6a59d041687c6f972e5744afa7ada3ac9440`;
their `perf stat` receipts are
`aa528aeb241bc39432cc892ac69436abc6bf03f10e3d29aef09980839ead8c34`
and
`25eecb06b87598199e39e689a540231ad04cd0ae7fb3861b5e43d88b627364cc`;
their symbolized public-copy reports are
`7bd2a641a29c4a18dcc7ead4a1da9ac85d0da87502e8ac9c343b1ba176e14770`
and
`5bcbe9dd9670e41df40ad7c18f3bc398e28b099b0a1af9c688ce76e58e636c4d`.

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests.
`cargo test -q --tests -p tex-exec` passes 759 unit tests with two ignored,
four main-control tests, and 24 external boundary tests. `scripts/check.sh`
passes all four gates: dprint, biome, rustfmt, and both clippy resolutions.
