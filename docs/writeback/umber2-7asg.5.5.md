# `umber2-7asg.5.5`: destination-directed caller map

## Evidence boundary

This inventory is read-only against
`fbeb606e7101a20e23de3a2ecd605e12daa52591`, before
`umber2-7asg.5.4`. The foundation API is therefore intentionally treated as
unsettled. Symbol and call counts below describe this base, not an API promise
to later implementation issues.

The semantic and ownership authorities are
[`tex_command_core.md`](../tex_command_core.md),
[`expansion_memory_lifetimes.md`](../expansion_memory_lifetimes.md),
[`alignment_brace_semantics.md`](../alignment_brace_semantics.md),
[`stepwise_execution.md`](../stepwise_execution.md), and
[`umber2-7asg.5.1`](umber2-7asg.5.1.md). The performance authority remains the
authenticated exact 20M run documented by `umber2-7asg.5.1` and
`umber2-7asg.5.3`. Its command-work vector is exactly
`(20000000,19913119,2218327,6020965,16785710,4011)`.

## Foundation-owned removal

All occurrences of the wide raw envelope are local to
`crates/tex-command/src/processor/next.rs`:

| Base symbol or operation      | Base location                                      | Foundation disposition                                                                                                                         |
| ----------------------------- | -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `take_input_token`            | line 1715                                          | Replace with the one fixed-frame, destination-directed raw operation.                                                                          |
| local `ActiveInput`           | lines 1716-1757 and match at 1773/1926             | Remove top-level source/token materialization; retain the storage-lifetime choice made when the input level was created.                       |
| `DeliveredToken`              | construction at 1822/1949/1964; definition at 2723 | Remove the 104-byte returned success value and write only present raw fields into call-local delivery storage.                                 |
| `get_next_canonical` unpack   | lines 1551-1635                                    | Parameter replay must restart before `CurrentCommand` construction; ordinary delivery continues into the caller-owned destination.             |
| exact-loop boundary assertion | `crates/tex-command/tests/it/boundaries.rs:72`     | Update after the foundation lands so it proves one raw semantic loop and absence of the retired envelope instead of naming `take_input_token`. |

The foundation may consequently touch `processor/next.rs`, input-level/frame
helpers, their focused tests, and the packed cutover gate. Later slices must
start from the landed `.5.4` tree and must not preserve a compatibility copy of
the old envelope.

`CurrentCommand::resolve_into` is _not_ foundation removal. It remains the
separately attributable once-at-delivery resolver in
`crates/tex-command/src/command.rs:250`. It borrows the dense meaning row,
clones the one required macro owner into the completed command, and ends the
row borrow before any mutation, recovery, expansion, backup, execution, or
suspension. The test-only/convenience `resolve` wrapper and synthetic active
character construction are later expansion/raw-seam callers; they do not
justify moving resolution back into input storage.

## Exhaustive `CurrentCommand` ownership inventory

The base has 18 production source files with explicit `CurrentCommand`
references. The table includes ownership edges and delivery consumers even
where type inference hides the name or no conversion is required.

