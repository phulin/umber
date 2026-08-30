# `umber2-66p0.8.71`: singular operation-result ownership

## Evidence boundary

The focused census compares base
`c8b10c950600211d6e61a775b3ec1f87dbda3598` with the final source change. The
baseline profiling binary has SHA-256
`bb57b87abb4d27e42ca96b8158bf0fecd3e3258851e621a85fa6b1861581ec0d`; the
candidate has SHA-256
`4dcd35827ffef73d8a6478a8087053bacd35c19a6ff470f9b83083cb823e89b4`. Both
runs use the same zero-loss copy probe, whose SHA-256 is
`12effee0f78fedd8326a6a6f92adbec4881efaa4818616ccb2866129dc44c6fe`.

The authority is the fixed-clock ArXiv workload used by the current
performance work. Its source, schema-12 format object, and ordered input-key
closure have SHA-256 identities
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`, and
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The packed distribution is pinned by aHash64 root `721e833071d92bba` and is
used offline.

Both rows stop at the exact authenticated work vector
`(1000000,1000000,116943,267911,898107,67)`, meaning fuel charges, token-frame
steps, expanded deliveries, meaning lookups, scanner tokens, and write
expansions respectively. The public `memcpy` and `memmove` entry points both
resolve to `0x191640` in the same libc image, so they are treated as one
implementation kernel while still being counted separately. Every public-API
row reports zero caller overflow and zero size overflow.

## Attribution and selection

After excluding startup work and families already deleted by semantic
`InputStack`, operation-owned scalar frames, and linear paragraph/hyphenation
traversal, the leading repeated production owners were:

| Owner                                    | `memcpy` calls | `memcpy` bytes |
| ---------------------------------------- | -------------: | -------------: |
| `MainControl::preflight_replay_delivery` |         55,710 |     11,809,616 |
| `CommandProcessor::scan_toks_inner`      |         19,754 |      5,177,636 |
| `MainControl::apply_prepared_operation`  |         13,968 |      2,989,152 |

The first family was independently avoidable. A successful scan was moved
through `Result<ScannedOperation>`, an outer tuple, and payload-bearing
`OperationDelivery`, although the caller's reusable `OperationFrame` already
had exactly the lifetime needed to own that result until application or
suspension.

## Ownership change

`OperationFrame` now has one mutually exclusive `OperationPayload` slot for
unavailable, prepared-cold, or hot operation state. Successful preflight moves
the operation directly into that caller-owned slot and returns only a compact
delivery tag plus scalar capabilities. Cold preparation changes the same slot
in place; application takes the payload once. The replacement adds no heap
owner, cache, alternate path, duplicate representation, registry, compaction,
or unsafe code. Resource suspension still moves the occupied frame as its sole
owner.

## Exact A/B

| Public API             |  Baseline calls / bytes | Candidate calls / bytes |                Delta |
| ---------------------- | ----------------------: | ----------------------: | -------------------: |
| `memcpy`               | 2,606,232 / 184,623,699 | 2,577,713 / 178,738,375 | -28,519 / -5,885,324 |
| `memmove`              |         2,069 / 396,534 |         2,069 / 388,134 |           0 / -8,400 |
| Combined shared kernel | 2,608,301 / 185,020,233 | 2,579,782 / 179,126,509 | -28,519 / -5,893,724 |

Aggregate calls fell 1.093% and aggregate bytes fell 3.185%. No calls shifted
from `memcpy` to `memmove`: `memmove` calls are exactly unchanged and its bytes
also fell. The selected `preflight_replay_delivery` owner fell from 55,710
calls / 11,809,616 bytes to 1,645 calls / 329,000 bytes, reductions of 97.047%
and 97.214%. Its small remainder is the cold-operation transition. The frame
itself shrank: the same fourteen measured frame moves fell from 184,898 to
176,498 bytes.

## Focused verification

- `cargo test -q --tests -p tex-exec fused_hot_and_typed_cold_dispatch_share_one_interpreter`
- `cargo test -q --tests -p tex-exec production_batch_keeps_ordinary_prefix_on_resource_need`
- `cargo test -q --tests -p tex-exec unified_operation_resource_suspension_is_observation_independent`
- `cargo test -q --tests -p tex-exec --features profiling hot_`

All focused tests pass. The profiling test exercises one and 4,096 complete
hot-operation frame cycles and reports zero allocation calls and bytes. No
broad gate or long whole-engine profile was run.
