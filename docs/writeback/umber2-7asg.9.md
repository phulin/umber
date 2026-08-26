# `umber2-7asg.9`: combined copy-kernel ownership audit

## Evidence boundary

This investigation measures commit
`d114f669fdb781249a38707eaaf36543900681fc`. It uses the immutable arXiv
`2606.12566` source rooted in slot 2, packed distribution root
`721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, and the 123-key authenticated closure. The
`ArXiv.tex`, format, and key-list SHA-256 values are respectively
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

One executable was built with full debug information and
`RUSTFLAGS='-C force-frame-pointers=yes'`, then copied once into the
issue-private evidence directory. Its SHA-256 is
`5be8e52340e4f78915847eb850a6327001219594e4378122dda0395bebabbb38`, ELF
build ID is `9a9d575f6ad5849c58cce7914780d868b51bbd76`, and size is 386,642,576
bytes. The runner verifies that digest before every row. The census and perf
receipts additionally name the same path, device `64513`, and inode `39879227`;
the perf header records that path as its executed command. Census and perf
therefore used the byte-identical force-frame-pointer ELF, rather than merely
two builds from the same commit.

The control, final dual-entry census, and perf rows all stopped at exact
command-fuel exhaustion with command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`. The control took 9.60
wall seconds and 326,952 KiB RSS; the final census took 10.15 seconds and
366,764 KiB. Census timing is diagnostic only. Every measured host row was
serialized with `flock /tmp/umber-perf-host.lock`.

Reproduction and complete issue-private evidence live under
`target/umber2-7asg.9/`: `run-row.sh`, `copy_probe.c`, `exact-sizes.tsv`,
`caller-summary.tsv`, `owner-summary.tsv`, `symbolized-stacks.tsv`, and the
`perf-20m/` reports. These ignored measurements are not production tools.

## Public entry-point census

The preload probe interposes public `memcpy` and public `memmove` separately,
records exact API, immediate return address, size, calls, and bytes, and then
delegates to the resolved libc implementation. It does not replace libc with
a scalar copy during the measured work. Both public symbols resolved to the
same `/lib/x86_64-linux-gnu/libc.so.6` offset `0x191640`. All 10,751 exact
API/caller/size rows returned directly to the Umber ELF; none returned to
libc. Both tables had zero overflow.

| Public entry |      Calls |         Bytes | Mean bytes/call |
| ------------ | ---------: | ------------: | --------------: |
| `memcpy`     | 45,982,145 | 7,259,043,927 |          157.86 |
| `memmove`    |     51,947 |     4,768,860 |           91.80 |

The following disjoint `memcpy` families are material by the prior audit's
threshold: at least 1% of public calls or bytes. They cover 36,134,147 calls
(78.58%) and 5,669,081,953 bytes (78.10%). `exact-sizes.tsv` retains every
individual immediate caller and size; the grouped table is the compact owner
ranking.

| Phase and concrete Rust owner family                                                                                                             | Structural cause                                                                |      Calls |         Bytes | Material sizes                           |
| ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------- | ---------: | ------------: | ---------------------------------------- |
| Hot command delivery: `collect_replacement`, `ResolvedMeaning::clone`, `get_x_token`, `get_token`, `scan_toks_inner`, spelling, macro argument   | Repeated `CurrentCommand`/meaning and token-list scanner value delivery         | 14,325,194 | 2,073,102,576 | Mostly 136/144; also 152/184/320/328/368 |
| Hot executor delivery: `preflight_replay_delivery`, `drain_file_framing_events`, `apply_prepared_operation`, `prepare_operation`, direct episode | Preflight, replay, prepared-operation, and execution carrier transfers          |  7,830,409 | 1,567,118,987 | 136--632; one 4,073-byte value           |
| Hot processor/capability refresh: `CommandProcessor::from_parts`, `last_node_value`, `page_insertions`, `ignored_depth_with_handle`              | Repeated borrowed processor facade and call-local capability values             |  3,200,673 |   665,739,984 | 208                                      |
| Hot scalar scanners: `finish_scalar_call`, `scan_something_internal`, `scan_integer`                                                             | Whole scalar-result/suspension carriers                                         |    861,633 |   648,667,896 | 712/752/792                              |
| Input admission: `register_incremental_inputs`                                                                                                   | `file.bytes().to_vec()` copies immutable workspace file bodies into the session |     21,250 |   465,275,787 | 329--6,762,750                           |
| Hot mode mutation: `ListJournal::record_once`                                                                                                    | Whole inverse-journal entry transfers                                           |    233,236 |   145,539,264 | 624                                      |
| Hot call-count tail: generic `String::clone`, `AHash64Hasher::write`, token display/name/line helpers, `ExecutionScratch::commit_macro_match`    | Many zero-to-small copies; material by calls but not bytes                      |  9,661,752 |   103,637,459 | Mostly 0--162                            |

