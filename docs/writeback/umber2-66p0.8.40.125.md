# `umber2-66p0.8.40.125`: give the definition store the macro-body cursor

## Exact current-main authority

Exactly one authenticated execution entered the engine on commit
`9163a68b9ae8754cd607b0efc5421c7dda5fcb30` (tree
`4ec58afaa240f38d78e183d5f0c5b782455357c1`). The optimized profiling binary
SHA-256 was
`6ebcd949e128a38c540b40b64c2ae252b8af479ade98bee29b0068ad18b385e6`;
the checked public-copy interposer was
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture combined 199 Hz `cycles:u` DWARF callchains with exact public
`memcpy` and `memmove` attribution, an 8 MiB ring, and no CPU hold, affinity,
serialization, cache purge, control workload, fuel ladder, or second arXiv
execution.

The finite workload remained arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
offline distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source date epoch `1787080434`. Guards were 20,000,000 canonical command
fuel and 40,000,000 committed executor steps. Expected status 1 occurred at
vector `(20000000, 19907047, 2216876, 6018541, 16781945, 4011)` for fuel
charges, token-frame steps, expanded deliveries, meaning lookups, scanner
tokens, and write expansions. Raw deliveries were source `463197`,
stored/body `11520843`, macro argument `7922916`, and synthetic end-v `91`.

The wrapper reported 8.11 s wall, 6.29 s user, 0.57 s system, and 225,356 KiB
peak RSS. The capture contains 1,290 samples, zero lost samples, and
14,680,250,058 weighted cycles.

## CPU and copy selection

`advance_resident_command_into` led application self time at 13.87%. The next
application owners were `expand_into` at 2.86%, `raw_delivery_entry` at 2.75%,
`ExecutionScratch::append_argument_token` at 1.88%, and
`CommandFuel::record_raw_delivery` at 1.50%. The 1.49% payload-reservation,
1.23% clone-path `Option`, and 0.91%/0.90% arena cursor/root rows belong to
excluded dense storage. Page, shipout, DVI, and the Type1 fixture also remain
excluded.

Source annotation of the leading resident function isolated a redundant
nonsemantic transition in its macro-body branch. Every body word updated the
profiling-only thread-local read census, with the census store alone carrying
4.13% of the annotated function. The same branch kept replacement bounds and a
relative position in a generic command-side `ResidentSpanCursor`, converted
that position to the definition store's absolute coordinate, then independently
advanced and checked the duplicate cursor after the load. These operations
changed no token, provenance, source, suspension, rollback, or execution fact.

Exact public-copy attribution reconciles 6,514,305 `memcpy` calls for
951,687,687 bytes and 126,700 `memmove` calls for 21,931,622 bytes, with zero
overflow or probe-internal calls. `ChunkStorage::release_lineage` leads
`memcpy` at 2,333,768 calls and 392,073,024 bytes and is excluded dense
storage. `PageMaterialArena::push_active_list` leads `memmove` and is excluded
page/DVI work. No command-delivery copy row motivated this CPU simplification.

## Architectural simplification

The store-minted `ResidentMacroBody` now owns the sole absolute replacement
cursor together with its immutable start and end. One sequential operation
checks the end, reads the stable chunk slot, advances the absolute cursor, and
returns the relative semantic position with the packed word. The command row
retains only input identity, active source, invocation/name, and optional
arguments; the generic `ResidentSpanCursor` and its duplicate bounds,
relative-to-absolute conversion, advance, and equality check are gone.

First-touch history captures an opaque four-byte `ResidentMacroBodyCursor`.
Rollback swaps that exact absolute coordinate into the same store owner, so
checkpoint rejection, retry, accepted-history release, and suspension preserve
the prior delivery position without exposing storage coordinates. Relative
command stamps, active source/provenance, parameter replay, local-region
ownership, and macro retirement are unchanged.

The exact per-word macro-body census is now test/testing-only. The standalone
profiling gate drives the production sequential cursor and derives its known
read and chunk-crossing volumes from the fixture shape at the measurement
boundary. This removes the selected profiling state transition without adding
a threshold, cache, alternate delivery route, or special-case fast path.

