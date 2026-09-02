# `umber2-66p0.8.40.135`: carry a hyphen decision, not a node

## Post-`.134` owner selection

The authenticated `.134` authority reported 7,210,352 public `memcpy` calls
for 893,028,925 bytes and 238,988 public `memmove` calls for 40,785,918 bytes.
The active logical-node work owns `ChunkStorage::release_lineage`, the direct
line-break collection rows, and `PageMaterialArena::push_active_list`; DVI
parity is also separately owned. Those lanes were excluded.

The largest independent remaining hot chain was TFM literal-hyphen emission.
Every completed glyph constructed an `Option<Node>` even when it was not a
literal hyphen, assigned that 168-byte value to a pending slot, took the value
at the next non-auto-kern item, and extended the output vector from it. The
`.134` report attributed 112,216 calls / 18,852,288 bytes to the pending take,
82,348 / 13,834,464 to glyph delivery, 82,348 / 13,834,464 to pending
discretionary assignment, and 16,616 / 2,791,488 to the terminal pending
extension. The complete locus therefore moved 49,312,704 bytes. Only the
glyph's final output delivery and a real discretionary's final delivery are
semantic.

The largest non-node `memmove` row outside the excluded page lanes remained
`CandidateRun::run` at 122 calls / 1,423,862 bytes, much smaller than this
`memcpy` chain.

## Ownership simplification

The TFM machine now retains one scalar `pending_literal_hyphen_disc` decision
while following auto kerns. Once the output position is known, it constructs
the explicit discretionary directly into the output vector. No `Node` exists
before its semantic owner can accept it.

This preserves the original ordering rule: auto kerns immediately after a
literal hyphen precede its discretionary, while every other item first closes
the pending position. Missing glyph handling, ligature folding, origins,
right-boundary suppression, fuel retry, operation rollback, candidate
settlement, deterministic output, and WebAssembly use the same paths and data.
The change adds no cache, threshold, alternate execution path, allocation, or
unsafe code.

## Focused exact gate

The focused gate ran one deterministic pdfLaTeX job containing 4,096 literal
hyphens under the checked `.132` public-copy interposer and
`cycles:u,instructions:u`. Both binaries used the same schema-12 format,
offline distribution, source-date epoch, fuel guards, isolated cache, and
output path shape. The source SHA-256 was
`3eb9f9915f8640525a4acc95b621f833de4b7e31268b41f5a3feb2fab28621e8`.

| Counter                                |                 Baseline |                    Final |                    Delta |
| -------------------------------------- | -----------------------: | -----------------------: | -----------------------: |
| Selected TFM `memcpy` calls / bytes    |       28,678 / 4,813,808 |        8,193 / 1,376,424 |     -20,485 / -3,437,384 |
| Whole-process `memcpy` calls / bytes   |  6,194,115 / 541,966,435 |  6,173,558 / 538,527,206 |     -20,557 / -3,439,229 |
| Whole-process `memmove` calls / bytes  |     443,903 / 98,329,730 |     442,270 / 98,270,858 |         -1,633 / -58,872 |
| Hot-core allocations / requested bytes | 226,717 / 17,256,543,409 | 226,717 / 17,256,543,409 |                    0 / 0 |
| User instructions                      |            7,732,564,294 |            7,728,184,584 |     -4,379,710 (-0.057%) |
| User cycles                            |            5,285,923,791 |            3,872,185,267 | -1,413,738,524 (-26.75%) |
| User / system CPU seconds              |              2.17 / 0.15 |              1.62 / 0.13 |            -0.55 / -0.02 |
| Peak RSS                               |              143,564 KiB |              145,768 KiB |               +2,204 KiB |

The exact instruction reduction is the primary CPU result. Cycles and CPU
seconds moved in the same direction but remain host-noisy supporting results;
peak RSS increased by 2,204 KiB, so this single focused pair does not claim a
residency improvement. It does prove the exact copy and allocation change.
The 14,656-byte PDF, empty stdout, work census, and terminal status are
byte-identical; both PDF SHA-256 values are
`c0de63974d02cdb4c41cf44bff488489485268a33188e906ec5cdb1dfc037703`.

Baseline/final profiling binary SHA-256 values are
`1a5b15c0c3e0edae90f870a56cec518a5ed9dd1e58655916a68269c6ef0992fd` and
`89053e71ac8c7653684124ed9b023b018f3017d6af16da7bb11dcd9b984fd2bc`.
Baseline/final symbolized copy report SHA-256 values are
`24cb04efd9ea9e15e4fe2a7f3a34e97adc76ce66ea9a6f4afbe603c73d136de9` and
`9bd5ebb5149ef283628125f7c2725851df4ea15e4fe190fc9b3e00bf2d351e3e`.
Ignored evidence is under `target/umber2-66p0.8.40.135/`.

## Remaining owners and validation

Without rerunning a broad profile, the unchanged `.134` ranking leaves the
pending-horizontal-run rollback chain as the next aggregate ownership audit:
`ModeList::take_pending_hchars`, `ModeListMutation::take_pending_hchars`, and
their typed journal lanes account for about 37.3 MiB of public `memcpy`.
Independent rows include `evaluate_ifx` at 94,786 calls / 13,649,184 bytes and
`clear_discretionary_replacements` at 59,434 / 9,984,912 bytes. Third-party
inflate rows are semantic decompression rather than redundant ownership.
After the excluded page lanes, `CandidateRun::run` remains the leading
application `memmove` owner at 122 / 1,423,862.

Validation passed the complete `tex-exec` suite (759 + 4 + 24 tests, with the
two declared executor tests ignored), the related `tex-state`, `tex-typeset`,
and `umber` suites, and `umber-wasm` compilation for
`wasm32-unknown-unknown`. The focused output and work-vector identity checks
passed. `scripts/check.sh` passed dprint, Biome, and rustfmt, and reported no
diagnostic from this change. Its clippy gate remains red solely on the active
logical-node implementation's pre-existing `tex-state` dead-code set: 86
union-pass and 82 shipping-pass diagnostics. This excluded slot did not alter
or suppress that work.
