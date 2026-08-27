# `umber2-66p0.41`: singular preflight command owner

## Architecture deletion

`OperationFrame` now owns one `PreflightCommand`: the sole optional current
command plus compact raw, settled, expanding, prefix, direct-scan, and immediate
PDF phase state; its delivery cursor; its ABA-tagged scalar scanner child; and
its direct-operation phase. Raw preflight, ordinary expanded delivery,
main-loop lookahead, alignment handoff, prefix scanning, leader handoff, and
`goto reswitch` borrow and mutate that owner in place. A semantic backup or
recovery moves the command into the command-owned input transition, and only a
real resource suspension moves the occupied frame into retained retry state.

The command-bearing `OperationDelivery` variants, `PendingPreflightCommand`,
`PendingOperationScan`, `PendingPrefixedCommandScan`, and `PendingPrefixScan`
are deleted. So are `for_delivery`, `with_cursor`, `with_scanner`, speculative
current-command cloning, and whole-enum reconstruction. `OperationDelivery`
now carries only payload-free command/replay statuses plus the pre-existing
alignment, hot, and prepared payloads whose ownership is unrelated to the
current command. Immediate PDF retry is a commandless terminal phase because
the recursive PDF command has already been semantically consumed.

The replacement adds no cache, special workload path, heap indirection,
compaction, lifetime owner, or alternate delivery loop. Source-boundary tests
require the inline command, phase, cursor, scanner, and operation-scan fields;
forbid heap-backed continuation storage; and reject every retired mirror type,
reconstruction helper, and `command.clone()` site in executor dispatch.

## Exact behavior and copy evidence

The authenticated candidate uses source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
distribution manifest SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`,
and ordered prefetch-key SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The frame-pointer profiling-profile ELF has SHA-256
`d32e5994468dd12171dcaca34a81c8348bf017aac4e0e3ac1c6169736ac1689c`
and build ID `03c788ce62b9ae54a84e9fbf8841e37138f2f8a7`.

Both the uninstrumented control and public-copy census stop intentionally at
status 1 with exact command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Both have empty standard output and publish no partial PDF.

The final census reports 31,656,809 public `memcpy` calls / 4,141,705,961
bytes and 51,948 `memmove` calls / 4,767,012 bytes, with zero caller or size
overflow. The latest prior combined authority at `b1279a623` reported
33,535,478 / 4,457,104,526 and 51,948 / 4,767,012 respectively, so the
cumulative current tree has 1,878,669 fewer `memcpy` calls and 315,398,565
fewer bytes while `memmove` is identical. This historical comparison includes
the intervening integrated ownership deletions and is not presented as an
isolated timing claim. The exact isolated evidence is structural: the two
named 144-byte rebuild owners and all speculative command clones no longer
exist in source or the candidate symbol table. Per coordinator direction, no
paired CPU profile was run.

## Verification and follow-up

The complete `tex-exec` suite and `cargo test -q --tests` pass.
`scripts/check.sh` reports all four gates passed, including both Clippy
resolutions. The excluded `benchmarks/tex-exec` allocation executable is
currently stale against `LoadedFont::new`: its fixture supplies a 32-byte hash
where the production API now requires eight bytes. The failed diagnostic
changed no tracked benchmark file; `umber2-md70` tracks that independent repair
and the three documented allocation rows.
