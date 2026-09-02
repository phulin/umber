# `umber2-klu1`: release vacant node-pool superblocks

## Decision

The lifetime-aligned reclamation is complete, but the issue's production RSS
acceptance remains **rejected**. A `NodePool` now drops an exact 64-KiB node or
annex payload when the physical block loses its final logical owner. It keeps
only stable slot/incarnation metadata, allocates a new payload before reusing
that slot, advances the physical incarnation, and continues to reject stale
logical and physical coordinates. The authenticated row proves that vacant
payload backing is exactly zero, without a scan, census-driven decision,
second representation, arbitrary cache bound, or per-node allocation.

Peak RSS nevertheless remains 651,640 KiB versus the historical 285,464 KiB:
366,176 KiB or 128.27% higher. The new census shows that the process reached
536,412,160 node bytes plus 8,650,752 annex bytes while those blocks were
simultaneously live. Releasing them after their last owner therefore cannot
bring the process peak into an acceptable historical range. This issue stays
open; the evidence narrows its unmet criterion from vacant retention to live
node-region ownership.

## Ownership and reclamation

The old allocation path already popped `free_blocks` in LIFO order and
incremented the selected block's incarnation before reuse. It appended a block
only when no vacant slot existed. The list still grew because the rising
simultaneously live/retained page-region set, including current/accepted and
candidate overlap, temporarily left no slots vacant. Later retirement pushed
those slots onto `free_blocks`, but `release_lineage` only truncated their
initialized prefixes: every 64-KiB allocation remained inside the block
vector. Reuse limited future vector growth; it could not lower resident memory
or the old `blocks.len() * 65,536` accounting.

`ChunkStorage` now separates stable block-table slots from optional backing.
The generic fork-arena policy remains warmed, while the node-pool policy drops
backing on final-owner release and recreates exactly one superblock on reuse.
An O(1) backed-block scalar makes heap accounting exact. Focused optional-node
and packed-annex tests prove last-owner return, stable slot reuse, incarnation
advance, and stale-key rejection. Existing page-material controls now charge
only the exact reallocated superblocks after rollback.

## Authenticated 50-million-command row

Exactly one engine-entering row used the profiling binary SHA-256
`502322fe8e9208ac967bef4cc8776854e1c12d3a610df15b652ceef10dec6529`
with ELF build ID `86d97784a719807523d5faaf7fe2944be9b82713` and checked
public-copy probe SHA-256
`3378f994509f85dac45d1f2c1c41453f3f447facf91a5319e3d2d15f2410b686`.
It used arXiv `2606.12566` `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
source epoch `1787080434`, and distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`.

The guards remained 50,000,000 canonical-command fuel, 100,000,000 executor
steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected status 1 occurred at
the exact accepted semantic vector
`(50000000,49903532,9457781,15936698,35326903,4203)`. Standard output was
empty and no PDF was published. User time was 45.55 seconds, system time 4.10
seconds, and wall time 41.75 seconds. Peak RSS fell 41,192 KiB or 5.95% from
the 692,832-KiB predecessor, but remained 366,176 KiB or 128.27% above the
historical row.

| Lane  | Fresh |   Reuse | Releases | Peak live blocks / bytes | Peak vacant slots / backed bytes | Final live/vacant backing |
| ----- | ----: | ------: | -------: | -----------------------: | -------------------------------: | ------------------------: |
| Node  | 8,185 | 509,496 |  512,668 |      8,185 / 536,412,160 |                        4,427 / 0 |                     0 / 0 |
| Annex |   132 |  36,911 |   36,945 |          132 / 8,650,752 |                           65 / 0 |                     0 / 0 |

Named allocations were 3,862,102 calls / 59,769,165,606 requested bytes:
delivery/scan 414,684 / 8,885,236,724; semantic apply 3,264,009 /
33,839,697,403; evidence publication 3,670 / 1,569,635; cold materialization
179,074 / 17,040,993,124; and attempt scratch 665 / 1,668,720. Interpreter
construction, interpreter borrow, generation boundary, and arena growth were
zero. Relative to the predecessor this is 498,972 more calls and
32,700,629,000 more requested bytes, the expected cost of returning and later
reallocating each exact payload rather than caching it without an owner.

| API       | Predecessor calls / bytes |     Current calls / bytes | Change calls / bytes |
| --------- | ------------------------: | ------------------------: | -------------------: |
| `memcpy`  | 9,585,394 / 1,476,261,497 | 9,585,391 / 1,476,243,397 |         -3 / -18,100 |
| `memmove` |        13,974 / 2,671,470 |        13,974 / 2,671,470 |                0 / 0 |
| Joint     | 9,599,368 / 1,478,932,967 | 9,599,365 / 1,478,914,867 |         -3 / -18,100 |

Complete symbolization still contains none of the rejected
`span_chunk_node`, `append_reencoded_chunk_range`, or `copy_list_recursive`
carrier families. The preserved Wasm workload remains `stable` at 5,308,416
bytes before construction, 5,701,632 after construction, and 629,080,064
after compilation and disposal: 623,771,648 bytes or 9,518 pages of growth.
Its evidence SHA-256 remains
`e9c651469dff3a7d4ea718df9a92c150a2fd9e22f7a469d3e603781f979dfe16`.

## Evidence and gates

Focused node and annex release/reuse tests, the affected page-material
allocation controls, and the full `tex-state` native test set pass. Profiling
feature tests also pass. The repository format and lint result is recorded by
the final `scripts/check.sh` run.

Ignored evidence is under `target/umber2-klu1/evidence/`. Raw copy data,
complete symbolization, timing, engine stderr, and the profiling build log have
SHA-256 values `89b6d5498246731712a6ac12c147c65d9aa0cce20d3127a7e96a5e451289e10b`,
`054334ec89a19d38dbad2e2dbd2d44e81a17c720e15195abe1eab7162660b350`,
`742e9b8c8495ef3b5882a97f0760b7bf6aba9cc95e1b4c992005039696f2f53f`,
`f0befcae601a1f4746d3f4465274b4b2b845e0f527bec9bc67e0e53bd2bf67bf`,
and `996387052b088d7217a9c9ba6e09b8738d06ffeb6b3fc65f7a460c3ca43eecfb`.
