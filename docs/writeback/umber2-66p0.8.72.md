# `umber2-66p0.8.72`: stationary operation frame

## Evidence boundary

The focused comparison starts at post-`.8.71` commit
`7632bdf244b565d596257e568294fb323672cee7`. Both rows use the same fixed-clock
ArXiv authority, schema-12 format, ordered 123-key resource closure, offline
packed distribution root `721e833071d92bba`, and zero-loss public-copy probe.
The source, format, key closure, and probe have SHA-256 identities
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and `12effee0f78fedd8326a6a6f92adbec4881efaa4818616ccb2866129dc44c6fe`.

Both runs stop at authenticated work vector
`(1000000,1000000,116943,267911,898107,67)`: fuel charges, token-frame steps,
expanded deliveries, meaning lookups, scanner tokens, and write expansions.
The public `memcpy` and `memmove` symbols both resolve to offset `0x191640` in
the same libc image and are therefore one sampled implementation kernel. Every
row has zero caller and size overflow.

## Stationary lifecycle

One caller owns one `OperationFrame`. Delivery and scanning fill its command,
scalar, retry, and mutually exclusive `OperationPayload` fields. A tiny
`ScannedOperation::{Hot,Cold}` marker selects preparation without carrying the
operation. Preparation mutates the resident hot operation or changes only a
cold operation's small `OperationTokenRoot` and `OperationDefinitionRoot`
fields from attempt coordinates to prepared owners. Application borrows the
same payload mutably, consumes only semantic leaves, and clears the slot after
commit. The complete frame moves only into a typed resource or scanner
suspension; `SuspendedScannedOperation` exists only while rebuilding that
genuine suspension.

## Exact copy census

| Public API             | Post-`.8.71` calls / bytes |     Final calls / bytes |                 Delta |
| ---------------------- | -------------------------: | ----------------------: | --------------------: |
| `memcpy`               |    2,577,713 / 178,726,947 | 2,515,161 / 165,600,651 | -62,552 / -13,126,296 |
| `memmove`              |            2,069 / 388,134 |         2,185 / 389,814 |         +116 / +1,680 |
| Combined shared kernel |    2,579,782 / 179,115,081 | 2,517,346 / 165,990,465 | -62,436 / -13,124,616 |

Combined calls fall 2.420% and bytes fall 7.327%. The small `memmove` increase
is 1,680 bytes, so the removed families did not shift to the other public API.

## Attribution proof

| Named owner                         | Post-`.8.71` calls / bytes | Final calls / bytes |
| ----------------------------------- | -------------------------: | ------------------: |
| `prepare_scanned_cold_operation`    |            1,746 / 391,104 |               0 / 0 |
| `preflight_replay_delivery`         |            1,645 / 329,000 |               0 / 0 |
| `OperationFrame::write_unavailable` |          6,786 / 1,438,632 |             2 / 528 |
| `apply_prepared_operation`          |         13,968 / 2,989,152 |     3,492 / 838,080 |

The two residual frame writes are uncommon 264-byte cold payload installation,
not an ordinary phase-to-phase frame move. The ordinary scanner's 1,742
264-byte `scan_command` rows are likewise the one-time `ColdOperation` payload
installation, not a whole-frame transfer. The remaining 240-byte
`apply_prepared_operation` rows symbolize to `Result::expect` at command-context
admission; other 240-byte rows in application call stacks attribute directly
to named-token-list publication. The broad
`execute_direct_episode` owner retains 9,287 calls, but its 240-byte rows
symbolize to option take/replace and command-context work; no operation-frame
move appears there. Ordinary 200--240-byte operation-frame rows are absent from
the exact `memcpy` and `memmove` tables.

## Integration-tip confirmation

The rebased composition on integration tip
`3c13318b08e55431447543cdd687830e656d70c4` preserves the stationary payload
while also retaining the singular scalar scan frame, compact active-source
delivery, two-lineage editor rebinding, lazy diagnostic coordinates, and the
deleted stale expansion field. The fresh exact 1M row under
`target/umber2-66p0.8.72/rebased-1m-v2/` reproduces the authority vector
`(1000000,1000000,116943,267911,898107,67)` with zero caller/size overflow.
`memcpy` reports 2,545,954 calls / 172,969,471 bytes and `memmove` reports
2,171 calls / 386,230 bytes; both still resolve to the same libc offset.

Fresh symbolization confirms that no whole-operation-frame row moved to either
API. `OperationFrame::write_unavailable` is absent; the only
`prepare_scanned_cold_operation` row is a zero-byte `Vec::extend_from_slice`;
`preflight_replay_delivery`'s nonzero row is a 440-byte processor drop; and
`apply_prepared_operation`'s 240-byte rows are exclusively three
`Result::expect` sites at admitted command-context boundaries. No matching
owner appears in the `memmove` table. The named ordinary frame-copy families
therefore remain zero rather than shifting between the shared libc entry
points.

## Focused verification

- `fused_hot_and_typed_cold_dispatch_share_one_interpreter`
- `production_batch_keeps_ordinary_prefix_on_resource_need`
- `unified_operation_resource_suspension_is_observation_independent`
- `one_and_4096_hot_operation_frame_phase_cycles_are_allocation_free_and_scalar`
- `diagnostic_coordinate_allocates_only_when_published_and_rejects_stale_input`
- `stream_open_retry_keeps_detached_rendered_source_context`
- `diagnostic_input_retry_reuses_the_retained_delivery_attempt`
- `active_source_lookup_is_one_top_read_at_one_and_4096_replay_levels`
- `source_owner_swap_candidate_reject_redoes_prior_and_accept_promotes_current`
- `late_edit_rebinds_the_restored_root_source_and_rejection_restores_it`
- `changed_prefix_convergence_rehomes_suffix_for_the_next_edit`

All pass. The operation-frame allocation test reports zero allocation calls
and bytes for one and 4,096 complete frame cycles. No broad gate or long
profile was run.
