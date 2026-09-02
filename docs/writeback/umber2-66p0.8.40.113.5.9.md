# `umber2-66p0.8.40.113.5.9`: page/output node high water

## Decision

Accepted. The remaining peak was sparse physical packing, exposed during the
page/output ownership overlap. At the largest automatic box-255 boundary in
the diagnostic row, 2,671 live node blocks held only 13,610 records: 5,456,598
record slots were stranded, every block was partial, and no block held more
than one logical list chunk. The 5,198-block event high water was therefore not
real node volume or accepted/candidate physical sharing.

Production node lists now use 16-record logical chunks packed into exact
64-KiB physical superblocks. Logical coordinates and incarnations remain
stable; packing performs no compaction, relocation, scan, or payload copy.
Automatic box 255 is also a journaled page-region root rather than a recursive
page-to-durable copy. Taking it for default or explicit output is a coordinate
move. Explicit `\copy255` and reassignment retain their normal on-demand copy
or promotion semantics. The fifth page payload root participates in operation
rollback, checkpoints, state identity, successor validation, sealed sharing,
unique adoption, and structural-copy fallback.

## Authenticated 50-million-command row

The final locked arXiv `2606.12566` row used schema-12 format, the ordered
123-key offline closure, source epoch `1787080434`, distribution aHash64
`df66c327ae636145`, the checked copy probe, 50,000,000 command fuel,
100,000,000 executor steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected
status 1 preserved the exact semantic vector
`(50000000,49903532,9457781,15936698,35326903,4203)` with empty stdout and no
PDF artifact.

Peak node backing fell from 5,198 blocks / 340,656,128 bytes to 68 /
4,456,448 bytes (98.69%). Annex peak stayed 80 blocks / 5,242,880 bytes. RSS
fell from 495,716 to 168,732 KiB (65.96%), below the historical 285,464-KiB
row. The largest sampled output boundary held 12,071 records in 22 node
blocks; all 22 physically packed multiple logical chunks, with 32,985 tail
slots stranded, and all 22 belonged to the output page region. Eight of 74
annex blocks belonged to durable/other owners. Across 873 installations there
were 872 zero-copy takes and one source-requested on-demand promotion.

Named allocation traffic fell from 3,359,974 calls / 26,861,467,230 bytes to
3,346,013 / 26,531,582,629. Joint public copy fell from 9,589,922 calls /
1,472,615,235 bytes to 9,489,668 / 1,453,497,316; memcpy, memmove, allocation,
and scan traffic therefore did not absorb the memory reduction.

## Correctness and evidence

Focused `tex-state`, `tex-exec`, `tex-incr`, and `tex-out` suites pass,
including automatic-output zero-copy ownership, checkpoint/page succession,
resource-retried output, and stale-coordinate coverage. Committed PDF parity
and the math/alignment DVI fixtures pass. The `umber-wasm` wasm32 build passes.
The page/output census is compiled only with `profiling`; normal builds retain
no census or scan. `scripts/check.sh` passed all gates.

Ignored final evidence is under
`target/umber2-66p0.8.40.113.5.9/evidence-final/`. The profiling binary,
stderr, timing, and raw copy-report SHA-256 values are respectively
`9b50a01f731c96d17508926128aeec31e0f7178c8be85a72e1eec67917d34e43`,
`8a29fb20942fa6324c1875f9fbfada842896ff6869937f1970fb4c6e4712a3d4`,
`83cb9d268c71bd1fce202667d8e440b90bafc03946b68790901f62f99773dd1b`,
and `941fc8680c30640cfe3e8a34a1ad049e01278c03618ab64c3780bff4b837ba5c`.
