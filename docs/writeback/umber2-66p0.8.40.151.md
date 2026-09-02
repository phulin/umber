# `umber2-66p0.8.40.151`: delivery-local expansion state

## Adopted ownership

An expanded-delivery invocation now owns one stack-local `delivery_expanded`
bit. Classification sets it immediately before the selected TeX82 §366
expansion dispatch. The bit alone decides whether TeX82 alignment lookahead
defers its terminal expanded-delivery observation. An actual immutable-resource
suspension moves the bit into that invocation's `PendingExpansion`; resumption
restores it, and completion drops it.

There is no job-global expansion count. The `ExpansionState` wrapper,
`cumulative_expansions` command root, scalar journal slot and undo variant,
snapshot rollback swap, dependency-projection hash input, detached-summary
field, and their tests are deleted. `CommandProfile` is now a direct immutable
command root. Expanded `scan_toks` direct splices likewise perform no obsolete
counter update. Fuel charges and profiling work counters remain in their
independent monotonic ledger; trace suppression, diagnostics, provenance,
child-continuation ownership, and rollback retain their existing owners.

## Focused profiling comparison

The exact baseline is the `.150` release/profiling executable from base commit
`2d6a5310d21d56522c3d8aa6106ad301fcb750bb`; its SHA-256 is
`fd3ddb969fc9363cfec5fdffff5faf9eecb3011c46edecc0eeed7e34d867358d`.
The candidate SHA-256 is
`0b882fc8b32b2da4540ab62ad66ed38055924df4b7d51e996d6586d0fd0206a7`.
One matched `destination_owned_macro_expansion` pair ran 1,000,000 empty macro
expansions, one terminal expanded delivery, 1,000,001 frame steps, and zero
warmed allocations or requested bytes.

| Exact result                         |        Baseline |       Candidate |                  Delta |
| ------------------------------------ | --------------: | --------------: | ---------------------: |
| `expand_classified_into` code size   |    20,630 bytes |    20,441 bytes |          -189 (-0.92%) |
| Complete executable `.text`          | 1,638,992 bytes | 1,639,344 bytes |          +352 (+0.02%) |
| User instructions                    |   1,153,440,227 |   1,145,439,861 |    -8,000,366 (-0.69%) |
| User branches                        |     186,600,194 |     185,599,997 |    -1,000,197 (-0.54%) |
| User cycles                          |     799,666,148 |     537,258,433 | -262,407,715 (-32.81%) |
| Internal nanoseconds per expansion   |          309.58 |          203.17 |      -106.41 (-34.37%) |
| Warmed allocations / requested bytes |           0 / 0 |           0 / 0 |              unchanged |
| Public `memcpy` calls / bytes        | 109 / 4,332,493 | 109 / 4,332,469 |                0 / -24 |
| Public `memmove` calls / bytes       |           2 / 0 |           2 / 0 |              unchanged |

The deterministic result is eight fewer retired instructions per expansion,
one fewer branch per expansion, a 189-byte smaller selected hot owner, and
unchanged zero-allocation/copy topology. The single-run cycle and elapsed
reductions are recorded but are not used as a speed claim. Both public-copy
reports reconcile exactly with zero collision probes, overflow, or
probe-internal calls.

The baseline/candidate counter receipt hashes are
`8c131ed9302418eb6705a694ddef6a684b10539361efd17bf517697542d18196` and
`935e8a1388e6c24bf1647ad0e7447b21a26b77845134909cea56077b5cd08591`;
the symbolized copy-report hashes are
`98a102776f3e6a58db54c5a0a847e61723872c2862159b3d75ac308dec8f4310`
and
`f5f3042bd4fce95b8c61ad1c9d190f4428803b25955670ee32c2111538de7c41`.
The checked interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored evidence is under `target/umber2-66p0.8.40.151/evidence/`.

## Validation and disposition

- The focused suspension/deferred-observation and exact linear-work profiling
  tests pass.
- `cargo test -q -p tex-command --tests`: 392 unit tests and 23 boundary tests
  pass.
- `scripts/check.sh`: all four gates passed; both Clippy resolutions are clean
  across 32 workspace members.

The measured counter and journal remainder is deleted without a distinct
measured successor, so no follow-up issue is filed.
