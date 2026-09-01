# `umber2-66p0.8.40.110`: exact public-copy caller attribution

## Profiling boundary

The checked profiling-only interposer in
`scripts/copy-attribution/copy_attribution_probe.c` counts public `memcpy` and
`memmove` calls separately. A direct return address in the main executable is
stored relative to its ELF load base. An external direct caller takes a bounded
48-frame unwind and uses the nearest application ancestor; if none exists, the
bin remains external and is reported by module-relative address. Instrumentation
recursion is excluded explicitly rather than folded into application totals.

Each API owns a fixed 32,768-bin atomic table. Open addressing stops after 24
probes, and any exhaustion becomes one explicit overflow caller bin. The report
includes collision probes, maximum displacement, overflow calls and bytes, and
probe-internal calls. Thus overflow cannot make the per-caller sum disagree with
the API total.

`scripts/copy-attribution/symbolize.py` independently parses and reconciles both
sums, ranks by bytes, hashes the exact binary, and sends all ranked application
addresses through one `addr2line -C -i` process. The checked opt-in gate
`scripts/check-tools.sh copy-attribution` compiles the interposer and exercises
an eight-byte scalar `memcpy`, a `Vec::extend_from_slice` `memcpy`, a shared-library
`memmove` attributed to its nearest Rust ancestor, and a separate shared-library
worker with no application ancestor. It verifies both totals, both application
chains, the separate external-only module bin, and zero table overflow. The
focused gate passes.

## Integrated authority

Exactly one integrated execution was recorded. It used profiling commit
`b28d6fc1f094b22f0b00080315921e57b89bf4ef`, binary SHA-256
`26c457895af9cc9dea71a46b8254a6664f837da6d51fcc3b38c2262dfbddf8a1`, and
probe SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The authority remained arXiv `2606.12566`: source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
explicit offline distribution manifest SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`
with aHash64 `721e833071d92bba`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source date epoch `1787080434`.

The expected status was canonical-command fuel exhaustion at exactly
`(50000000, 49911858, 9459678, 15939192, 35332486, 4203)` fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Raw deliveries were source `463704`, stored/body `30203660`,
macro argument `19244403`, and synthetic end-v `91`. Wall/user/sys were
`16.99 / 15.55 / 0.35 s`; peak RSS was `285464 KiB`. There was no comparison
run, fuel ladder, second workload, CPU profile, or storage change.

## Exact reconciliation and isolation

| API       |      Calls |         Bytes | Caller bins | Caller call sum | Caller byte sum |
| --------- | ---------: | ------------: | ----------: | --------------: | --------------: |
| `memcpy`  | 13,581,465 | 2,026,475,309 |       1,370 |      13,581,465 |   2,026,475,309 |
| `memmove` |    191,437 |    32,922,922 |         184 |         191,437 |      32,922,922 |
| Joint     | 13,772,902 | 2,059,398,231 |       1,554 |      13,772,902 |   2,059,398,231 |

All 1,554 bins were ranked and symbolized against that binary into 5,887
function/source frames. Every integrated bin was `application_direct`; nearest-
application-ancestor and external-only totals were both zero. They remain
separate report classes, as the focused shared-library controls prove. There
was no allocator/libc-only public-API caller to fold into an application bin;
kernel copies are outside the public `memcpy`/`memmove` interception boundary
and are not presented as application volume.

`memcpy` recorded 7,345 collision probes and `memmove` 214; maximum displacement
was one in both tables. Both APIs reported zero overflow calls and bytes and
zero probe-internal copy calls. The bounded table and unwind path therefore
lost no target calls.

The leading source chains were:

| API       | Rank |     Calls |       Bytes | Exact symbolized source chain                                                                  |
| --------- | ---: | --------: | ----------: | ---------------------------------------------------------------------------------------------- |
| `memcpy`  |    1 | 3,143,705 | 528,142,440 | `ChunkStorage<Node>::release_lineage` at `tex-state/src/fork_arena.rs:724`                     |
| `memcpy`  |    2 |   564,210 | 135,410,400 | `Result::expect -> MainControl::execute_cold_episode` at `tex-exec/src/main_control.rs:7365`   |
| `memcpy`  |    3 |   368,291 |  61,872,888 | `SourceCursor::next_compact_unicode_step` at `tex-command/src/input/tokenizer.rs:516`          |
| `memcpy`  |    4 |       286 |  48,757,243 | `Vec::extend_from_slice -> umber_fetch::cache::encode_entry` at `umber-fetch/src/cache.rs:521` |
| `memcpy`  |    5 |       286 |  48,757,243 | `slice::to_vec -> BlobStore::store` at `umber-fetch/src/cache.rs:197`                          |
| `memcpy`  |    6 |   252,328 |  42,391,104 | `Option::take -> ModeListMutation::take_pending_hchars` at `tex-exec/src/mode.rs:940`          |
| `memcpy`  |    7 |   249,209 |  41,867,112 | `Vec::push -> ListJournal::push_pending_value` at `tex-exec/src/mode/journal.rs:482`           |
| `memmove` |    1 |   178,251 |  29,946,168 | `PageMaterialArena::push_active_list` at `tex-state/src/page_node_arena.rs:1099`               |
| `memmove` |    2 |       128 |   1,520,512 | `CandidateRun::run` in `tex-incr/src/lib.rs`                                                   |
| `memmove` |    3 |     3,693 |     679,512 | `ForkArenaBuilder::push_with_identity` at `tex-state/src/fork_arena.rs:5158`                   |

The first `memmove` bin is 90.958% of all public `memmove` bytes. It is the
final placement of a 168-byte page node, not allocator or libc-only work. The
ranked output retains the remaining application and standard-library chains
without merging them into that owner.

## Architecture verdict

The largest architecturally unnecessary copy is the first `memcpy` bin:
3,143,705 copies of exactly 168 bytes, totaling 528,142,440 bytes or 26.062% of
all public `memcpy` bytes. Exact-binary disassembly at return offset `0x14c21cd`
shows `release_lineage` first calls
`drop_in_place<Option<tex_state::node::Node>>` on the resident slot, then calls
`memcpy` with `0xa8` bytes from one preconstructed `None` value back into that
already-released slot. The dynamic volume is vacancy representation traffic,
not semantic node transfer, allocator work, or a required destructor copy.

No engine storage changed here, so no before/after result is claimed. Follow-up
`umber2-66p0.8.40.113` owns a principled safe vacancy representation and the
required focused one/4,096 release gate plus exact before/after public-copy
census. The issue specifically forbids adding per-node allocation, duplicate
storage, unsafe engine code, or merely shifting the bytes to `memmove`.

## Evidence

Ignored issue-local evidence lives under
`target/umber2-66p0.8.40.110/integrated/`. The raw exact report has SHA-256
`ccf01dc8edda007acfeeef0c550f99264bb3dae9d963353d304d3543201494a1`;
the complete 1,554-bin symbolization has SHA-256
`503d1784fa7e3e160c983e6efcd2d567dd838964f32e95b24de7d4994ab9ccf0`;
`time.txt` has SHA-256
`e647a646d06529084f2a369214194d67483ad481906924975d724c750ef1c3f1`;
and the engine diagnostic stream has SHA-256
`0864f6b6c91507cd27f262e419306c5160bc146c9be77fef8c593e40ef39d312`.
