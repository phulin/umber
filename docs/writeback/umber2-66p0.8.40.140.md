# `umber2-66p0.8.40.140`: integrated post-definition CPU profile

## Authority and bounded result

Exactly one authenticated arXiv execution entered the engine on integrated
commit `2a6e6bcf3345769b975088ef27561ef257d4ab7e`. The force-frame-pointer,
full-debuginfo profiling binary has SHA-256
`cbead21f71d3a5fe3e92107d8cc91dfdf831d5b53005cb6786891e8615c14545`
and build ID `b9a08507a25dc0f30be56ac163f39b3d525e1395`; the checked copy
interposer has SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture used 199 Hz `cycles:u`, 8,192-byte DWARF callchains, and an 8 MiB
ring.

The workload was arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and TeX Live 2026 distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`. The ordered 123-key input closure has SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`;
the fixed source epoch was `1787080434`. The schema word, format hash, closure,
and archived authority receipt were validated before the run; no format,
distribution, corpus, oracle, or shared cache state was regenerated or
mutated.

The guards were 20,000,000 canonical command fuel, 40,000,000 executor steps,
45 seconds, and 1,536 MiB RSS. Expected status 1 occurred at the exact vector
`(20000000,19907047,2216877,6018482,16781922,4011)` for fuel charges, token
frame steps, expanded deliveries, meaning lookups, scanner tokens, and write
expansions. Raw source, stored-body, macro-argument, and synthetic-end-v
deliveries were respectively `463168`, `11520891`, `7922897`, and `91`; they
sum exactly to `19907047`. The run took 8.31 seconds wall, 7.79 user, and 1.15
system, with 164,796 KiB peak RSS. It produced 1,330 samples, zero lost
samples, and approximately 15,204,813,822 cycles. Fuel exhaustion published no
PDF artifact, as expected.

## CPU attribution without double counting

Self percentages below are disjoint sampled owners. Inclusive percentages are
overlapping ancestry and must neither be summed together nor added to self.

| Owner                                     | Inclusive |   Self | Interpretation                                                 |
| ----------------------------------------- | --------: | -----: | -------------------------------------------------------------- |
| `expand_classified_into`                  |    32.87% |  3.92% | Actual expansion dispatch self under broad expansion ancestry. |
| `execute_direct_episode`                  |    27.60% |  0.53% | Executor wrapper over delivery, scanning, and apply.           |
| `expanded_delivery_entry`                 |    27.23% |  1.53% | Entry/error/resume wrapper; most inclusive work is below it.   |
| `advance_resident_command_into`           |    21.54% | 13.27% | Dominant actual application self owner.                        |
| `raw_delivery_entry`                      |    19.92% |  2.67% | Entry/error/freshness wrapper around resident advancement.     |
| `Universe::with_command_context`          |    18.36% |  0.38% | Borrow wrapper over descendants, not 18.36% of facade self.    |
| `scan_toks_buffers`                       |    16.27% |  0.99% | Mostly delivery ancestry, not collector self.                  |
| `ExecutionScratch::append_argument_token` |     1.95% |  1.56% | Actual accepted-argument settlement self.                      |
| `next_compact_exact_byte_step`            |     2.60% |  0.93% | Tokenizer self plus its source-step descendants.               |

The public-copy probe itself accounted for 4.93% self, its `memcpy` wrapper
for another 0.70%, detailed command-fuel accounting for 0.73%, and the
profiling allocator for 0.61%. These are measurement costs, not production
recommendations. Likewise, the now-direct definition builder must not be
reopened merely because `scan_toks_buffers` has wide inclusive ancestry.

## Exact public-copy and allocation census

Exact attribution reconciled 6,754,872 `memcpy` calls / 817,413,380 bytes and
238,866 `memmove` calls / 40,758,998 bytes: jointly 6,993,738 calls /
858,172,378 bytes. `memcpy` had 24,161 collision probes with maximum probe two;
`memmove` had none. Both APIs had zero overflow calls, overflow bytes, and
probe-internal calls.

These totals are not all redundant representation. The largest `memcpy` row,
`ChunkStorage::release_lineage` at 2,445,622 calls / 410,864,496 bytes, and the
large line-break, page-arena, mode-list, durable-box, and discretionary rows
belong to active node/annex work and are excluded here. Miniz inflation's two
largest rows total 29,182,650 bytes of semantic decompression. Candidate
attachment parking/restoration accounts for 122 moves in each of three rows
and 4,277,686 bytes jointly; those are aggregate owner transfers, not copies of
the heap-backed semantic payload. Format/cache decoding, interning, and source
registration similarly materialize semantic ownership. By contrast, the
residual cold `CommandContext` `Result::expect` moves a 240-byte facade 33,886
times / 8,132,640 bytes, and `firm_up_the_line` converts a borrowed UTF-8 line
to an owned string 56,964 times / 1,842,726 bytes before a hook that only needs
`&str`; these are redundant representation candidates.

