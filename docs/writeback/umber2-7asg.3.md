# `umber2-7asg.3`: current-main memmove ownership audit

## Evidence boundary

This audit measures commit `e1f21f7275da208ddfc6b308027f9ddaa4e981a6`.
It uses the immutable arXiv `2606.12566` source rooted at
`target/umber2-66p0.8/authority-workload/source` in slot 2, packed distribution
root `721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, and the 123-key closure whose SHA-256 is
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The format object's SHA-256 is
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
`ArXiv.tex` is
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`.

The issue-private runner changes only the binary, output, cache, and probe
paths from the immutable invocation template. Both the uninstrumented control
and the interceptor census stopped on the exact command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`. The control measured
12.88 wall seconds and 319,828 KiB RSS. Interception deliberately replaces
libc's implementation with a scalar overlap-safe copy, so its 8.13-second wall
measurement is not performance evidence.

The build and measured commands were:

```bash
CARGO_BUILD_JOBS=4 cargo build --profile profiling -p umber --bin umber
cc -shared -fPIC -O2 -g -fno-builtin-memmove -Wall -Wextra -Werror \
  -o target/umber2-7asg.3/memmove_probe.so \
  target/umber2-7asg.3/memmove_probe.c -ldl
flock /tmp/umber-perf-host.lock \
  target/umber2-7asg.3/run-row.sh control-20m
flock /tmp/umber-perf-host.lock \
  target/umber2-7asg.3/run-row.sh census-20m
