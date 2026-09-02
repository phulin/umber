# `umber2-66p0.8.40.144`: one resident macro-argument advance

## Measured redundant owner

The integrated `.140` profile remains the corpus authority: commit
`2a6e6bcf3345769b975088ef27561ef257d4ab7e`, exact command vector
`(20000000,19907047,2216877,6018482,16781922,4011)`, and exactly `7,922,897`
macro-argument deliveries. Its largest concrete CPU owner was
`advance_resident_command_into` at 21.54% inclusive / 13.27% disjoint self;
the byte tokenizer itself was only 0.93% self. The narrower current-tree mixed
resident gate reproduced `advance_resident_command_into` as the largest owner
at 17.61% disjoint self in 797 zero-loss samples over approximately 914 million
cycles. `MacroWordLane::get_sequential` was a separately visible 0.67% leaf.

The baseline annotation identifies redundant work immediately before that
leaf. One admitted macro-argument delivery first subtracted the range start to
reconstruct a relative position, branched on underflow, compared the same
absolute cursor with the range end, and called the lane. The lane then compared
the cursor with its total length, compared the saved provenance-run start with
the cursor, read a packed word and origin, and constructed a
`TracedTokenWord`. The caller immediately split that carrier back into its
`TokenWord` and `OriginId`. Admission had already validated the lower bound,
lane extent, and opening provenance run; the sealed cursor can only advance or
be restored with that run by rollback. The cursor's range-end comparison is
therefore the only ordinary per-word bound required.

This is distinct from the completed `.124` absolute argument cursor, `.125`
resident macro-body cursor, `.126` replay cursor, `.128` resident-top removal,
and `.1` fused input-stack cursor mutation. It does not reopen rejected
command-resolution candidate `.30`.

## Adopted boundary

`MacroArgumentCursor::advance_delivery` now performs the complete resident
operation:

1. compare its authoritative absolute coordinate with the admitted range end;
2. read separate packed word/origin parts using its admitted provenance run;
3. derive the pre-advance relative delivery position;
4. update the absolute coordinate exactly once; and
5. return `(position, word, origin)` directly to command admission.

The execution-scratch read no longer accepts a parallel range representation,
rechecks admission-time lower/lane bounds, or creates the combined traced-word
carrier. The impossible safe-lookup miss still follows the prior exhaustion
transition, so recovery behavior is not broadened. The row remains exactly the
absolute cursor plus provenance-run index needed for rollback; no cache,
retained borrow, source-kind specialization, or second cursor was introduced.

Ordinary file input still has its one lexer cursor and the separately semantic
input-frame position, while stored token lists, macro bodies, replay, and macro
arguments each retain their one authoritative cursor. Source and endline
transitions, alignment interception, provenance, recovery, suspension, and
rollback remain on their established cold/lifecycle boundaries.

## Exact warmed gate

The exact baseline executable was reconstructed from the same checkout by
reversing only this source patch and rebuilding the existing optimized
`mixed_macro_resident_pipeline` row. Each executable was warmed before its
measured run. The row performs 1,000,000 parameter deliveries and 1,000,000
empty-macro expansions while mixing macro body, argument, replay, and raw
resident input.

| Exact result                       |                          Baseline |                         Candidate |                 Delta |
| ---------------------------------- | --------------------------------: | --------------------------------: | --------------------: |
| User instructions                  |                     2,393,743,084 |                     2,352,740,452 | -41,002,632 (-1.713%) |
| User branches                      |                       398,193,382 |                       388,192,523 | -10,000,859 (-2.512%) |
| Allocation calls / requested bytes |                             0 / 0 |                             0 / 0 |             unchanged |
| Macro body / parameter deliveries  |             2,000,000 / 1,000,000 |             2,000,000 / 1,000,000 |                 exact |
| Replay / raw / expanded deliveries | 1,000,004 / 2,000,004 / 1,000,000 | 1,000,004 / 2,000,004 / 1,000,000 |                 exact |
| Macro expansions / command copies  |                     1,000,001 / 0 |                     1,000,001 / 0 |                 exact |
| Suspension in / out                |                             0 / 0 |                             0 / 0 |                 exact |

The exact instruction reduction is about 41 instructions per parameter
delivery. The resident function's symbol shrank from 11,597 to 10,833 bytes;
the out-of-line sequential lane leaf disappeared into the one direct path.
Cycles were noisy across captures and are not used as an acceptance claim.
The post-change 1,000-sample profile had zero lost samples and left resident
advance at 16.79% self; there is no longer a separate sequential-read leaf.

The exact public-copy interposer reports:

| API       |                  Baseline |                 Candidate |
| --------- | ------------------------: | ------------------------: |
| `memcpy`  | 142 calls / 353,141 bytes | 142 calls / 353,138 bytes |
| `memmove` |         2 calls / 0 bytes |         2 calls / 0 bytes |

Both reports reconcile with zero table collisions, overflow, or suppressed
probe-internal calls. The three-byte startup-layout difference is not on the
resident command path; neither build performs an attributed copy there.

Baseline and candidate binary SHA-256 values are respectively
`4b09517bb1ef9fecd375c1c99f4d3b179a0b36b5cc58dbd977a6c1dee8f05670`
and
`c9711ea187fe55f7a487907354366be64afd8768af8999889235cec07ca2635c`.
Ignored evidence is under `target/umber2-66p0.8.40.144/`; baseline and final
`perf.data` hashes are
`3d2e8ef179a5893d675aee9b27b23193156a28d619c8192ff554e87f5ab6cc8f`
and
`5b5dc50b7981a1babeb255ad99ee0b555191f9333e5ac6a1a17c924e0a534b81`.

## Semantic coverage

The command-core boundary gate now asserts that the macro-argument arm calls
the single cursor operation, never reconstructs `top.position()`, never splits
a traced word, and that the scratch method owns neither a range parameter nor
a traced-word return. Existing command tests exercise:

- physical line transitions, control sequences, comment/space state, enabled
  and disabled endline insertion, and direct-source exhaustion;
- backed-up input, stored token lists, macro bodies, macro arguments, and
  segmented replay with exact provenance;
- nested file pop/re-entry with exact line restoration;
- outer/runaway/EOF recovery and alignment interception;
- input/scanner resource suspension with one prefix replay; and
- transient and committed rollback of source, resident-frame, argument, and
  replay coordinates.

The executor suite additionally covers `\scantokens`, `\read`/`\readline`,
interactive source replacement, nested macro retirement, recovery, resource
resumption, and operation rollback.

Validation results:

- `cargo test -q --tests -p tex-command`: 391 unit and 23 boundary tests pass.
- `cargo test -q --tests -p tex-exec`: 760 unit tests pass with two ignored,
  plus four main-control and 24 external boundary tests.
- `cargo test -q --tests`: complete routine workspace suite passes.
- `scripts/check.sh`: all four gates pass; both Clippy resolutions are clean
  across 32 workspace members.
