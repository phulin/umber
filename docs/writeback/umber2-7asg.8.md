# `umber2-7asg.8`: incremental input refresh ownership

## Ownership result

The virtual compile path now transfers immutable input bodies without
materializing another byte vector. `VirtualFile` already owns each body as an
`Arc<[u8]>`; the incremental session and revision candidate retain clones of
that same owner, and candidate initialization gives the owner directly to the
memory-backed `World`. The separately edited root remains owned by the
incremental source buffer.

The initial workspace is enumerated once. After a resource-response batch has
validated atomically, `VirtualCompileSession` registers only the request keys
whose bindings were newly admitted. An initial retained candidate routes that
binding to its private session; an accepted session receives it directly.
`refresh_candidate_files` now refreshes only the candidate VFS snapshot and a
pending edited root. It does not scan and rebuild the registered-input overlay.

This consolidates the default ownership path and deletes repeated state
handoff. It adds no cache, fast path, byte representation, heap allocation,
generation owner, lifetime mechanism, whole-engine scan, or compaction step.
The shared owner is the immutable allocation already required by `umber-vfs`.

## Semantic boundary

The path-keyed registered-input map remains independent from the candidate
workspace because accepted sessions need it to seed later checkpoint forks.
That per-path insertion is the necessary semantic root transfer. The measured
candidate retains exactly the baseline's 39 public `memcpy` calls and 237
public `memmove` calls attributed to `Session::register_input_file`; the
smaller shared-owner value reduces their bytes from 3,944 to 3,296 and from
12,224 to 10,248 respectively.

The command-side dependency and source-registration paths were not changed.
The exact census reproduces baseline and candidate counts and bytes for
`World::record_input_dependency`, input probes, and registered-source removal,
including required-read dominance over probes. Resource bindings are inserted
only after the complete staged response batch succeeds, so rejection still
restores the original workspace and registers nothing. A progressing retry
continues the same retained candidate, while the new binding is available to
future accepted-session candidates.

## Exact paired evidence

The authenticated arXiv `2606.12566` workload used the packed distribution root
`721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, and the ordered 123-key prefetch closure. The
base is `7021cc23eb5a48fac1b0bd5a8e449bf3920747b8`; the candidate is
`16ea8c76c5ab8cf9bcb03359a533a7b76958f9df`.

The base and candidate force-frame-pointer ELF SHA-256 values are respectively
`d0dd4f0c27c7770b0c88911908c1f7545ce8a3832cb256e77c5d44cc7742199f`
and `42e48ed783ded1918a2b31a0212c3bb78a359335fe55af4223539ce93a416c45`.
Every measured row ran under one holder of
`/tmp/umber-perf-host.lock`, with no Cargo, rustc, Umber, or perf peer in its
process census. The stricter control and perf rows each started at CPU pressure
`some avg10=0.00` and `full avg10=0.00`; both perf captures report zero lost
samples.

Every row stopped at the exact fuel endpoint and reproduced command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`. Standard output was empty,
no partial PDF was published, and distribution reads, validations, selections,
and bytes were identical.

| Quiet row            | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| -------------------- | -------: | -------: | ---------: | -------------: |
| Base control         |     8.54 |     9.20 |       1.00 |        326,308 |
| Candidate control    |     7.61 |     8.28 |       0.83 |        296,588 |
| Base `cycles:u`      |     9.26 |     9.40 |       1.20 |        326,820 |
| Candidate `cycles:u` |     8.28 |     8.32 |       1.13 |        296,924 |

The paired control reduces wall time by 0.93 seconds (10.9%), user time by
0.92 seconds (10.0%), and peak RSS by 29,720 KiB (9.1%). Timing is supporting
evidence; the exact copy census and zero-loss cycle ancestry identify the
removed owner.

| Public API | Base calls | Candidate calls | Call change |    Base bytes | Candidate bytes |    Byte change |
| ---------- | ---------: | --------------: | ----------: | ------------: | --------------: | -------------: |
| `memcpy`   | 35,364,432 |      33,574,753 |  -1,789,679 | 5,740,577,919 |   4,519,055,111 | -1,221,522,808 |
| `memmove`  |     51,947 |          52,071 |        +124 |     4,768,860 |       4,793,580 |        +24,720 |

The direct `register_incremental_inputs` owner falls from exactly 21,250
`memcpy` calls and 465,275,787 bytes to zero. Its baseline sizes range from 329
through 6,762,750 bytes and come from `file.bytes().to_vec()`. The candidate's
shared-World insertion has one 15-byte path copy and no file-body copy. Global
copy totals are reported as observed but are not assigned wholly to this source
change because rebuilding dependent crates can alter unrelated inlining.

| Zero-loss owner                        |    Base cycles | Candidate cycles |         Change |
| -------------------------------------- | -------------: | ---------------: | -------------: |
| Complete run                           | 19,381,275,704 |   17,180,775,245 | -2,200,500,459 |
| Shared libc copy kernel                |  1,195,214,956 |      837,698,675 |   -357,516,281 |
| `refresh_candidate_files` ancestry     |    151,244,952 |                0 |   -151,244,952 |
| Copy kernel immediately under refresh  |     90,388,904 |                0 |    -90,388,904 |
| `register_incremental_inputs` ancestry |     36,862,920 |                0 |    -36,862,920 |

The candidate's residual workspace refresh is small enough to inline and has
no named sampled ancestry; critically, no shared-copy-kernel sample retains
refresh as its immediate parent. Complete cycles fall 11.4%, while shared
copy-kernel cycles fall 29.9%. Raw ELFs, row receipts, process and pressure
censuses, exact copy tables, symbolized stacks, and perf scripts remain ignored
under `target/umber2-7asg.8/`.

## Validation

- Focused `cargo test -q --tests -p tex-state -p tex-incr -p umber`: passed.
- Full `cargo test -q --tests`: passed.
- `scripts/check.sh`: all four gates passed, including both clippy resolutions
  and rustfmt.