| Owner                                                                 | Material role and migration edge                                                                                                                                                                                                                                                                                                                                 |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tex-command/src/command.rs`                                          | Defines the 144-byte ephemeral value, direct resolver, exact delivery stamp, backup copy, alignment adjustment, direct-source provenance, and one-delivery `\noexpand` treatment. The value remains owned across mutable meaning changes; it is never durable or serializable.                                                                                   |
| `tex-command/src/processor/next.rs`                                   | Sole raw constructor and owner of raw observation, outer recovery, alignment adjustment, delivery freshness, backup, ErrorStop deletion/insertion, source-line capture, and provenance projection. `.5.4` removes its input envelope; `.5.10` handles residual raw/recovery callers after that landing.                                                          |
| `tex-command/src/processor/expand.rs`                                 | Owns the destination driver, value-returning conveniences, raw/expanded policy, expansion dispatch, pending observation, in-place settlement, `\expandafter` operands, synthetic active-character delivery, and the exact completed command moved into typed expansion suspension. `.5.6` owns this slice.                                                       |
| `tex-command/src/processor/mod.rs`                                    | Defines alignment-lookahead command outcomes, the nonsemantic `CommandDeliveryCursor`, `resume_current_command`, and access to the command retained by a pending expansion frame. It moves with `.5.6`.                                                                                                                                                          |
| `tex-command/src/state.rs`                                            | `PendingExpansion` retains one command plus its exact child destination; `CommandReplayDelivery` is the value-returning replay convenience. It moves with `.5.6`; neither type may become a mailbox or durable coordinate.                                                                                                                                       |
| `tex-command/src/macro_call.rs`                                       | Borrows the macro-call command, already supplies local destinations to five `get_token_into` calls, backs up exact deliveries during parameter recovery, and transfers matched argument storage. `.5.13` adapts only to the landed foundation.                                                                                                                   |
| `tex-command/src/scan_toks.rs`                                        | Retains collector command/operand fields across typed expansion suspension, owns `ScannedLeftBrace::Consumed`, and already uses destination calls at nine material fetch sites. One cleanup loop still uses value-returning `get_token`. `.5.13` owns it.                                                                                                        |
| `tex-command/src/conditionals.rs`                                     | Borrows conditional commands and already supplies destinations to five raw, one token, and two expanded fetches. It owns raw `\ifx` operands, relation scans, active-character settlement, `pass_text`, and `\tracingifs` order. `.5.9` owns foundation adaptation.                                                                                              |
| `tex-command/src/scanners/scalar.rs`                                  | Stores commands in reusable scalar continuation phases and radix/keyword state; has two value-returning raw-token and fifteen expanded-token calls. Exact terminator backup, optional-space absorption, and child suspension are inseparable. `.5.7` owns it.                                                                                                    |
| `tex-command/src/scanners/expression.rs`                              | Borrows completed commands for classification and uses two value-returning nonblank-expanded helpers. Parenthesis and arithmetic recovery belong with `.5.7`.                                                                                                                                                                                                    |
| `tex-command/src/scanners/font.rs`, `hyphenation.rs`, `token_list.rs` | Each uses a value-returning expanded fetch or nonblank helper. Their font backup, nonabsorbing hyphenation classification, and token-list assignment semantics belong with `.5.7`.                                                                                                                                                                               |
| `tex-command/src/scanners/structured.rs`                              | Retains one preamble-span expansion command, returns already delivered accent assignments, and owns definition/general-text/show/write/immediate/alignment/math/let probes. It has thirteen value-returning token calls, six expanded calls, one protected replay call, and four nonblank helpers. `.5.8` owns this file-only slice.                             |
| `tex-command/src/processor/alignment.rs`                              | Stores completed commands in alignment events and mutates only their typed delivery adjustment. It owns delimiter interception, closing-brace recovery, and exact undo. It moves with `.5.10`.                                                                                                                                                                   |
| `tex-command/src/processor/observe.rs` and `observation/mod.rs`       | Borrow the completed command to project canonical identity, spelling, semantic operand, delivery stamp, source origin, and direct provenance. These are cold/demand-selected views, not alternate command owners. `.5.10` proves their order but must not cache them.                                                                                            |
| `tex-exec/src/main_control.rs`                                        | Owns `OperationDelivery`, `PendingPreflightCommand`, pending prefix/operation scans, raw/settled/expanding handoffs, typed retry, replay completion, alignment delivery, main-loop lookahead, and thirteen explicit command clones at preflight/retry or multi-consumer seams. It has the hot remaining value-returning executor callers and belongs to `.5.12`. |
| `tex-exec/src/main_control/hot_apply.rs`                              | Borrows the already settled command while scanning fused hot operands. It moves with `.5.12` and must not acquire input ownership.                                                                                                                                                                                                                               |
| `tex-exec/src/main_control/cold/scan.rs`                              | Moves the delivered command into uncommon operation scans and borrows it only to detach origin/context. Two raw token calls and one expanded call remain value-returning. `.5.11` owns this disjoint file.                                                                                                                                                       |
| `tex-command/src/lib.rs`                                              | Re-exports the opaque ephemeral type. No separate migration owner.                                                                                                                                                                                                                                                                                               |

Test and measurement consumers are deliberately not production migrations:

- `command/tests.rs` pins resolution, ownership, semantics, and the 144-byte
  bound; `tests/ui/ephemeral_serialization.rs` proves the value cannot cross a
  durable boundary.
- `processor`, macro, scanner, snapshot, input, and executor tests exercise
  value-returning conveniences as public-boundary controls. They may remain
  where the final architecture intentionally keeps cold convenience wrappers.
- `benchmarks/tex-command/src/bin/packed_cutover_gate.rs` pins the command and
  status sizes, uses both convenience and destination calls, and owns the
  deterministic warmed-allocation rows. `command_allocations.rs` covers
  scanner, recovery, macro, source, and replay allocations.

## Delivery-call census at the base

The census excludes tests and benchmark binaries. A value-returning call is a
migration candidate only when its caller already owns the final command slot;
the canonical architecture permits cold convenience entry points over the same
driver.

| Call                          | Production consumers                                                                                                                                                                                                    |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `get_next`                    | `processor/expand.rs` 1.                                                                                                                                                                                                |
| `get_token`                   | `processor/expand.rs` 4; `processor/next.rs` 4; `scan_toks.rs` 1; `scanners/scalar.rs` 2; `scanners/structured.rs` 13; `tex-exec/main_control/cold/scan.rs` 2.                                                          |
| `get_x_token`                 | `processor/expand.rs` 3; `processor/next.rs` 1; `scanners/font.rs` 1; `hyphenation.rs` 1; `scalar.rs` 15; `structured.rs` 6; `tex-exec/main_control.rs` 6; `cold/scan.rs` 1.                                            |
| replay/alignment conveniences | `main_control.rs` has one `get_next_with_replay_completion`, one undefined-preserving expanded fetch, one `settle_current_command`, and one `get_x_alignment_delivery`; `structured.rs` has one protected replay fetch. |
| nonblank conveniences         | `structured.rs` 4, `token_list.rs` 1, `expression.rs` 2, and `main_control.rs` 6.                                                                                                                                       |

Existing direct-destination consumers are important negative controls:

| Direct call                                   | Production consumers                                                                                                                |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `get_next_into`                               | `conditionals.rs` 5 and `scan_toks.rs` 1, plus the convenience wrapper in `next.rs`.                                                |
| `get_token_into`                              | `macro_call.rs` 5, `scan_toks.rs` 6, and `conditionals.rs` 1, plus the convenience wrapper.                                         |
| `get_x_token_into`                            | `scan_toks.rs` 2 and `conditionals.rs` 2, plus the expansion convenience wrapper.                                                   |
| replay/preflight/main-loop/alignment `*_into` | The hot executor already uses one each in `main_control.rs`; their convenience wrappers live in `processor/next.rs` or `expand.rs`. |

No caller conversion may turn those local destinations into a root slot,
searchable result tape, destination enum inferred from call order, or
redispatch fallback.

## Shared seam order and invariants

Every migration issue depends on `.5.4` and preserves this order:

1. Charge monotonic command work, claim an executor-owned replay completion if
   ready, then inspect the current input frame.
2. Borrow the backing selected when the frame was created, read one packed
   word or tokenize one source token, and advance only that owner's cursor.
   Depletion retires the exact level before the enclosing level is read; file
   close and tracing-nesting output keep their existing retirement order.
3. A parameter marker pushes its argument range and restarts before a delivery
   stamp or final command is constructed.
4. An ordinary spelling receives one fresh `DeliveryStamp` and resolves once
   into the active caller destination. The dense meaning borrow ends here.
5. Record the token frame, perform outer-validity recovery, then classify and
   record the exact alignment adjustment.
6. An intercepted alignment delimiter is surfaced or starts its v-template
   without publishing a raw command observation. Every other delivery
   publishes the optional raw observation only after alignment handling.
7. Expanded settlement owns expansion trace, undefined recovery, macro call,
   pending expanded-observation commitment, and closing-brace classification.
   A terminal command moves to its final consumer or, only on a real immutable
   resource need, into its exact typed continuation.

Subsystem-specific invariants are:

- **Scanners:** status and warning identity enter and leave through one scanner
  episode; raw versus expanded fetch choice is semantic; keyword mismatch,
  optional-space absorption, and backup order are unchanged. A backup consumes
  the live delivery stamp and reverses its recorded `align_state` adjustment at
  most once.
- **Expansion:** active expansion depth balances on success, error, and
  suspension. A resumed primitive does not repeat its already emitted trace.
  `\expandafter`, `\csname`, collectors, and scalar children resume at their
  typed destinations deepest-first.
- **Conditions:** condition frames remain independent of input. `\ifx` keeps
  raw operands, active-character handling keeps its in-place `x_token`
  semantics, and `\tracingifs` prints at conditional entry and delimiter
  resolution through the existing diagnostic collector.
- **Alignment and recovery:** literal braces alone adjust the single command-
  owned `align_state`; delimiter interception precedes raw observation;
  preamble recovery retains `aligning` until the recovered frozen `\cr`;
  `off_save`, outer validity, ErrorStop deletion/insertion, and closing-brace
  recovery preserve their exact input and diagnostic order.
- **Executor:** §1038 main-loop lookahead remains raw for accepted characters;
  `goto reswitch` and §1270 handoffs dispatch the already delivered command in
  place and never substitute `back_input`. Mutation-free preflight and typed
  suspension move the command, delivery cursor, scanner child, and completed
  operands without redelivery.
- **Suspension and rollback:** call-local raw delivery storage never suspends
  and needs no rollback entry. Input position, source cursor, provenance
  watermark, replay-completion frontier, and scratch frames remain in
  `CommandState`. The observation cursor carries sequence only, never semantic
  or input ownership. Abort closes children before parents; rollback never
  refunds command fuel.
- **Tracing and provenance:** unobserved delivery constructs no record.
  Observation projects the completed command after semantic ordering is fixed.
  Direct source provenance is captured before source retirement; stored tokens
  keep their existing `OriginId`/optional exact provenance. No migration adds a
  provenance arena, cache, eager source resolution, or meaning classifier.

## Merge-safe implementation partition

All implementation slices are children of `umber2-7asg.5`, discovered from
this issue, and blocked on `.5.4`. They have disjoint production-file
ownership and can be dispatched in parallel only after the foundation lands:

| Issue              | Exclusive production ownership                                                                                 |
| ------------------ | -------------------------------------------------------------------------------------------------------------- |
| `umber2-7asg.5.6`  | `processor/expand.rs`, `processor/mod.rs`, and expansion-owned fields in `state.rs`.                           |
| `umber2-7asg.5.7`  | `scanners/scalar.rs`, `expression.rs`, `font.rs`, `hyphenation.rs`, and `token_list.rs`.                       |
| `umber2-7asg.5.8`  | `scanners/structured.rs`.                                                                                      |
| `umber2-7asg.5.9`  | `conditionals.rs`.                                                                                             |
| `umber2-7asg.5.10` | `command.rs`, `processor/next.rs`, `alignment.rs`, `observe.rs`, and `observation/mod.rs`, after `.5.4` lands. |
| `umber2-7asg.5.11` | `tex-exec/src/main_control/cold/scan.rs`.                                                                      |
| `umber2-7asg.5.12` | `tex-exec/src/main_control.rs` and `hot_apply.rs`.                                                             |
| `umber2-7asg.5.13` | `macro_call.rs` and `scan_toks.rs`.                                                                            |

`umber2-7asg.5.15` is the serial integration/profile task. It depends on all
eight slices and owns only the cross-crate boundary assertion, standalone
command benchmark, and final issue-scoped measurement writeback. The closed
`umber2-7asg.5.14` is an empty duplicate of `.5.13` created by a delayed
tracker batch; it owns no work.

## Acceptance and performance gates

Every implementation slice must run its focused crate tests, then the complete
routine suite with `cargo test -q --tests`, followed by `scripts/check.sh`.
Only `scripts/check.sh` may be reported as the format/rustfmt/clippy gate.

Every slice also preserves these exact cross-cutting results:

- authenticated work vector
  `(20000000,19913119,2218327,6020965,16785710,4011)`;
- byte/order command and corpus semantics, including rollback, replay
  completion, typed suspension, alignment/recovery, tracing, and provenance;
- no allocation after warmup for mixed replay, macro replacement, macro
  argument, attempt, and durable packed spans; and
- no new cache, inferred classifier, duplicate delivery loop, semantic owner,
  command clone used only as an API bridge, or raw slot crossing suspension.

The deterministic standalone checks are:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin packed_cutover_gate -- --mixed-stored-only
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin packed_cutover_gate -- --only=destination_directed_warm_delivery
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin packed_cutover_gate
```

The final `.5.15` profile must use the same authenticated source,
distribution, format, closure, environment, host serialization, fixed clock,
20M fuel boundary, and zero-loss accounting as `.5.3`. It reports absolute
weighted self and inclusive cycles for
`tex_command::processor::next::get_next_canonical` and separately for
`CurrentCommand::resolve_into`. Caller migration may not claim an inferred
resolution gain or hide a regression by reporting only percentages.