```

The interceptor records every exact `(return address, byte size)` pair and one
20-frame stack per return address. Its totals reconcile exactly to 51,947
out-of-line calls and 4,768,860 bytes, with zero caller-table and zero
size-table overflow. `ownership-raw.tsv` retains all 914 exact size rows and
190 symbolized callers under `target/umber2-7asg.3/`.

## Material ownership census

"Material" retains the prior audit's threshold: at least 1% of all calls or
bytes after grouping by the nearest concrete application owner. The six
lifecycle families below cover every thresholded owner, 49,694 calls (95.66%)
and 4,550,652 bytes (95.42%). Size lists are exact where short; longer lists
are losslessly recorded in `ownership-raw.tsv`.

| Lifecycle and concrete owner                                                                                             | Rust value and cause                                                                                                                                                                                                                               |  Calls | Bytes per call                                                               | Total bytes |                                   Weighted `memmove` cycles |
| ------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -----: | ---------------------------------------------------------------------------- | ----------: | ----------------------------------------------------------: |
| Format startup: `tex_state::format::validate_logical_rows`                                                               | Temporary `BTreeSet<(u8, &str)>` name and `BTreeSet<(u32, u32)>` cell indexes; container key/edge insertion shifts                                                                                                                                 | 30,386 | 8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 96, 120, 144, 168, 192, 216, or 240   |   2,529,712 |                                                   0 sampled |
| Hot execution: `EngineUsageRuntime::make_string`                                                                         | `BTreeSet<String>`; 24-byte `String` key and 8-byte edge insertion shifts, not semantic owner transfer                                                                                                                                             | 15,568 | 8--240; exact same discrete set as above                                     |   1,402,840 |                                                   0 sampled |
| Hot resource execution: `FontStore::intern`, font-definition `cold::apply`, and `CommandHostCapabilities::register_font` | 113 semantic transfers of 1,288-byte `LoadedFont`; 129 copies of the 504-byte payload inside 512-byte `FontResource`; remaining `FontKey`/`FontHashFragmentKey` and `BTreeMap<PathBuf, FontResource>` shifts                                       |    357 | `LoadedFont`: 1,288 × 113; resource payload: 504 × 129; map shifts: 0--4,608 |     274,648 |     17,433,933 below cold font apply; other sites 0 sampled |
| Distribution/resource startup: `DistributionResolver::resolve_batch_with_prefetch`                                       | `BTreeMap<String, SelectedDistributionRecord>` with 24-byte keys and observed 80-byte values, `BTreeMap<u32, Vec<String>>`, fetched-object map removal, and response/fetch vector growth                                                           |  1,245 | 0--800 in 4/8/24/80-byte layout multiples                                    |     112,516 |                                                  20,875,485 |
| Format startup: `FontStore::from_frozen`                                                                                 | 41 semantic transfers of 1,288-byte `LoadedFont`; remaining `FontKey` and 80-byte `FontHashFragmentKey` B-tree insertion shifts                                                                                                                    |    545 | `LoadedFont`: 1,288 × 41; map shifts: 8--560                                 |     111,872 |                                                   0 sampled |
| Hot input refresh: `World::record_input_dependency`, `register_input_probe`, and `take_registered_source`                | `Arc::make_mut` plus insertion in `BTreeMap<Arc<Path>, InputDependency>` where `InputDependency` is 56 bytes; `BTreeMap<String, FileEnquiryResource>` where the value is 128 bytes; 944 empty-leaf removals from `BTreeMap<u32, RegisteredSource>` |  1,593 | dependency: 8--560; probe: 8--1,280; retirement: 0 × 944                     |     119,064 | 112,825,681 for complete incremental-input refresh ancestry |

The runtime layout probe also records `SourceRegistration` at 96 bytes,
`CommandHostCapabilities` at 272, `ResourceResponse` at 232, `FetchRequest` at
88, `ObjectEntry` at 56, and VFS `FileRequestKey` at 32. These are explanatory
layout facts, not proposed replacement sizes.

The ownership distinction matters:

- `LoadedFont` and the `FontResource` payload are semantic ownership
  transfers. Their exact repeated fixed sizes are not capacity growth.
- Format validation and the string pool are B-tree leaf/internal shifts.
- Distribution resolution mixes B-tree insertion/removal with ordinary vector
  growth during a resource boundary; it is not steady-state command delivery.
- Input refresh mixes a real COW semantic root transfer at `Arc::make_mut` with
  B-tree shifts and empty removals. Treating the complete row as container
  growth would hide its ownership clone.

## Independent CPU attribution

The independent `umber2-7asg.4` capture uses the same commit, immutable inputs,
fuel boundary, frame pointers, and `cycles:u` event. It collected 1,610 samples
with zero lost and 19,196,380,076 weighted cycles. Libc
`__memmove_avx_unaligned_erms_rtm` owns exactly 1,438,339,741 self cycles
(7.49%); its rounded inclusive share is 7.84%, approximately 1.505 billion
cycles. The profile binary and `perf.data` SHA-256 values are
`9ed990aea7d86083c2d03f30d54dfc00b0cbad27f272bc664fe8318e66af50a7`
and `9c0c95319c6f3413ab64fa0876718c1cf538e9beecfc9dbb16bdd785dee119d2`.

Disjoint immediate callers above 1% of `memmove` self weight are:

| Immediate caller                    | Samples | Weighted cycles | `memmove` self share | Cause class                            |
| ----------------------------------- | ------: | --------------: | -------------------: | -------------------------------------- |
| `execute_direct_episode`            |      22 |     263,766,548 |               18.34% | hot semantic/DTO transfers             |
| recursive `BTreeMap::clone_subtree` |      16 |     180,523,342 |               12.55% | semantic COW ownership clone           |
| `prepare_operation`                 |      14 |     171,931,124 |               11.95% | hot semantic/DTO transfers             |
| `refresh_candidate_files`           |       9 |     102,545,389 |                7.13% | input ownership refresh                |
| `execute_operation`                 |       8 |      98,784,889 |                6.87% | hot semantic/DTO transfers             |
| `scan_toks_buffers`                 |       7 |      84,916,781 |                5.90% | scanner scratch shifting/growth        |
| `expand_with_trace`                 |       5 |      61,661,763 |                4.29% | expansion scratch/ownership transfer   |
| libc `realloc`                      |       5 |      61,176,957 |                4.25% | container growth                       |
| `complete_boolean`                  |       4 |      48,581,523 |                3.38% | conditional scratch/ownership transfer |
| `scan_csname_characters`            |       3 |      37,536,888 |                2.61% | scanner scratch growth                 |
| `complete_integer`                  |       2 |      24,776,916 |                1.72% | scanner scratch/ownership transfer     |
| `resume_if_csname`                  |       2 |      24,729,327 |                1.72% | conditional scratch/ownership transfer |
| `expand_expandafter`                |       2 |      24,670,816 |                1.72% | expansion ownership transfer           |
| `BlobStore::load`                   |       2 |      20,875,485 |                1.45% | distribution/cache startup             |
| `CommandContext::internal_integer`  |       2 |      19,470,570 |                1.35% | hot command-state transfer             |
| bincode `deserialize_seq`           |       2 |      16,194,186 |                1.13% | format startup container growth        |

Sampling and exact interception answer different questions. A zero in the
material census table means the zero-loss profile took no `memmove` sample
below that concrete stack, not that its exact calls cost no cycles. Conversely,
the broad inlined episode callers do not provide exact call or byte counts and
must not be relabeled as one census owner. This audit therefore keeps the
exact call/byte table and disjoint sampled CPU table separate rather than
fabricating a join.

## Follow-up ownership

The existing `umber2-5ane` now carries the current format-validation counts,
and `umber2-7asg.1` already owns the prepared-operation transfer. This audit
files `umber2-7asg.6` for control-sequence string-table shifts,
`umber2-7asg.7` for `LoadedFont`/`FontResource` transfers, and
`umber2-7asg.8` for incremental-input COW and capability traffic. No
production representation or behavior changed in this audit.
