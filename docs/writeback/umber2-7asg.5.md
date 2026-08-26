# `umber2-7asg.5`: canonical delivery completion audit

## Audit boundary

This is a fresh completion audit of the integrated production tree at
`e9f90b57b21043822d8917c9857c7f2776b3082a`. It derives the closure contract
from the parent bead, children `.5.1` through `.5.16`, their comments and
writebacks, [TeX command core](../tex_command_core.md),
[Expansion memory lifetimes](../expansion_memory_lifetimes.md),
[Alignment and brace semantics](../alignment_brace_semantics.md), and
[Stepwise execution](../stepwise_execution.md). The duplicate `.5.14` owns no
implementation. No production behavior or architecture changed during this
audit.

The earlier `.5.15` capture directly preserved its ELF and `perf.data`, full
CLI arguments, exact vector, zero-loss reports, and separate symbol-period
sums. Its retained receipt did not directly name the source working directory,
source hash, or fixed-clock environment, however. Those facts existed only in
the writeback. Because indirect evidence is insufficient for parent closure,
the completion audit repeated the control and caller/callee capture through a
non-overwriting issue-scoped runner under
`target/umber2-7asg.5-completion-audit/`. Each row records the binary, source,
closure, distribution manifest, and format hashes before execution, plus the
fixed environment. The verified row below is the final closure authority.

## Requirement-by-requirement proof

| Requirement                                                                 | Direct integrated proof                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Attribute canonical construction independently from resolution              | [`.5.1`](umber2-7asg.5.1.md) partitions all 2,215,870,854 original `get_next_canonical` self cycles and records exact delivery frequencies. [`.5.2`](umber2-7asg.5.2.md) separately partitions all 886,122,219 original `CurrentCommand::resolve_into` self cycles and its 1,031,349,285 inclusive period sum.                                                                                                                                                                           |
| Resolve once into an owned command without a cache or lazy reinterpretation | `command.rs` has one `resolve_into` implementation. `next.rs` invokes it once after parameter replay and before outer validity, alignment, or observation. Direct dense-row borrowing ends inside resolution; static meanings copy and a macro definition gains exactly one final owner. `delivered_command_keeps_the_resolved_meaning_and_exact_spelling` and `macro_delivery_carries_a_generation_typed_definition_coordinate` cover reassignment and owner lifetime.                  |
| One destination-directed raw input path                                     | `boundaries::raw_delivery_keeps_one_profile_shared_input_path_and_semantic_free_levels` proves exactly one `deliver_raw_input_into` and one `get_next_canonical`. Stored cursors use their admission-selected `PackedTokenSpanHandle`; source tokenization writes the same call-local `RawDeliverySlot`; parameter candidates restart before command construction.                                                                                                                       |
| No returned wide raw envelope                                               | Production contains no `take_input_token`, `ActiveInput`, or `DeliveredToken`. `RawDeliverySlot` is 88 bytes, owns no cursor or backing, and is discarded before any return or suspension. `DeliveryStatus` contains no command-bearing payload.                                                                                                                                                                                                                                         |
| No compatibility hot path                                                   | `boundaries::migrated_production_delivery_callers_own_their_command_destinations` walks every production Rust file in `tex-command` and `tex-exec` and rejects all migrated value-returning call forms. Public conveniences are thin cold/test boundaries over the same driver and have no ordinary production caller. The sole undefined-preserving value convenience is pinned to `diagnostic_expand_step`, whose destination is selected only after diagnostic classification.        |
| No alternate loop, destination inference, or redispatch                     | The boundary suite proves one raw and one expanded policy loop, rejects destination inference/search and redispatch names across production, and rejects command-state mailboxes and result tapes. Typed child continuations carry explicit return destinations. Main-control `reswitch` and assignment handoffs consume the already delivered command in place.                                                                                                                         |
| Exact rollback and backup                                                   | Input positions, source cursors, replay-completion frontiers, provenance watermarks, and scratch frames remain in `CommandState`; the raw slot needs no rollback word. `destination_raw_delivery_mints_fresh_stamps_and_reverses_backup_once` proves a fresh delivery identity, stale rejection, and one alignment reversal. Snapshot replay-lane rollback and executor journal rollback tests pass. Fuel is monotonic and is not refunded.                                              |
| Typed suspension and deepest-first resumption                               | A real resource need moves one completed command into `PendingExpansion` beside one typed child destination. Resume consumes the exact ABA-tagged chain; abort closes child before parent. Focused expansion, nested scanner, `expandafter`, `csname`, alignment-preamble, preflight, and resource-retry tests pass. No `RawDeliverySlot` or inferred owner crosses suspension.                                                                                                          |
| Alignment and recovery order                                                | `get_next_canonical` records the token frame, performs outer validity, classifies the one command-owned `align_state` adjustment, intercepts delimiters, and only then publishes an ordinary raw observation. The boundary suite proves one outer/runaway recovery table and one alignment classifier. Focused stale-backup, nested alignment, v-template, closing-brace, `off_save`, ErrorStop, and runaway tests pass.                                                                 |
| Conditional, scanner, macro, and collector semantics                        | Conditional operands retain raw versus expanded policy and independent frame identity. Leaf and structured scanners retain optional-space absorption, two-level keyword rollback, alphabetic-constant brace correction, expression recovery, accent/math in-place handoff, and exact child phases. Macro parameters restart before resolution; macro argument and `read_toks` ownership tests pass.                                                                                      |
| Provenance and tracing order                                                | Direct source provenance is captured before retirement and borrowed from the final command for observation; stored input keeps its existing origin and optional exact provenance. Raw observation follows alignment and intercepted delimiters remain unobserved. Command, macro, conditional, assignment, replay, and resource-resume trace-order tests pass. No delivery cache or provenance arena was added.                                                                          |
| Zero warmed allocation                                                      | Fresh locked `--mixed-stored-only`, `--only=destination_directed_warm_delivery`, and complete `packed_cutover_gate` runs pass. `warmed_mixed_stored_cursor` and all 24,576 raw non-creating, raw creating, and expanded destination-directed calls report zero allocation calls and zero requested bytes. The complete gate reports zero for source, backup/replay, every stored owner, long macro arguments, control-sequence delivery, macro matching/expansion, and keyword rollback. |
| Exact semantics and corpus coverage                                         | Fresh focused `tex-command`, `tex-exec`, and `tex-command-stream` suites pass. Their active cases cover the command-semantic fixtures, corpus/channel comparison, rollback, typed retry, alignments, recovery, provenance, and tracing. The complete `cargo test -q --tests` result and `scripts/check.sh` gate are recorded at issue closure.                                                                                                                                           |

