# `umber2-66p0.8.40.141`: one admitted macro-argument brace delta

## Selection and architecture

The integrated `.140` authenticated profile is the broad authority. At its
exact 20,000,000-command work vector,
`ExecutionScratch::append_argument_token` retained 1.56% disjoint self and a
separately sampled 0.51% closing-brace projection. Closed `.131` had removed
the repeated _opening_-brace projection, but its accepted-token transition
still called `ClassifiedToken::spelling_is_end_group` after
`PendingArgumentFacts::settle` returned only an opening boolean.

`PendingArgumentFacts::settle` now decodes the literal catcode once and returns
one one-byte `ArgumentBraceDelta`: `Open`, `Close`, or `Neither`. The same
settlement uses `Open` for the first-token outer-group fact, and
`append_argument_token` consumes the returned delta directly for saturating
brace-depth adjustment. The prior opening boolean and the second
closing-spelling projection are gone. No fact cache, alternate matcher, heap
owner, or second token representation was added.

The writer still owns one admitted append position, provenance boundary,
delimiter rollback coordinate, depth, and first-scan aggregate. Append order,
prefix holdback and overlap commitment, final outer-pair trimming, publication,
backup, suspension, rollback, and LIFO retirement therefore retain their
existing ownership and transition order.

## Focused before and after

The exact baseline is integrated commit `316891bca`; the candidate differs
only by this implementation and its tests/docs. Both optimized binaries use
the existing `macro_argument_append` row: one warm invocation followed by one
measured 1,000,002-token braced argument. Each reports exactly 1,000,002
accepted tokens, 1,000,004 raw frame steps, and zero warmed allocations or
requested bytes.

One outer `/tmp/umber-perf-host.lock` covered the paired baseline and candidate
`cycles:u,instructions:u` rows and both exact public-copy censuses.
Instructions are the primary deterministic CPU evidence. Cycles fell in the
same paired window; internal elapsed time rose by 0.68 ms and remains a noisy
supporting diagnostic rather than a contradictory work count.

| Counter                              |      Baseline |         Final |                Delta |
| ------------------------------------ | ------------: | ------------: | -------------------: |
| User instructions                    | 1,195,932,292 | 1,183,932,655 | -11,999,637 (-1.00%) |
| User cycles                          |   451,573,719 |   447,397,823 |  -4,175,896 (-0.92%) |
| Internal elapsed nanoseconds         |    67,678,296 |    68,359,566 |    +681,270 (+1.01%) |
| Nanoseconds per accepted token       |         67.68 |         68.36 |       +0.68 (+1.00%) |
| Hot append function bytes            |         1,011 |           996 |         -15 (-1.48%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                0 / 0 |
| Public `memcpy` calls / bytes        | 118 / 338,753 | 118 / 338,750 |               0 / -3 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                0 / 0 |

The three-byte process-total `memcpy` difference is startup layout noise; call
counts are identical and neither report attributes a public copy to argument
append. Both APIs report zero overflow and probe-internal calls.

## Projection proof and semantic coverage

Symbolized disassembly of the baseline append symbol contains one call to
`PendingArgumentFacts::settle` followed by one call to
`ClassifiedToken::spelling_is_end_group`. The final append symbol contains the
settlement call and zero `spelling_is_end_group` references. Final settlement
itself has one literal-catcode decision tree that returns `-1`, `0`, or `1`;
no second spelling projection exists in the append path.

The focused scratch test now checks the exact `1, 2, 2, 1, 0` depth sequence
for opening, nested opening, neutral, nested closing, and outer closing tokens,
then checks first-scan facts and stripped-outer publication without word
rereads. Canonical macro tests retain literal delimiter termination,
overlapping-prefix recovery, delimiters beneath nested braces, stripped-outer
trace/observation bounds, and paragraph recovery. The paragraph test now also
checks the backed-up forbidden token, and a new extra-right-brace test checks
the inserted paragraph remains ahead of the backed closer. Attempt rollback
and suspension tests now carry an actual braced argument and verify its facts
remain live only through the intended frame lifetime.

## Evidence and validation

Ignored evidence is under
`target/umber2-66p0.8.40.141/focused-gate/`. Baseline and final binary SHA-256
values are
`ba8accf98a569955130f6aef193cabc87184398c940840d9b5742355a2d9598f`
and
`6877193e2d8656cc0e87f8adfe13a368e073b96da960de48a4ddaa02dac07735`;
their build IDs are `4e77362d77fbf2c37f24a20dc8ec8a7cf9b606b8` and
`38501690187048ff7b1c3bd61de1789afd990ede`. The checked public-copy
interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Baseline/final counter receipts are
`5fdc22822314f258960e0924b3427343324d99df28144be9c900ec40f5b27483`
and
`4aadf69376b4aebb8993c5cd344559fdf8d07322a5dfc827fcbe833f89560cdf`;
performance receipts are
`1a244c0bec64038e3785ecc5226a36776254c62c1fcbc4b0520ac38ea19da1cc`
and
`b229d9a5f166026179d31a1a1abef5959d7d590494b44453c526444a9471ef52`;
symbolized public-copy receipts are
`107e0c7715d413012891a76ecf8adc6848582ad0281787049c47c8863efdb5e1`
and
`1da0827887a71b50ff119165ea6c8971e15648a3db778f09d0822cd6d86d9cbf`;
and append disassembly receipts are
`aee655cad6a89624359ad0dc862d568d5d17005f80ab1cd8386a330cf0028f86`
and
`f19b1bdd8191d0aa58201fa47516762fb2662e71f98062f470ae03fa47aec0ab`.

`cargo test -q --tests -p tex-command` passes 388 unit and 23 boundary tests.
`cargo test -q --tests -p tex-exec` passes 760 unit tests with two ignored,
four main-control tests, and 24 external boundary tests. `scripts/check.sh`
passes all four gates: dprint, Biome, rustfmt, and both clippy resolutions.
