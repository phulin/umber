# `umber2-66p0.8.40.159`: direct annex-span destination construction

## Ownership diagnosis

The authenticated `.157` copy census's leading bin was 373,119 calls and
71,638,848 bytes at
`Vec::SpecFromIterNested::from_iter`. The preserved build-ID binary resolves
all five static callers through `NodeAnnexView::detach_span`. The concrete
collection is the owned `Vec<u32>` returned by that boundary; the copied value
is its 192-byte `Skip<ArenaListIter<u32>>` source iterator. Inlined callers
include compact-node decode, whatsit decode, semantic hashing, and
cross-region re-encoding. This ancestry is independent of the
`LegalBreakpoints` whole-node copies assigned to `.158`.

The annex span itself still has to become owned when decoding an owned node or
moving its payload across region lifetimes. The transient iterator does not.
`detach_span` now allocates its final destination at the exact borrowed-view
length and pushes each borrowed word directly into that destination. It does
not retain a cache, a second representation, or a special execution path.

## Exact copy and allocation effect

The after row used the same 50,000,000 canonical-command workload, checked
copy interposer, schema-12 format, distribution, ordered prefetch closure,
source epoch, and arXiv `2606.12566` source as `.157`. It stopped at the same
status-1 semantic vector
`(50000000,49903532,9457781,15936698,35326903,4203)` and the same raw-delivery
subtotals `463672/30199338/19240431/91`.

| API       |      `.157` calls / bytes |     Current calls / bytes |    Delta calls / bytes |
| --------- | ------------------------: | ------------------------: | ---------------------: |
| `memcpy`  | 9,203,569 / 1,245,500,251 | 8,763,523 / 1,153,623,187 | -440,046 / -91,877,064 |
| `memmove` |        21,610 / 2,827,518 |        21,482 / 2,799,870 |         -128 / -27,648 |
| Joint     | 9,225,179 / 1,248,327,769 | 8,785,005 / 1,156,423,057 | -440,174 / -91,904,712 |

The former rank-1 `Vec` iterator-copy bin is absent from the current top 40;
rank 1 is now the pre-existing 20,491,902-byte node-range option projection.
Named allocation calls are exactly unchanged at 3,346,017. Requested bytes
fell from 26,510,056,981 to 26,509,883,709 because exact destination capacity
also removes `Vec`'s minimum-capacity over-allocation for short annex spans.
No allocation or public-copy bin replaces the removed work.

## CPU observation

The target's sampled self work was 0.27% before, split between copied-iterator
`next` (0.18%) and generic `Vec::from_iter` (0.09%). After destination
construction it is 0.27% directly in `detach_span`; the focused profile
therefore shows no sampled CPU change at its resolution while making the
ownership and copy deletion exact. Whole-run user instructions fell from
71,563,607,021 to 70,924,022,639 (-0.89%). User cycles rose from
40,323,285,089 to 55,395,663,813 while IPC fell from 1.77 to 1.28 under normal
concurrent host activity, so the unmatched whole-run cycle increase is not
treated as a causal latency result.

## Evidence

Ignored evidence is under `target/umber2-66p0.8.40.159/evidence/`. SHA-256
values for `perf.data`, raw copy data, symbolized copy report, counter receipt,
engine stderr, outer timing, and the checked interposer are respectively
`8beda887337062cf43dadd2918d85f34d53fe7ed4ce4aba2f23150f5d0a7787a`,
`8bf30e3e1f3ab4af4ce500cf00e30413e274c1662b3fad04465eb254c994487e`,
`48b86d47ce99bab91a05158efe50a2cb1dff9c8a2347f71eb2c654b7b01aa58a`,
`21c361411a4efa0b92f75211c0aca4baa8aa3102cf262d205f3dadb846e26aa8`,
`abe19ce5781bec38882b37a94e86630c9349fdf5060c501b8ab763b66b60ddd6`,
`e337d4cd85c5e7a44ac0d5eafbbf3716aaf84ccdafd0aae37164fd13e2e019ca`,
and
`3378f994509f85dac45d1f2c1c41453f3f447facf91a5319e3d2d15f2410b686`.