## Authenticated final profile

Both verified rows use the same current force-frame-pointer ELF, source,
schema-12 format, packed distribution, 123-key closure, offline policy,
`SOURCE_DATE_EPOCH=1787080434`, `FORCE_SOURCE_DATE=1`, `LC_ALL=C.UTF-8`,
20,000,000 command-fuel limit, 45-second guard, 1,536 MiB RSS guard, and host
lock. The recorded SHA-256 identities are:

| Input                          | SHA-256                                                            |
| ------------------------------ | ------------------------------------------------------------------ |
| final ELF                      | `cb8b5bffe3124300ed5ca1aaca538b5df5d001f9ce0a8cfc63eca6c0f4b57a40` |
| arXiv `2606.12566` `ArXiv.tex` | `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537` |
| 123-key closure                | `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e` |
| packed distribution manifest   | `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9` |
| schema-12 `pdflatex.fmt`       | `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4` |

The warmed control and perf row intentionally exit 1 at exact fuel exhaustion
and both reproduce
`(20000000,19913119,2218327,6020965,16785710,4011)`. The 199 Hz `cycles:u`
caller/callee capture contains 1,586 samples, zero lost samples, and exactly
18,907,770,993 weighted user cycles. Its `perf.data` SHA-256 is
`25af075096b260fb43c4bddc6e4cd988cf46c386fee755ba0d46d8a6e0c3246d`.

| Final integrated symbol        | Self samples | Weighted self cycles | Inclusive samples | Weighted inclusive cycles |
| ------------------------------ | -----------: | -------------------: | ----------------: | ------------------------: |
| `get_next_canonical`           |          148 |        1,797,796,458 |               418 |             5,087,487,398 |
| `CurrentCommand::resolve_into` |          102 |        1,243,975,680 |               115 |             1,402,517,461 |

These are disjoint symbol reports. `get_next_canonical` inclusive already
contains resolver descendants, so the two rows are never added. Against the
authenticated `.5.3` pre-destination baseline, canonical self work is lower by
477,884,782 weighted cycles, or 21.00%, while inclusive work is higher by
66,762,409 cycles, or 1.33%. Resolver self and inclusive work are separately
higher by 230,520,178 and 229,560,563 cycles. The audit therefore proves the
structural normalization reduction and reports the combined inclusive and
resolver regressions without hiding or reallocating them.