The direct API caller is exact. An inline chain at that address is also exact,
but the one stack saved for each direct address is only representative. In
particular, generic `String::clone` serves multiple upper callers, so its
3,026,448 calls and 71,949,724 bytes are not assigned wholesale to the first
observed upper stack. The raw tables retain this distinction rather than
inventing exact semantic-owner counts above a shared generic routine.

The final `memmove` census reproduces the prior 51,947-call result on the new
single-ELF boundary. The material families cover 49,694 calls (95.66%) and
4,550,652 bytes (95.42%).

| Phase and concrete Rust owners                                                                    | Cause                                                          |  Calls |     Bytes | Sizes                     |
| ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- | -----: | --------: | ------------------------- |
| Format startup: `tex_state::format::validate_logical_rows`                                        | B-tree name/cell-index insertion shifts                        | 30,386 | 2,529,712 | 8--240                    |
| Hot execution: `EngineUsageRuntime::make_string`                                                  | `BTreeSet<String>` key and edge shifts                         | 15,568 | 1,402,840 | 8--240                    |
| Hot font resource: `FontStore::intern`, cold font apply, `CommandHostCapabilities::register_font` | `LoadedFont`/`FontResource` semantic transfers plus map shifts |    357 |   274,648 | 0--4,608; fixed 504/1,288 |
| Distribution startup: `DistributionResolver::resolve_batch_with_prefetch`                         | B-tree insertion/removal and vector growth                     |  1,245 |   112,516 | 0--800                    |
| Format startup: `FontStore::from_frozen`                                                          | `LoadedFont` transfers plus font-map shifts                    |    545 |   111,872 | 8--1,288                  |
| Input refresh: `World::record_input_dependency`, `register_input_probe`, `take_registered_source` | COW root transfer, B-tree shifts, and 944 zero-byte removals   |  1,593 |   119,064 | 0--1,280                  |

## Shared libc kernel and internal traffic

The `cycles:u`, `-F 199`, frame-pointer perf capture has 1,617 samples, zero
lost, and 19,188,279,156 approximate weighted cycles. The shared glibc symbol
`__memmove_avx_unaligned_erms_rtm` owns 123 self samples and 1,432,580,357
weighted cycles, or 7.47% of the complete capture. Because public `memcpy` and
public `memmove` resolve to that same address, perf cannot split these cycles
by API. Exact API counts/bytes and sampled shared-kernel cycles are therefore
parallel evidence, not a fabricated join.

| Defensible shared-kernel ancestry                             | Samples | Weighted cycles | Kernel self share | Complete-run share |
| ------------------------------------------------------------- | ------: | --------------: | ----------------: | -----------------: |
| Unresolved above the shared kernel                            |     103 |   1,209,574,591 |            84.43% |              6.30% |
| Application `BTreeMap::clone_subtree` ancestry, API ambiguous |      13 |     143,267,192 |            10.00% |              0.75% |
| Libc-hidden/internal `realloc` ancestry                       |       5 |      54,891,421 |             3.83% |              0.29% |
| Application `core::fmt::write` ancestry, API ambiguous        |       1 |      12,423,753 |             0.87% |              0.06% |
| Application detached-artifact source ancestry, API ambiguous  |       1 |      12,423,400 |             0.87% |              0.06% |

The `realloc` row is the concrete hidden-libc evidence: those copy-kernel
samples have `realloc` immediately above the shared implementation, while the
public census has no libc return-address row. It is internal allocator traffic
and cannot honestly be called either public `memcpy` or public `memmove`.
Seven additional blocks containing 56,074,403 weighted cycles have an
unresolved context marker before the shared symbol; `perf report` does not
count them as shared-kernel self, so `copy-kernel-samples.tsv` retains them as
non-self callchains outside the table.

Compiler-inlined copies are a fourth, separate population. They do not cross
either public libc entry and therefore have no call/byte row in this census;
their cycles remain charged to the surrounding Rust self symbols. The 7.47%
libc bucket is not a bound on all copying, and no per-owner cycle estimate is
made for public census rows without supporting sampled ancestry.

## Follow-up ownership

This audit files `umber2-7asg.10` for hot command-value seams,
`umber2-7asg.11` for scalar scanner carriers, `umber2-7asg.12` for processor
facade/capability values, and `umber2-7asg.13` for mode inverse-journal entries.
It adds the 465 MB input-body admission fact to existing `umber2-7asg.8`.
Existing `umber2-7asg.1`, `.6`, and `.7` continue to own prepared-operation,
string-pool, and font transfers respectively. No production code,
representation, behavior, or architecture changed.
