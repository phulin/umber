# `umber2-7asg.10`: destination-owned command collection

## Evidence boundary

The before/after comparison uses the authenticated arXiv `2606.12566`
workload from the combined copy audit: packed distribution root
`721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, and the 123-key closure. The source, format,
and key-list SHA-256 values are respectively
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

Both profiling binaries were built with
`RUSTFLAGS='-C force-frame-pointers=yes'`. The base executable has SHA-256
`8fe32a1b9fe8778f37e3d94cc19f753b57890082166661e99bef02223da91dd1`
and ELF build ID `2d289996b09a0120d9b1e976d285693bd90d8b83`. The candidate has SHA-256
`81844206ebe27d02800fac1bd45063e445d4b3c2f2aed700edc33d4a98c95378`
and build ID `f4d60e8f050669a757748f021a07101663c7853d`.

Every measured row was serialized with
`flock /tmp/umber-perf-host.lock`. Issue-private reproduction and receipts
live under `target/umber2-7asg.10/`; the interposer and analysis scripts are
ignored diagnostic evidence, not production tools.

## Structural change

`scan_toks` replacement collection now keeps one caller-owned
`Option<CurrentCommand<G>>` for the entire synchronous loop. Canonical raw or
expanded delivery writes into that destination. Classification, observation,
and spelling borrow the resident command, and successful progress clears the
option in place. Only semantic token backup and typed resource suspension move
the value out of the slot.

The same ownership rule removes concrete meaning clones in `pass_text`,
`scan_csname_characters`, and `is_expandable_command`: each operation borrows
the command and resolves its meaning once. No heap owner, meaning cache,
special-case workload path, or generation-retained destination was added.

## Exact public-copy result

The preload census interposes public `memcpy` and `memmove` separately and
records exact API, immediate caller, size, call count, and bytes before calling
the real libc implementation. Both runs had zero caller-table and size-table
overflow.

| Public API | Base calls | Candidate calls | Call change |    Base bytes | Candidate bytes |  Byte change |
| ---------- | ---------: | --------------: | ----------: | ------------: | --------------: | -----------: |
| `memcpy`   | 45,300,283 |      38,658,349 |  -6,641,934 | 7,201,016,266 |   6,276,876,662 | -924,139,604 |
| `memmove`  |     52,070 |          52,070 |           0 |     4,795,428 |       4,795,428 |            0 |

The exact command-work vector is identical in both rows:
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Both stop at the authenticated fuel boundary before
publishing an output file, with identical empty standard output and the same
canonical fuel diagnostic. Routine semantic, fixture, and output tests cover
completed runs.

## Honest call-site attribution

Owner-family rows are exact within one binary, but are not an attribution of a
shared generic function to whichever upper stack happened to symbolize first.
Inlining and compiler lowering also move immediate caller addresses between
builds. The defensible observations are:

- `collect_replacement` falls from 5,522,652 calls and 771,901,248 bytes to
  2,920,739 calls and 397,225,776 bytes. Source inspection and disassembly show
  the removed 144-byte handoff on ordinary progress; the remaining 136/144-byte
  calls are compiler lowering around option tests and drops plus the real
  backup/suspension exits, not a returned-command API.
- The shared `ResolvedMeaning::clone` entry falls from 3,557,468 calls and
  484,058,856 bytes to 459,051 calls and 62,674,144 bytes. That aggregate is
  reported only as shared evidence. The source-level claim is narrower: the
  concrete `pass_text`, `scan_csname_characters`, and expandability call sites
  no longer request those clones.
- The separate `scan_csname_characters` closure row of 1,242,105 136-byte
  calls disappears. Its ordinary character path borrows the slot; only missing
  `\endcsname` recovery consumes the command for backup.
- `scan_toks_inner`'s 320/328/368-byte progress carriers, command spelling,
  and undelimited macro argument collection remain separately visible. They
  were not relabeled as benefits of the shared clone reduction.

## Cycle and allocation confirmation

Paired `cycles:u`, 199 Hz, frame-pointer captures contain 1,674 and 1,675
samples with zero lost samples. Total approximate weighted cycles are
19,647,205,676 and 19,708,708,171. The shared glibc
`__memmove_avx_unaligned_erms_rtm` self bucket falls from 1,306,297,092
weighted cycles (6.65%) to 1,090,676,935 (5.53%), a reduction of 215,620,157
cycles. Public API counts and shared-kernel sampled cycles remain parallel
evidence because libc exposes both public APIs at the same implementation.

The paired guarded wall times are 9.31 and 9.26 seconds. The warmed
destination-directed benchmark performs 24,576 raw, token, and expanded
deliveries with zero allocation calls and zero requested bytes. The complete
`command_allocations` run retains zero-allocation macro argument matching,
command spelling, and token-list iteration rows.

## Verification

`cargo test -q --tests` passes the complete routine suite. The focused
`tex-command` tests pass 241 unit and 18 integration tests.
`scripts/check.sh` reports all dprint, Biome, rustfmt, and clippy gates passed.
The `destination_directed_warm_delivery` packed cutover row passes its exact
allocation contract.
