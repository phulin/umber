# `umber2-66p0.8.40.132`: derive raw-frame volume from its owner vector

## Exact current-main authority

Exactly one authenticated execution entered the engine on integrated commit
`a48c954ddcb40ef2229bf94506c094b23f5b0709`. The optimized profiling binary
SHA-256 was
`e557e962ab167755f0b8b6c29de05e31bbcf27fad73820d1b1599d7b78556acc`;
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
with aHash64 `df66c327ae636145`, and the ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
Guards were 20,000,000 canonical command fuel and 40,000,000 committed
executor steps. Expected status 1 occurred at vector
`(20000000,19907047,2216876,6018541,16781945,4011)`. Raw deliveries remained
source `463197`, stored/body `11520843`, macro argument `7922916`, and
synthetic end-v `91`.

The wrapper reported 8.13 s wall, 6.09 s user, 0.56 s system, and 231,452 KiB
peak RSS under concurrent host load. The capture contains 1,263 samples, zero
lost samples, and 14,624,925,054 weighted cycles.

## Current application-self and copy ranking

`advance_resident_command_into` led application self time at 10.56%. The next
application owners were `raw_delivery_entry` at 3.82%,
`expand_classified_into` at 3.30%,
`ExecutionScratch::append_argument_token` at 2.21%,
`ArenaListView::cursor_span_at_node` at 1.72%,
`MacroWordLane::get_sequential` at 1.35%, and `scan_toks_buffers` at 1.29%.
The public-copy probe itself was 5.09% self and is reported separately.

Exact public-copy attribution reconciled 7,287,659 `memcpy` calls for
1,080,804,810 bytes and 238,872 `memmove` calls for 40,776,774 bytes, or
7,526,531 calls and 1,121,581,584 bytes jointly, with zero collision overflow
or probe-internal calls. Relative to `.127`, the increase was 773,821
`memcpy` calls and 129,229,576 bytes plus 112,172 `memmove` calls and
18,845,152 bytes. The largest new rows were the active node-view/record
cutover's line-break traversal: its direct view callback alone accounted for
115,040 `memmove` calls and 19,326,720 bytes, and adjacent node-view callback
rows account for most of the new `memcpy` bytes. That lane, dense-arena work,
page material, and DVI parity were excluded from selection.

## Architectural simplification

The dominant resident transition's named nonsemantic subowner was
`CommandFuel::record_raw_delivery` at 0.83% self. Every resolved delivery
incremented both one entry of the exhaustive four-owner raw-delivery vector
and a separate `token_frame_steps` total. The authority itself proves their
identity: the four raw kinds sum exactly to 19,907,047 frame steps.

`CommandWorkDetail` now stores only the exhaustive owner vector. Cold work
publication derives `token_frame_steps` by summing its four bounded entries.
This removes one mutable word and one saturating update per raw delivery while
preserving the public telemetry vocabulary and exact terminal vector. It adds
no cache, threshold, alternate delivery path, or special case. Provenance,
suspension, rollback, retirement, acceptance, and TeX semantics are unchanged.

## Focused before/after gate

The exact baseline was integrated current main before this change. Both
release binaries ran the production `mixed_macro_resident_pipeline` once
under `perf stat` and the checked public-copy interposer. Both report
2,000,000 macro-body words, 1,000,000 parameter deliveries, 1,000,004 replay
words, 2,000,004 raw frame steps, 1,000,000 expanded deliveries, 1,000,001
macro expansions, zero command copies or suspension moves, and zero warmed
allocations or requested bytes.

| Counter                              |      Baseline |         Final |                 Delta |
| ------------------------------------ | ------------: | ------------: | --------------------: |
| User instructions                    | 2,400,757,245 | 2,392,757,928 |   -7,999,317 (-0.33%) |
| User cycles                          |   950,739,269 |   935,904,255 |  -14,835,014 (-1.56%) |
| Internal elapsed nanoseconds         |   372,917,927 |   332,558,783 | -40,359,144 (-10.82%) |
| Nanoseconds per macro-body word      |        186.46 |        166.28 |      -20.18 (-10.82%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                 0 / 0 |
| Public `memcpy` calls / bytes        | 132 / 344,575 | 132 / 344,572 |                0 / -3 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                 0 / 0 |

The exact instruction reduction is the primary CPU result: 3.9997
instructions per raw frame, matching removal of one bounded 64-bit update.
Cycles and elapsed time moved in the same direction but remain supporting
diagnostics under concurrent host load. Public copy calls are unchanged; the
three-byte process-total difference is startup layout noise. Both symbolized
reports reconcile with zero overflow and probe-internal calls.

## Evidence and validation

The full profiling `tex-command` suite passes 422 unit and 23 boundary tests;
the default suite passes 384 unit and 23 boundary tests. `tex-exec` passes 759
unit tests with two ignored, four main-control tests, and 24 external boundary
tests. The focused production gate and its exact work, allocation, ownership,
and copy invariants pass.
`scripts/check.sh` passes all four standard gates: dprint, biome, rustfmt, and
both declared clippy resolutions.

Ignored authority evidence is under `target/umber2-66p0.8.40.132/`. Authority
`perf.data`, raw copy report, symbolized copy report, self ranking, inclusive
ranking, and timing receipt SHA-256 values are respectively
`25311e9dbe173750ad4585357723cefbf38fff1eca1d6b4fe2a49ff0ac1c2014`,
`597c3038f52da2dd87600f9c84355a5a336eb4384b3fa57d065d2f5915ac01b2`,
`f05ff0c0948713155a39d405526c9b6c117d6ea1f5a7a0c961a958e16e7840b4`,
`11d397e8f46d2129b0b47008d498ade8433de20f89a962d6e41b22ed0d3bf236`,
`57f8cf53d42512eda229613702035262abd57b2618cb224148fd016222715284`,
and `81a913b5be479e619857846c8702dbbc6bf14b81f612ea87fa1d5130e812e421`.
Focused baseline/final binary SHA-256 values are
`9caf2ffa232d6ea1859af36ba3156121d0008fa47ba29f1b3971ba70a6af2b14`
and `33a9b235d425be41044434ccaf704fcbf380ecab6a59ba26abecbabc1b82a647`;
their counter receipts are
`59d239a41e1b4a32119f28f51b54f26e627c597e7e23561738959109a9bf4ed4`
and `1082af306648fafcb7495f13e14310ea9772f4a265589844c20ed323282fd260`;
their symbolized copy reports are
`b8d16c648bcce28cd5ab25d9cb744b2d4cc03b152f5c5fe753a50be0ac2f183d`
and `386e859d7709d18744394a29c18ea639a4fc0be3ce4e156956af3f79e27d49b5`.
