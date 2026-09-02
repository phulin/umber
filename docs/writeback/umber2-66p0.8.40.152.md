# `umber2-66p0.8.40.152`: cold expanded-delivery failures

## Adopted boundary

`expanded_delivery_entry` is now the one expanded destination loop. Ordinary
command, end, replay-completion, and alignment results leave that loop directly;
the deleted `expanded_delivery_loop` and `expanded_destination_loop` wrappers no
longer relay a successful result through `DeliveryFailed` and
`DeliveryErrorSlot`. The shared cold resident-transition helper returns a rich
error only on failure, and `fail_expanded_delivery` alone clears the provisional
command, restores the caller's expansion depth, invalidates freshness, and
publishes that error.

Continuation decoding moved to cold `resume_expanded_delivery`, reached only
when `expansion_resume` or its typed scanner wrapper proves a genuine parked
expansion. It restores the exact command, child destination, typed resume phase,
and delivery-local has-expanded bit. Raw/expanded observation, macro recovery,
provenance, replay completion, alignment interception, rollback, suspension,
and trace suppression retain their existing owners. No carrier, policy driver,
command-family path, second loop, or cache was added; the obsolete error types
and their size test were deleted.

## Focused profiling comparison

The exact baseline is the release/profiling executable built from assigned base
`0294a6d5a`; its SHA-256 is
`75c9ef598f1c5d26b37d03f6f3c7f1f919b4de5bfac5bed7a619211c7e1717ba`.
The candidate SHA-256 is
`35d2d92b349ec14320310e48fb4bcd25a1be0986e31e33906691cfd45bcee664`.
One matched `destination_owned_macro_expansion` pair ran 1,000,000 empty macro
expansions, one terminal expanded delivery, 1,000,001 frame steps, and zero
warmed allocations or requested bytes.

| Exact result                         |        Baseline |       Candidate |               Delta |
| ------------------------------------ | --------------: | --------------: | ------------------: |
| `expanded_delivery_entry` code size  |     4,069 bytes |     1,528 bytes |    -2,541 (-62.45%) |
| Complete executable `.text`          | 2,113,156 bytes | 2,109,912 bytes |     -3,244 (-0.15%) |
| User instructions                    |   1,144,409,797 |   1,143,409,139 | -1,000,658 (-0.09%) |
| User branches                        |     185,333,709 |     185,333,456 |                -253 |
| User cycles                          |     820,622,244 |     837,484,772 |         +16,862,528 |
| Warmed allocations / requested bytes |           0 / 0 |           0 / 0 |           unchanged |
| Public `memcpy` calls / bytes        | 109 / 4,332,477 | 109 / 4,332,478 |              0 / +1 |
| Public `memmove` calls / bytes       |           2 / 0 |           2 / 0 |           unchanged |

The deterministic improvement is approximately one fewer retired instruction
per expansion, a 62.45% smaller selected hot owner, and a 3,244-byte smaller
executable text section. The single-run cycle increase is recorded and is not
used as a speed claim. Both public-copy reports reconcile exactly with zero
collision probes, overflow, or probe-internal calls and attribute no copy to
expanded delivery.

The baseline/candidate counter receipt hashes are
`b7dcd3da7359c765bdd5a6765e3fb3267323c2f890f259993812ebf33bc4c3c7` and
`49477e5e9fef28f7744e78106df16ae534f9fbaf520848d7533ea595c571707f`;
the `perf stat` receipt hashes are
`7aa9cc7a45aaa6e347f81bf3273bb53ca3d41520dd47e1418179b38c92613eb9` and
`e574a9a9f0bc28797769f293a7e086967b745a3cb405c9701c40494cc527b822`;
the symbolized copy-report hashes are
`46c1e928297c46e64c7d32df7a524e18dee0542b726ab230255c514975a320e5` and
`87ffb37e10fe9447aab2d7e8b6cbadf5000da0c511ac51c8f8479a31d0d7f140`.
The checked interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored evidence is under `target/umber2-66p0.8.40.152/evidence/`.

## Validation and disposition

- `cargo test -q -p tex-command --tests`: 391 unit tests and 23 boundary tests
  pass.
- `scripts/check.sh`: all gates pass.

The measured target is removed without a distinct residual defect, so no
successor is filed.
