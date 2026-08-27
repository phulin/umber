# `umber2-66p0.33`: one macro argument scratch owner

## Evidence boundary

The paired comparison uses the authenticated arXiv `2606.12566` workload from
the integrated command-core profile: source archive SHA-256
`05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`,
selected `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
packed distribution root `721e833071d92bba`, `manifest-v8.json` SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`,
and ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

Both executables use `RUSTFLAGS='-C force-frame-pointers=yes'`. The baseline
is exactly `ef836b42571dbf88098699d31eab2b2b039621e8`; its 386,004,488-byte ELF
has build ID `8a2df7cae597f610d8088c86b1b09427e5a9a59b` and SHA-256
`1c5049f77c30039b09deb53d6c14647ced66a72aba9186433a96ff8d60c90935`.
The candidate is exactly `5557ff10ad35df685ddfb17f5fe834a14d995311`;
its 387,091,152-byte ELF has build ID
`e50795c81efd28de3ccb9fde4b8a9c88dc184754` and SHA-256
`17bfe53e5ab543ef3575e73736ef5d7cce0c43fd873995d723a47aeb4e39f705`.

One outer `flock /tmp/umber-perf-host.lock` covered the final paired control,
perf, and census rows. CPU `some` and `full` pressure had `avg10=0.00` before
and after every accepted row and at both outer boundaries. Saved accepted-row
process censuses contain no Cargo, rustc, Umber, perf, or ansible peer. The
runner explicitly rejected one candidate census whose own completion left
nonzero CPU pressure, then waited without measuring through scheduled host
maintenance and accepted its retry after maintenance ended. Earlier
architecture iterations and ansible-overlapped windows remain explicitly
rejected issue-private evidence under `target/umber2-66p0.33/`; none is
promoted here.

## Architecture deletion

Macro match admission now initializes the one canonical `MacroSlot` which
will become the activation's sealed frame. Its fixed nine argument entries own
the current slot cursor, relative word range, §394 paragraph fact, and exact
outer-group progress while collection appends directly to that frame's
intrusive segment chain. Completing an argument finalizes that existing entry;
committing a match changes only the frame role and live depth. No argument
table, range, fact, segment owner, or token word moves during sealing.

The former `PendingMacroMatch`, match-segment vector, delimiter-segment vector,
absolute live bump stack, segment watermark, spare-segment vector, and repeated
serial/range validation are deleted. One stable generation-owned segment arena
serves pending frames, active frames, and the temporary delimiter prefix.
Frame or delimiter retirement returns its disjoint chain to one intrusive free
head. A pending child therefore remains canonical when an older active frame
retires beneath it; no cache, fast path, heap indirection, compaction, second
scratch owner, or new lifetime machinery is present.

Sealed parameter replay carries only `(frame, argument slot)`. The frame's
runtime serial rejects stale ABA reuse, the slot selects its range without a
linear search, and a sequential iterator follows each 4,096-word segment link
once. Warmed same-depth replacement performs zero allocations and copies zero
macro words.

## Exact 20M result

Every accepted row intentionally stops with status 1 at the exact fuel limit
and reports the identical vector
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
deferred-write expansions. Standard output is empty, no partial PDF or input
receipt is published, and no warmed row acquires a resource.

| Row                          | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| ---------------------------- | -------: | -------: | ---------: | -------------: |
| Baseline control             |     7.08 |     7.78 |       0.77 |        296,756 |
| Candidate control            |     7.16 |     7.82 |       0.76 |        296,728 |
| Baseline `cycles:u`          |     8.23 |     7.87 |       1.08 |        296,844 |
| Candidate `cycles:u`         |     8.27 |     8.33 |       1.10 |        296,604 |
| Baseline public-copy census  |     8.48 |     9.28 |       0.94 |        336,096 |
| Candidate public-copy census |     8.99 |     9.75 |       1.00 |        335,588 |

The control rows are flat at this granularity: candidate wall is 0.08 seconds
higher, user time is 0.04 seconds higher, system time is 0.01 seconds lower,
and peak RSS is 28 KiB lower. The perf and copy rows are attribution evidence,
not latency controls.

## Zero-loss absolute cycles

The baseline 199 Hz capture contains 1,349 samples and 16,792,121,581 weighted
cycles. The candidate contains 1,433 samples and 17,893,709,113 weighted
cycles. Neither raw stream contains a `PERF_RECORD_LOST` event; both
`perf script` and raw-dump error files are empty. The `perf.data` SHA-256
values are
`67a90c063d760315bbbb8125b9fe6237fdfb83106c49e23e47d642f64b452656` for
the baseline and
`e6180e2f691bdf690aaf856689f5650b4aeaf3c1a0a27a99d8bd4698cd42fb44` for
the candidate.

| Macro argument scratch union |    Baseline |   Candidate | Absolute change | Relative change |
| ---------------------------- | ----------: | ----------: | --------------: | --------------: |
| Self cycles                  | 765,440,131 | 691,629,996 |     -73,810,135 |          -9.64% |
| Complete ancestry cycles     | 778,834,756 | 717,751,887 |     -61,082,869 |          -7.84% |

The union is the non-additive whole-engine ancestry of
`ExecutionScratch::push_match_word`, `sealed_argument`,
`commit_macro_match`, and `argument_word_facts`. Both absolute measures fall
materially while the ordinary control stays flat; the architecture is
therefore retained. The higher total weighted period in the candidate sample
is not relabeled as a latency result: frequency sampling varies in total
weight, and the direct control is the timing authority.

## Deterministic public-copy census

Both exact size tables report zero caller and size overflow. Candidate public
`memcpy` calls fall by 423,622 (1.26%), while bytes rise by 15,101,310 (0.34%);
public `memmove` is byte-for-byte unchanged. The targeted result is exact:
`commit_macro_match` owns 684,496 baseline `memcpy` calls and 10,743,024 bytes,
across sizes 0, 24, and 48, and is absent from the candidate census.

| Public API | Baseline calls | Candidate calls | Baseline bytes | Candidate bytes |
| ---------- | -------------: | --------------: | -------------: | --------------: |
| `memcpy`   |     33,535,478 |      33,111,856 |  4,457,107,922 |   4,472,209,232 |
| `memmove`  |         51,948 |          51,948 |      4,767,012 |       4,767,012 |

## Semantic verification

Focused coverage retains delimited and undelimited matching, overlapping
delimiter prefixes, outer-pair stripping, ordinary versus failed-prefix
non-`\long` paragraph classification, runaway pseudoprint, tracing, nested
activations, pending-child rollback, stale-frame rejection, strict LIFO
retirement, multi-segment replay, and 8,192 warmed same-depth replacements.
The focused `tex-command` suite passes 245 unit and 18 integration tests. The
complete `cargo test -q --tests` routine suite passes. `scripts/check.sh`
reports dprint, Biome, rustfmt, and both clippy-resolution passes clean.