Named hot-core allocation scopes are disjoint innermost owners, so their sum is
the exact process census rather than inclusive ancestry:

| Allocation owner                |         Calls |    Requested bytes |
| ------------------------------- | ------------: | -----------------: |
| `delivery_and_scan`             |       245,075 |      8,579,437,547 |
| `semantic_apply`                |       594,825 |        333,830,944 |
| `evidence_publication`          |         1,138 |            815,729 |
| `cold_materialization`          |       179,066 |     17,040,680,100 |
| `attempt_scratch`               |           665 |          1,668,720 |
| all four remaining named owners |             0 |                  0 |
| **Exact total**                 | **1,020,769** | **25,956,433,040** |

Requested bytes measure allocator requests, not simultaneously resident bytes.
The same census recorded two interpreter constructions for 396,552 operation
entries; materializations were 46,410 expansion commands, 13,190 scanned
steps, and 147,938 prepared operations.

## Exactly three ranked recommendations

1. **Return one brace delta from macro-argument fact settlement.** Make
   `PendingArgumentFacts::settle` classify the literal catcode once and return
   `Open`, `Close`, or `Neither`; `append_argument_token` should update depth
   from that result instead of calling `spelling_is_end_group` after the
   opening test. This targets 1.56% append self plus the separately sampled
   0.51% closing projection and part of 0.25% fact settlement. Closed Bead
   `.131` removed the earlier duplicate _opening_ projection only; this is the
   measured remaining closing projection, not a re-proposal of that work.
2. **Extend single admitted command-context ownership through the residual cold
   scan branch.** Construct the facade inside the existing callback-scoped
   admission and pass reborrows through tracked-region observation and cold
   scan, eliminating the line-6629 `Result::expect` move without caching or
   retaining a borrow across suspension/resource boundaries. This targets
   33,886 exact 240-byte moves / 8,132,640 bytes and residual
   `command_context` self. Closed Beads `.39` and `.25` consolidated ordinary
   operation and hot apply/publication admissions; their receipts do not cover
   this surviving cold dispatch row.
3. **Borrow valid UTF-8 source lines through the TeX82 §363 pause hook.** Pass
   the borrowed `str` from `String::from_utf8_lossy` directly to
   `SourceStepQueries::firm_up_the_line`, allocating only for genuinely lossy
   invalid input or for a returned replacement. This deletes 56,964 exact
   copies / 1,842,726 bytes and their allocations without changing registered
   replacement ownership. No Bead matches `firm_up_the_line`; closed `.22` and
   `.28` instead tried shared ownership for checkpoint summaries and were
   rejected because they burdened every live line, which this borrowing change
   does not do.

The ranking is by likely CPU return on this capture, not by inclusive ancestry.
Active node/annex work and PDF parity/tooling are deliberately absent from the
recommendations.

## Evidence

Issue-private ignored evidence is under
`target/umber2-66p0.8.40.140/`. SHA-256 values for `perf.data`, raw copy data,
symbolized copy report, hot-core census, identity receipt, self report,
inclusive report, timing receipt, and engine stderr are respectively
`f99827a05debcfb2cdc8547959b5e6d6f481936d4c7efee4615bc06bb8b0fa3d`,
`71e44555fbb76e0a12d9ea0b2b25fb5819f07d0bd4da82e603e37f46b54cc6ed`,
`ca2286439058bad0aa328a7ca4c10651aad23f87c4a58cb97e9eeb8723e89f3b`,
`abc3fc89f9caed5ef011864fb482eec6bf63f8407593295ad3019460d05b7d2c`,
`886ef7dd92aa2a61106133c5d3a7b682856a6185102ca9590c1a1362f4d5dd63`,
`768be9845586a278bffe6576a7b4fecac8a5a5d3be848ea28496f42bbe1ec028`,
`7584dc090ed84a118870923568fb5a92e9d9d1da8329e1546d48c4ad0107e747`,
`643478984fd12223746ad126cf212f8e29635c3b4329b44167d5cec408cd706c`,
and `87ef2ad9720557175b1fdd202ecdfbfeca2ea5a31f7d2c6dd47637c061c60084`.