## Focused before/after gate

The exact baseline binary was current-main plus integrated `.124`, before this
change. Both binaries ran the production `mixed_macro_resident_pipeline` once
under `perf stat` and the public-copy interposer. Both report the exact vector
`macro_body=2000000`, `parameters=1000000`, `replay=1000004`,
`raw=2000004`, `expanded=1000000`, `macro_expansions=1000001`,
`suspension_in=0`, `suspension_out=0`, `command_copies=0`, with zero warmed
allocations and requested bytes.

| Counter                              |      Baseline |         Final |                Delta |
| ------------------------------------ | ------------: | ------------: | -------------------: |
| User cycles                          |   972,434,502 |   969,806,165 |  -2,628,337 (-0.27%) |
| User instructions                    | 2,541,760,574 | 2,493,759,239 | -48,001,335 (-1.89%) |
| Internal elapsed nanoseconds         |   363,672,309 |   388,597,991 | +24,925,682 (+6.85%) |
| Nanoseconds per macro-body word      |        181.84 |        194.30 |      +12.46 (+6.85%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                0 / 0 |
| Public `memcpy` calls / bytes        | 130 / 346,948 | 130 / 346,949 |               0 / +1 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                0 / 0 |

The exact 48,001,335-instruction reduction is the primary CPU result; cycles
also decrease. Wall-clock time moved oppositely under concurrent host load and
is diagnostic, not the selection measure. The one-byte startup `memcpy`
difference leaves calls unchanged, and neither binary performs a nonzero
`memmove`. Both symbolized reports reconcile with zero overflow, collision, or
probe-internal calls.

The owning resident-body gate also passes for 1, 4,096, and 8,193 words. It
reports the exact derived 1/4,096/8,193 direct reads, 0/0/2 chunk crossings,
one admission lookup, one region owner, zero additional retains or owner
acquisitions, zero whole-body copies, and zero warmed allocations.

## Validation and evidence

`cargo test -q --tests -p tex-state -p tex-command` passes tex-command's 384
unit and 23 boundary tests and tex-state's 549 unit, 12 boundary, and one
structural lifecycle tests. `cargo check -q -p tex-command --features
profiling` and the standalone resident-body profiling build pass. The full
format and clippy verdict is recorded by `scripts/check.sh`.

Ignored evidence is under `target/umber2-66p0.8.40.125/`. Authority
`perf.data`, raw copy report, symbolized copy report, self report, inclusive
report, and timing receipt SHA-256 values are respectively
`6ecead8912f92b7d425069973e5622713656b60b1dd6daacb22812484275b409`,
`d37ff0ad727afcda6e3e4c07668718c5223a665c16d44768902009ecae32bc4f`,
`0055b5677c662b594015d917e0311307f45de25229501549fbf78eb91f52e314`,
`037bfb607378e2ef7bfad643ba7befa6a1d6d9d48be6063cb9fe674c5f2a9812`,
`d5c23276e30fc2eca4421f1cc1f2a5187f010c62f7a1116bf9c55a87891643ae`,
and `f21437d708a971413a308def18fa06c80c885eb0fe25709f19f49a4530483887`.
Focused baseline/final counter receipt SHA-256 values are
`69a3cbbd8090433a14fc663095891506fb0df1c3982d96a06aec1b0cd371464d`
and `60987659f450d770762c57b4e98a4f8da40ff9a7ab829621b99a1f3a120a93a2`;
copy reports are
`9c3b8910f09c1f26c56f718d79e99b022bf6d8f9bff18810cf50cbbcc7db1964`
and `b3b473790b4303e12dc16bc4cfde829d68d40be60597c80a1caea4ea274bb444`.
Exact baseline/final binary SHA-256 values are
`4226f1bc83bb61011e97106c929d25d9a008f22435a4db11b576e9ba6f285c51`
and `a950b8d39bf39ea21c0c6fa5797741d46752932d37c3cba42304403a982c6220`.
