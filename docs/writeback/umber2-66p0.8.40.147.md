# `umber2-66p0.8.40.147`: cold raw-delivery failures

## Adopted boundary

`raw_delivery_entry` is now the one raw destination loop. Its ordinary command
and end branches write the final `Result<DeliveryStatus, CommandError>` directly
into the caller's return slot. The entry no longer creates or passes a
`DeliveryErrorSlot`, returns a zero-sized `DeliveryFailed`, or wraps a second
raw loop.

The shared recovery and retirement implementation retains its existing cold
error-slot protocol for expanded delivery. Raw delivery enters that protocol
only through `settle_raw_resident_cold_transition`, which is cold and
non-inlined. `fail_raw_delivery` is likewise cold and is the sole raw owner of
failure cleanup: it drops any provisional command, invalidates freshness, and
returns the already-constructed rich error. Provenance, outer recovery,
alignment interception, replay completion, EOF restart, fuel exhaustion, and
suspension ownership are unchanged. No carrier, second loop, dynamic dispatch,
cache, or command-family branch was added.

The command-core architecture test now requires exactly one raw entry, rejects
the deleted `raw_destination_loop`, and proves that the ordinary entry body
does not name `DeliveryErrorSlot` or `DeliveryFailed`.

## Focused mixed-delivery evidence

The baseline is an issue-private build of exact base `0cfffb481`; the candidate
uses the same release/profiling manifest, Rust 1.93.0 toolchain, and dependency
resolution. Both run `mixed_macro_resident_pipeline`: 2,000,000 macro-body
transitions, 1,000,000 parameter deliveries, 1,000,004 replay words, 2,000,004
raw frame steps, 1,000,000 expanded deliveries, 1,000,001 macro expansions,
zero suspension moves, zero command copies, and zero warmed allocations or
requested bytes.

| Exact result                         |        Baseline |       Candidate |           Delta |
| ------------------------------------ | --------------: | --------------: | --------------: |
| `raw_delivery_entry`                 |     1,312 bytes |       668 bytes |  -644 (-49.09%) |
| Complete executable text             | 2,120,285 bytes | 2,117,180 bytes | -3,105 (-0.15%) |
| Raw-entry stack reservation          |       568 bytes |       456 bytes |  -112 (-19.72%) |
| User instructions                    |   2,370,771,701 |   2,370,772,433 |            +732 |
| User branches                        |     389,199,240 |     389,199,696 |            +456 |
| User cycles                          |   1,028,733,697 |   1,730,434,151 |    +701,700,454 |
| Warmed allocations / requested bytes |           0 / 0 |           0 / 0 |       unchanged |
| Public `memcpy` calls / bytes        |   136 / 347,050 |   136 / 347,051 |          0 / +1 |
| Public `memmove` calls / bytes       |           2 / 0 |           2 / 0 |       unchanged |

The accepted improvement is code footprint: the hot raw entry is 49.09%
smaller and the complete executable text is 3,105 bytes smaller. Retired
instructions are unchanged to measurement noise and cycles regressed in this
paired run, so neither is used as a speed claim. The copy reports reconcile
both APIs with zero collisions, overflow, or probe-internal calls and attribute
no copy to raw delivery.

Baseline and candidate binary SHA-256 values are respectively
`41fe203fffb8174163a3f07ca237cbcf4fc2a6b280b8d8d7f018e9844b1310b4` and
`a0612fe20d429d137c41201e352722aea411c498fdc5ddb8d02844ca7ea98de0`.
Their `perf stat` receipt hashes are
`7f373151eb9868479578ce1205b7e8ede7f434e60bb03c5a691ac2b39997e1d5` and
`a0e5d1006540043151431f531fda596992f07bb731fd9403e346219930de01c0`;
their symbolized public-copy report hashes are
`eba2d55f8301f900613c46663dc433796db3ffd0b2bf28a2dca01620c41c2672` and
`de93753554f244f20055946e9f08bffaea730864dce4e3df5aa641023fbe8d31`.
The checked interposer hash is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored issue-private evidence is under
`target/umber2-66p0.8.40.147/focused-gate/`.

## Validation and disposition

- `cargo test -q --tests -p tex-command`: passed.
- `cargo test -q --tests -p tex-command --features profiling`: passed.
- `scripts/check.sh`: passed.

The measured target is removed without a distinct residual defect, so no
successor is filed.
