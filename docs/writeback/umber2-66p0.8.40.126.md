# `umber2-66p0.8.40.126`: give replay input one resident cursor

## Selection authority

The integrated `.125` authenticated 20,000,000-command capture is the sole
broad selection authority. It ran commit
`9163a68b9ae8754cd607b0efc5421c7dda5fcb30` with exact work vector
`(20000000, 19907047, 2216876, 6018541, 16781945, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Raw delivery comprised 463,197 source words, 11,520,843
stored/body words, 7,922,916 macro-argument words, and 91 synthetic end-v
commands. `advance_resident_command_into` led application self time at 13.87%.
The next application owners were `expand_into` at 2.86%, `raw_delivery_entry`
at 2.75%, and macro-argument construction at 1.88%.

The `.125` change removed the macro-body branch's duplicate command-side
cursor and profiling census. The same annotated resident function left one
other redundant warm coordinate: a replay-backed token advanced its physical
`ResidentReplayCursor`, then independently advanced and checked the generic
packed frame's logical position. Macro arguments already had one absolute
cursor after `.124`; durable and attempt rows need only their packed-frame
position. The replay branch was therefore the next distinct resident-input
owner. No second broad profile was needed.

The authority reconciles 6,514,305 public `memcpy` calls for 951,687,687 bytes
and 126,700 `memmove` calls for 21,931,622 bytes with zero overflow or
probe-internal calls. Its leading copy rows belong to excluded dense storage,
page/DVI work, and cold external resources. No copy row motivated this CPU
simplification.

## Architectural simplification

`ResidentReplayCursor` now owns the logical word position together with its
prefix/body run, physical segment, in-segment offset, cached segment end, and
remaining run length. One sequential load advances that single coordinate.
The replay row's common packed frame retains immutable input identity, active
source, behavior flags, and retirement metadata, but its position remains
unchanged during warm delivery.

First-touch history now stores one `ReplayCursor` inverse rather than a packed
position plus an optional physical cursor. Rollback swaps that exact cursor;
diagnostic validation, cold semantic projection, exhaustion, delivery stamps,
prefix-to-body transition, retry, rejection, and acceptance derive their
logical position from the same owner. Durable and attempt rows retain their
existing scalar packed-frame inverse. The e-TeX aftergroup prefix path still
extends only before first delivery and installs the store-minted cursor at
position zero.

This deletes a per-word position mutation and equality check without a cache,
threshold, alternate replay route, or second representation. Provenance,
suspension, retirement, active-source inheritance, rollback ordering, and
token semantics are unchanged.

## Focused before/after gate

The baseline binary is the exact `.125` accepted gate binary, SHA-256
`a950b8d39bf39ea21c0c6fa5797741d46752932d37c3cba42304403a982c6220`.
The final binary SHA-256 is
`15ac5434d13388063f63535e0d25ea12b360fec08a5d7598666d87e9706ca247`.
Each binary ran the production `fused_raw_expanded_delivery` row once under
`perf stat` and the checked public-copy interposer. The row delivered exactly
2,000,000 words across 666,667 replay, 666,666 attempt, and 666,667 durable
words. Both rows report 2,000,000 fuel charges, token-frame steps, and meaning
lookups, 1,000,000 expanded deliveries, zero intermediate relays, zero command
copies, and zero warmed allocations or requested bytes.

| Counter                              |         Baseline |            Final |               Delta |
| ------------------------------------ | ---------------: | ---------------: | ------------------: |
| User instructions                    |    1,150,649,936 |    1,148,647,704 | -2,002,232 (-0.17%) |
| User cycles                          |      477,025,100 |      483,550,874 | +6,525,774 (+1.37%) |
| Public `memcpy` calls / bytes        | 132 / 24,338,344 | 132 / 24,335,564 |          0 / -2,780 |
| Public `memmove` calls / bytes       |            2 / 0 |            2 / 0 |               0 / 0 |
| Warmed allocations / requested bytes |            0 / 0 |            0 / 0 |               0 / 0 |

The exact instruction reduction is three instructions per replay word plus
minor fixed process variation. Cycles moved oppositely under concurrent host
load, so no wall-time or cycle improvement is claimed. Public copy calls are
unchanged, neither binary performs a nonzero `memmove`, and both symbolized
reports reconcile with zero overflow, collision loss, or probe-internal calls.

The focused replay push/pop lifecycle row independently preserved one million
deliveries with zero warmed allocation and exact copy calls: `memcpy` remained
107 calls, `memmove` remained four zero-byte calls. Its instruction totals were
flat, as expected because row admission and retirement dominate its
single-token replay cycle; it is structural evidence rather than the CPU
claim.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests.
The sequential replay lifecycle test additionally proves the packed-frame
position remains zero across forward delivery, checkpoint rollback, candidate
redo, and acceptance while the resident cursor reaches positions 300 and 301.
The full format and clippy verdict is recorded by `scripts/check.sh`.

Ignored evidence is under `target/umber2-66p0.8.40.126/focused-gate/`. Baseline
and final counter receipt SHA-256 values are
`0621f4009eecf3b364ca7fb404cca49b9386bb5a48631d460c9f82406e12322e`
and `b889d4fe28901233d2fd6734b5eb44c4f5540331812525a7f5da4602471657de`.
Their symbolized copy reports are
`f53bcc38451b88f46d1edbf04d79e68971884e7e6f62da898697d07ee494cffa`
and `53eacd2df84b5b6f1922ed535515c1cc96bed2fb502667ad595221af5f6dcd11`.
The `.125` authority `perf.data`, copy report, self report, inclusive report,
and timing receipt retain SHA-256 values
`6ecead8912f92b7d425069973e5622713656b60b1dd6daacb22812484275b409`,
`0055b5677c662b594015d917e0311307f45de25229501549fbf78eb91f52e314`,
`037bfb607378e2ef7bfad643ba7befa6a1d6d9d48be6063cb9fe674c5f2a9812`,
`d5c23276e30fc2eca4421f1cc1f2a5187f010c62f7a1116bf9c55a87891643ae`,
and `f21437d708a971413a308def18fa06c80c885eb0fe25709f19f49a4530483887`.
