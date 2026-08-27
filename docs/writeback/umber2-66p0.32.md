# `umber2-66p0.32`: owned input frame transitions

## Evidence boundary

The paired comparison uses the authenticated arXiv `2606.12566` workload from
the integrated command-core profile: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
packed distribution root `721e833071d92bba`, and `manifest-v8.json` SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`.

Both executables use `RUSTFLAGS='-C force-frame-pointers=yes'`. The baseline
ELF is 387,008,440 bytes, has SHA-256
`26b094d014af0ff49f39727f3ece9a5b0c8dbf6f9d637abe2d037e256fe56e`,
and is the production-identical `3a91f1d26` authority. The candidate ELF is
386,151,104 bytes, has SHA-256
`f9f209eff917a28d32f801a1f280d9e4a749a73f636725c24ee1960acc1e5bf6`,
and build ID `5202d8398cc08c5dd273c7a4e88cc37effe66a9f`.

One outer `flock /tmp/umber-perf-host.lock` covered the final paired window.
Every accepted row began and ended with CPU `some` and `full` pressure
`avg10=0.00`; the window process receipts contain no Cargo, rustc, Umber, or
perf peer. A first attempt is retained as explicitly rejected issue-private
evidence because its cold cache warm-up left nonzero CPU pressure at later row
boundaries. All evidence, runners, binaries, and analysis remain ignored under
`target/umber2-66p0.32/`.

## Architecture deletion

`CommandUsageTracker` and its shared `Arc<Mutex<CommandStackUsage>>` and
`Arc<AtomicUsize>` owners are deleted. The existing singular live
`CommandState` owns TeX82's three maximum scalars and terminal-buffer width
directly. Source and token levels pass through one `push_input_level`
transition, which updates `max_in_stack` and publishes the frame without a
lock or helper call. Runtime maxima remain outside snapshot roots, so rollback
does not refund them.

Nested `\input` and `\scantokens` use one processor-owned ancestry capture and
hand the completed `SourceOpenDepths` owner to source construction. The new
source frame therefore becomes visible with its `\tracingnesting` record
already installed. Retirement pops the validated top source and moves that
same optional boxed owner into `InputRetirement`; `file_warning` consumes its
value. The post-open identity walk, pre-retirement identity walk, and clone of
both boxed ancestry slices are deleted. Root, terminal, and `\read` source
frames retain `None`, preserving the 96-byte `InputLevel` layout and adding no
allocation, cache, indirection, representation, or lifetime owner.

Focused tests prove that source retirement retains the exact two boxed-slice
addresses and that the canonical push maximum survives rollback of the
aggregate command roots. Existing suites retain source framing, `\everyeof`,
`\endinput`, v-template and macro retirement, tracing-nesting warnings,
suspension, snapshot, and output semantics.

## Exact 20M result

Every control and perf row intentionally stops with status 1 at the same fuel
boundary and reports the exact vector
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Standard output is empty and no partial PDF is published.
No warmed row acquires a resource.

| Row                  | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| -------------------- | -------: | -------: | ---------: | -------------: |
| Baseline control     |     7.56 |     8.22 |       0.81 |        325,664 |
| Candidate control    |     7.59 |     8.26 |       0.83 |        318,828 |
| Baseline `cycles:u`  |     8.27 |     8.56 |       1.12 |        326,028 |
| Candidate `cycles:u` |     8.23 |     8.40 |       1.16 |        319,236 |

The control rows are effectively flat at this granularity; the candidate's
peak RSS is 6,836 KiB lower. The cycle captures provide the owner-specific
acceptance evidence rather than relabeling the 0.03-second control difference
as a latency claim.

## Zero-loss absolute cycles

The baseline 199 Hz capture contains 1,489 samples and 17,575,384,772 weighted
cycles. The candidate contains 1,468 samples and 17,257,351,733 weighted
cycles, 318,033,039 fewer (1.81%). Neither raw stream contains a
`PERF_RECORD_LOST` event and both symbolization stderr files are empty.

| Owner union or leaf                      | Baseline self | Candidate self | Baseline ancestry | Candidate ancestry |
| ---------------------------------------- | ------------: | -------------: | ----------------: | -----------------: |
| `CommandUsageTracker::record_input_push` |   121,616,188 |              0 |       121,616,188 |                  0 |
| `CommandState::source_open_depths`       |   229,602,742 |              0 |       229,602,742 |                  0 |
| `retire_and_restart`                     |   159,082,685 |     71,298,308 |       627,550,590 |        275,893,685 |
| non-additive three-owner union           |   510,301,615 |     71,298,308 |       761,188,010 |        275,893,685 |

The disjoint self union falls by 439,003,307 cycles (86.03%), while complete
union ancestry falls by 485,294,325 cycles (63.75%). The two deleted owners
have no candidate symbol; `push_input_level` and ancestry capture inline into
their singular callers. Static inspection also finds no
`CommandUsageTracker`, `record_input_push`, or `source_open_depths` symbol in
the final candidate ELF.

## Verification

The focused `tex-command` suite passes 244 unit and 18 integration tests. The
complete `cargo test -q --tests` routine suite passes. `scripts/check.sh`
reports all dprint, Biome, rustfmt, and both clippy-resolution gates clean. No
follow-up issue was discovered.
