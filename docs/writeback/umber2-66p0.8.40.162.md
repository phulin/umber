# `umber2-66p0.8.40.162`: inline small definition regions

## Allocation attribution

The `.157` authority run attributed 414,672 calls and 8,885,226,036 requested
bytes to `delivery_and_scan`. A size-filtered allocation backtrace and a
focused exact allocation trace resolved the recurrent 16,384-byte family to:

```text
scan_macro_definition
  -> scan_toks_buffers
  -> push_scan_toks_word
  -> DefinitionArena::push_replacement
  -> DefinitionRegion::push_word
  -> DefinitionRegionOwner::ensure_chunk
  -> DefinitionWordChunk::new
```

`DefinitionWordChunk` reserves 4,096 `TokenWord`s. Before this change, the
first word in every nonempty local semantic region acquired that full block,
even when a group defined only a one-word macro and retired immediately.
Those blocks were not scanner scratch and could not be cached globally: their
correct owner was the independently reclaimable definition region.

## Lifetime-aligned storage

`DefinitionRegionOwner` now carries eight words inline and allocates stable
4,096-word blocks only for the overflow suffix. The inline cells share the
region's existing `Rc` lifetime, rollback, local-region pin, and group
retirement rules. Large definitions retain the same flat stable overflow
directory; macro admission and resident delivery address the inline prefix or
overflow block directly without body materialization, relocation, or a
publication copy. Suspension state is unchanged.

The focused regression tests cover an exactly full inline definition, the
first overflow word, direct semantic reads, resident delivery across the new
inline/overflow boundary, cursor rollback, and later 4,096-word crossings.

## Exact focused evidence

The bounded source (SHA-256
`fef53065f1c4c9d35ddfb2db34d5ba2495a257ed8031ca66ca8c92aa531fb339`)
performs 100,000 one-word local definitions in independently retired groups.
Both binaries completed with empty, byte-identical stdout and exactly 400,004
delivery/scanning boundaries, 400,004 semantic-apply boundaries, 500,005
evidence boundaries, 100,001 `def` operations, 100,000 group entries, and
100,000 group exits.

| `delivery_and_scan` |      Baseline | Inline region |                    Delta |
| ------------------- | ------------: | ------------: | -----------------------: |
| Allocation calls    |       500,013 |       300,013 |       -200,000 (-40.00%) |
| Requested bytes     | 1,698,446,232 |    62,446,232 | -1,636,000,000 (-96.32%) |

The longer counter run repeats the same completed 100,000-group episode five
times. Three-run `perf stat` means fell from 18,995,861,475 to 16,852,492,312
user cycles (-11.28%) and from 25,883,914,852 to 25,371,343,263 user
instructions (-1.98%). Cycle standard deviations were 0.45% and 0.93%; the
instruction deviations were 0.01% in both binaries.

The checked public-copy probe reported identical recurrent work: 706,484
`memcpy` calls in each binary and 10 `memmove` calls / 11,791 bytes in each.
`memcpy` bytes were 69,427,489 baseline and 69,428,751 candidate, a 1,262-byte
whole-process difference with no additional call. Thus the removed 1.636 GB
allocation family was not replaced by recurrent public copying or moving.

The baseline and candidate profiling binaries have SHA-256
`45c625938ffc7c7e722ca0c63880d608657e4ad3a4697ec798e8dfc40716d4c3`
and `798d88d5b1e9ca471950b00d2c251022a80fe31693a0eda8d76e0870ef79986b`.
The copy interposer SHA-256 is
`3378f994509f85dac45d1f2c1c41453f3f447facf91a5319e3d2d15f2410b686`.
Ignored raw census, counter, copy, and symbolized evidence is under
`target/umber2-66p0.8.40.162-local-{base,candidate}/`.

## Validation

- `cargo test -q -p tex-state --lib definition_arena`: 43 passed.
- `cargo test -q --tests -p tex-command -p tex-exec`: command 392 + 23
  passed; executor 761 passed / 2 ignored plus 4 + 24 integration tests.
- `cargo test -q --tests -p tex-state`: 579 + 12 + 1 passed.
- `scripts/check.sh`: all four gates passed on the final tree.
