# `umber2-66p0.8.40.121`: remove the freshness-publication census

## Selection from the exact 20M authority

Issue `.118`'s authenticated integrated arXiv `2606.12566` capture remains the
sole broad-profile authority. Its application self table, after excluding the
copy probe, ranks `CommandState::advance_resident_command_into` first at 14.39%
and `CommandProcessor::raw_delivery_entry` next at 4.44%. Issue `.119` removed
the former path's parallel resident-domain census. The next 2.03% owner,
`ArenaListView::cursor_span_at_node`, and the 391,001,688-byte
`ChunkStorage<Node>::release_lineage` copy row belong to `.113`; active DVI
parity is excluded. After `.118` removed the independent 61,798,296-byte source
carrier, no other individual public-copy row reaches 50 MiB. No new broad
profile was run.

Raw delivery must select and advance the semantic input row, resolve its
meaning, charge canonical work, apply alignment and outer-command rules, and
publish the exact input-level/position coordinate needed to reject stale
backup or alignment handoff. The `profiling` resolution additionally updated
`delivery_freshness_writes`, a second saturating count of each successful
coordinate publication. That counter drove no behavior and duplicated the
focused fixture's known delivery volume.

## Architecture change

The required `immediate_delivery_stamp`, observation sequence, and typed retry
cursor remain unchanged. `CommandProcessor` no longer owns, initializes,
increments, or exposes a parallel freshness-publication census. Focused
fixtures derive their exact admitted delivery volume at the measurement
boundary, as they already do for resident storage domains. This removes one
field and one universal profiling hot-loop update without a cache, threshold,
fast path, new owner, or semantic classifier. The optimized focused
`raw_delivery_entry` symbol shrinks from 1,239 to 1,218 bytes.

## Focused before/after gate

The existing `fused_raw_expanded_delivery` row performs 1,000,000 raw and
1,000,000 expanded stored-control-sequence deliveries across exact known
replay, attempt-local, and durable spans. Both exact binaries report PASS with
zero warmed allocations/requested bytes, zero intermediate relays, zero
whole-command/input copies, 2,000,000 fuel charges, 2,000,000 token-frame
steps, 2,000,000 meaning lookups, and 1,000,000 expanded deliveries.

One `cycles:u,instructions:u` execution and the checked `.118` public-copy
interposer were used for each binary. Both copy reports reconcile without
overflow or probe-internal calls.

| Counter                            |         Baseline |            After |                 Delta |
| ---------------------------------- | ---------------: | ---------------: | --------------------: |
| User instructions                  |    1,220,985,706 |    1,211,985,469 |  -9,000,237 (-0.737%) |
| User cycles                        |      477,802,057 |      488,430,148 | +10,628,091 (+2.224%) |
| Warmed allocations/requested bytes |            0 / 0 |            0 / 0 |                 0 / 0 |
| Public `memcpy` calls/bytes        | 132 / 24,338,375 | 132 / 24,338,364 |               0 / -11 |
| Public `memmove` calls/bytes       |            2 / 0 |            2 / 0 |                 0 / 0 |
| Hot resident-delivery copies       |            0 / 0 |            0 / 0 |                 0 / 0 |

The removed update saves 9,000,237 retired instructions, or 4.50 instructions
per complete delivery, without changing semantic work, allocation, or either
public copy call count. Host-frequency noise made the single candidate cycle
row slower, so it is not presented as a cycle improvement; the exact retired-
instruction reduction is the focused CPU result. The 11-byte `memcpy`
difference is startup layout noise, not a new or shifted application owner.

## Validation and evidence

`cargo check -q -p tex-command --features profiling` passes. The complete
`cargo test -q --tests` run passes every suite except
`pdf_parity::committed_embedded_font_fixtures_match_bytes_structure_and_attestations`;
that deterministic embedded-Type1 fixture mismatch reproduces from `.118` and
remains tracked by `umber2-emmj`. In particular, tex-command's 383 unit and 23
boundary tests pass. `scripts/check.sh` reports all four repository gates
passed: dprint, Biome, rustfmt, and both clippy resolutions.

The profiling tex-command test target is independently unable to compile
because `input/history/tests.rs` still constructs the already-removed
`InputLevel::Tokens` variant and old `TokenCursor` fields. That integrated-base
drift is filed as `umber2-48o7`; this issue does not change the input topology.

Ignored evidence is under `target/umber2-66p0.8.40.121/`. Baseline binary,
symbolized copy report, hardware-counter receipt, and semantic stdout SHA-256
values are respectively
`a92b573e5cda64c2e91c6acd3a1f8c95115f14daa035067f0bf5827c74acfead`,
`079db9b4e5a9e75726f3c9cefd1c718e447a9d35fdc463eae40adc88d933a448`,
`ab9db804a3ab0fc1dcaeb7dbfeb93104c7aca2d53fdc159afd9fa3d7b28876ab`,
and `d074d11ee756152a7d2884297b7066fed9f6fb7848934bfaebf2b496df55ca66`.
Candidate values are
`6905d1a0308975dffa0eef03cc21aae9d017d05a69072824ba514ec2fad0de76`,
`55f8d46ab1f52cf890012cff8a9bf1387e3391e35c80a8e018fba4a49eca7ce0`,
`d5779028ad62707bd0fd1180a96c48459c47f11db361006f740b3de26fafe82a`,
and `510c572c35c1c0dca441a9ef6936f47f24cf83f6aeabdc352a9eb7685e9a98eb`.
