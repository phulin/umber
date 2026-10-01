# Minimal hot loop

The current packed mode/command/prefix implementation is mapped in
[Main-control dispatch](main_control_dispatch.md). Its eligibility lane,
operation unions, phase requests, and per-command lifecycle carrier are
removed. Earlier measurements below describe the pre-replacement source;
they are informative evidence, not current source maps or closure gates.

Status: revised implementation target, reconciled against `eb754fc2d` on
2026-10-01. The original measurements use the frozen binary from the
`23548ce50` full-book profile; the source was read at `ea2b720e6`. Tracking:
Beads `umber2-efje` (design) and `umber2-i0n5` (implementation). The
reconciliation below controls the remaining implementation order and
supersedes the original migration sequence and any storage detail it marks as
measurement-dependent.

This proposal replaces the input, expansion, and main-control core with a
machine shaped like TeX's own:

- direct immutable token spans and one retained argument store;
- dense packed meanings;
- compact frame-owned reader positions;
- a direct TeX save stack and checkpoint first-touch history.

Checkpoints, resource replay, provenance, observation, and fuel move off the
per-token path. They attach to boundaries the engine already has: frame pushes,
command dispatch, and quiescent checkpoints.

It subsumes the first and third proposals in
[Book performance refactors](book_performance_refactors.md) and absorbs the
second. Those proposals keep today's owners and thin them. This one removes
them.

## Reconciliation after the first implementation wave

This section controls implementation order.
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
is the sole authority for the selected storage, mutation and lifecycle model. The proposal's original
two-ended token heap, reference-count rollback epochs/quarantine, and blanket
deletion list are retired. Compact frame-layout experiments are not prerequisites. The implementation through `eb754fc2d` has
already removed several named owners, and the next changes must follow the
measured costs that remain. Every replacement deletes its predecessor in the
same logical stage; no compatibility format, API shim, feature fallback, or
second engine is retained.

| Original stage                   | State at `eb754fc2d`                                       | Reconciliation                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 0. Harness                       | Complete in `b5dc8429c`                                    | Keep the original case table and thresholds. Receipts are per-revision evidence.                                                                                                                                                                                                                                                                                                           |
| 1. Halt and rollback             | Implementation complete; performance gates carried forward | Sticky halt and checkpoint replay replaced command rollback. The old `roll_attempt_operation` path is deleted. `settle_attempt` remains a boundary check that clears command scratch; do not schedule deletion of the already-removed scope protocol. The original `group` and `toks` measurements missed their intermediate limits and remain open performance evidence for later stages. |
| 2. Transient checkpoint state    | Partial in `0552fe902` and `270b0ef91`                     | Journals and transient snapshots were deleted and named captures moved by value. Keep the existing full-checkpoint admission rules and verify all checkpoint kinds, replay, and incremental restore. The `mbody` intermediate limit has not been met.                                                                                                                                      |
| 3. Definitions and token storage | Partial in `73434d995`                                     | Definition bodies are shared immutable spans. Resident macro and durable-list rows retain those owners and read the spans directly; matched arguments remain transient words. The earlier two-ended `mem`, reference-count rollback epochs, quarantine and interval allocator are retired. Address global-row reclamation separately from active-frame ownership.                          |
| 4. Frames and delivery           | Partial in `eb754fc2d`                                     | The six-domain replay lane was replaced by one token stack, and immutable definition/list activation no longer copies payloads. Frames and delivery are still larger/richer than the proposed machine. Finish one-pass argument construction, matcher plans, packed delivery, and meaning resolution as separate measured changes.                                                         |
| 5. Main control                  | Not started                                                | Keep dense mode/command dispatch and direct prefixed-command execution as a later stage, after the packed delivery seam is stable.                                                                                                                                                                                                                                                         |
| 6. Code layout                   | Not started                                                | Apply cold attributes, generic removal, PGO, and post-link layout only against the hot symbols and full-book profile of the then-current tree.                                                                                                                                                                                                                                             |
| 7. Node memory                   | Not started                                                | Retain the existing node/checkpoint contract. Move storage to epochs only if profiling shows node allocation or page/box journals still block the full-book target.                                                                                                                                                                                                                        |

The immutable-span stage replaces macro-body and durable stored-list copies
with direct input-frame reads. The input stack retains the shared owner for the
frame lifetime and releases it through LIFO row retirement or aggregate
checkpoint replacement. This preserves the existing named-checkpoint
admission rule: live macro activations are not eligible checkpoint contents.
Generic capture code being able to clone a macro row does not authorize a
checkpoint taken with one active. Do not narrow checkpoint coverage to hide a
missing admission check. Keep checkpoint and resource replay ownership on the
existing aggregate checkpoint path; do not add token-specific checkpoint
history or a new allocator chronology.

Immutable span delivery and packed delivery are one replacement stage. Active
rows own their immutable bodies and stored lists directly. The reader passes
packed token and origin scalars to a statically selected consumer, without
resident token/result carriers. That consumer resolves into the final native
`CommandWord`: static word, macro key and flags, or font capability. Fonts
belong to that meaning value, so commands keep no second font column. Cold
diagnostic, observation, scanner and execution boundaries construct their
required command from the final value. Performance receipts describe this
implementation; the numerical targets remain goals, not closure blockers.

Next, construct macro arguments once in their final retained storage while
matching. Do not build an argument in command scratch and copy it again into
the input stack. Compile each definition's parameter text into an immutable
matcher plan when the definition is sealed; delimited-argument failure tables
belong to that plan, not to a per-call matcher. The plan must implement TeX82
§§392--399, including leading literals, undelimited arguments, brace trimming,
paragraph and outer checks, and delimiter recovery.

The sealed definition owns one shared payload containing the packed parameter
and replacement words, definition origin, and `MacroParameterPattern`. The
pattern retains its validated marker index and compiles packed-word delimiter
failure links during publication, including direct scans, memo import, and
format materialization. Promotion shares this same payload. A resident macro
body retains the payload through matching and replacement delivery, so retiring
a definition row cannot invalidate its matcher.

Calls borrow the pattern and each delimiter's word/failure slices directly.
They construct no invocation plan and allocate no failure table. Failure links
only select the longest reusable literal prefix: the call-local argument cursor
still owns held source origins, brace depth, paragraph checks, outer recovery,
and group trimming under TeX82 §§392--399. Empty and single-word delimiters
need no failure allocation. Arguments are constructed in their final pooled region and published by
coordinate, without a scratch-to-input transfer.

### Final macro argument regions

`InputStack` owns one reusable `ArgumentArena` pool. Each simultaneous
invocation uses one vector for all of its arguments; individual arguments are
half-open ranges, never payload allocations. A local match holds at most nine
ranges and one local scanner cursor for the current argument. Parameter rows
borrow the region owned by their macro body. Definition delimiter plans remain
borrowed immutable metadata.

Capture writes directly into this final vector. A delimiter candidate occupies
an unpublished suffix: mismatch reveals words in place, a completed delimiter
truncates them, and outer-brace removal narrows a range. Publication appends
only the ranges to `param_stack` and transfers the region to the body, without
copying or materializing its words. Distinct live source and destination
regions allow forwarded runs to borrow two vectors safely while appending once.

Raw matching never expands another macro. Alignment interception pushes a
template for the same raw reader; outer and EOF recovery push backup/frozen
input without invoking expansion. Parents can nevertheless retire during a
match. Separate region lifetimes let retirement return the parent's vector
without truncating the child's unfinished capture or backed-up words. Tail
chains alternate two reusable slots rather than retaining dead prefixes.

One optional capture region records the unfinished owner for cleanup,
quiescence, and TeX memory accounting. One result epilogue restores the shared
scanner status and releases any capture not transferred to a body, including
errors from stack conservation after successful scanning. Abort, empty-body
completion, body retirement, and restore each release their exact owner.
There are no match serials, pending capabilities, leases, or argument undo logs.

Named checkpoints admit no macro frames or captures. Their input copy excludes
vacant vector pools; aggregate restore clears abandoned regions while retaining
the live engine's reusable capacity. Physical pool capacity is charged once to
that engine. Semantic equality and hashing ignore vacant slots and capacities.
The existing aggregate replay path remains the sole restart authority.

The combined immutable-span and packed-delivery replacement resolves each
interpreted token once into the final native meaning. Ordinary unobserved
macros activate directly; actual scanner, execution, observation, alignment,
backup, or recovery consumers construct their required command. The previous
`ResidentWord`/`TokenMeaning` materialization chain is deleted. Direct
mode-by-command main-control dispatch follows matcher and argument storage
work. No adapter keeps the previous reader alive.

Retention follows semantic owners: definitions and stored lists, active input
frames, TeX save entries, and retained aggregate checkpoints. A definition
replaced in a group remains available while a save entry, active frame, or
retained checkpoint owns it; a local definition released by `unsave` is
reclaimed after its last frame/checkpoint owner disappears. Resource replay
continues to unwind the ordinary parser and restore a full checkpoint. No
scanner, matcher, or Rust caller continuation crosses that boundary. Retention
must be charged to the existing aggregate contract, with exact release on
checkpoint drop and no append-only activation history.

The direct reader must preserve provenance. Source-derived argument tokens
retain their source locations when forwarded through nested macros; generated
tokens retain their established derivation; diagnostics and output provenance
must remain resolvable after the originating frame retires and across restore.
Do not remove `TracedTokenWord`, origin records, or side metadata until the
replacement proves the same cases and deletes the old owner in that stage.
Halt remains sticky: fuel, cancellation, capacity, error-limit, and resource
halts propagate to an existing bounded poll, and effect sinks remain gated.
Polling may move to a measured reader/dispatch boundary but must retain a
documented maximum amount of work before unwind.

### Implementation children and acceptance

The Beads children under `umber2-i0n5` are the execution plan; this table
states their dependency and closure evidence. Every child names deleted
representations/APIs and runs the native routine suite and authoritative check
gate. Performance receipts and the original numerical budgets are informative
architectural goals. The user's 2026-10-01 instruction explicitly makes them
nonblocking for implementation closure; report misses without weakening targets.

| Order | Focus                                                               | Required semantic and deletion evidence                                                                                                                                                                                                               |
| ----- | ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | Combined immutable spans and packed delivery (`.4`/`.11`)           | No body/list activation copy; native scalar consumer loop and one final meaning target; obsolete reader carriers and font column deleted; retained owners survive group exit and release on retirement/restore; named checkpoints reject live macros. |
| 2     | Definition-owned matcher plans (`.5`)                               | No per-call plan/failure-table allocation; TeX82 matcher cases pass.                                                                                                                                                                                  |
| 3     | One-pass macro argument construction (`.10`)                        | No scratch-to-input second copy; final frame-readable argument storage preserves provenance.                                                                                                                                                          |
| 4     | Full-checkpoint and TeX group ownership (`.14`, `.6`, `.13`)        | Follow the selected aggregate contract: direct group stack and indexed interval undo under one mutation policy; one mutable generation with cold accepted tails; counted dynamic definitions; delete displaced owners.                                |
| 5     | Direct main-control dispatch (`.6`)                                 | No duplicate dispatch engine or eligibility shim; exact semantics, effect ordering, and byte-exact story.                                                                                                                                             |
| 6     | Checkpoint, halt, reclamation, and provenance closure audit (`.12`) | Full restore reproduces hashes/incremental state; resource replay leaves no parser continuation; exact owner release and forwarded provenance; bounded halt unwind.                                                                                   |
| 7     | Hot-set layout and full-book performance (`.7`)                     | Report production instruction, cycle, time, RSS, and cache receipts against unchanged goals; retain layout changes only with evidence.                                                                                                                |
| 8     | Node memory if required (`.8`)                                      | One node/page/box checkpoint ownership contract; old journals deleted; semantic retention gates pass.                                                                                                                                                 |

The microbenchmark table below remains the original per-operation target.
Intermediate-stage misses do not change its numbers or block implementation
closure. The canonical tracer baseline is an **exact total of 663 known
divergences** (exit 1, `VERDICT: DIVERGED`). Compare the exhaustive ordered
worklist/signatures as well as the total; `PARTIAL` totals are lower bounds.
For unintended semantic changes, follow
[Canonical Divergence Working Contract](canonical_divergence_workflow.md) and
diagnose against the pinned oracle. The byte-exact canonical story test remains
a separate required gate. Full-book auxiliary, text, raster, known-difference,
instruction, cycle, time, and RSS targets below and in
[Book performance refactors](book_performance_refactors.md) remain unchanged.

## Thesis

On the 588-page book, Umber executes 3.8 times pdfTeX's instructions and 4.8
times its cycles: 644G instructions and 464G cycles, against 169G and 97G. On
primitive operations the gap is 4 to 14 times. Removing layers one at a time has
not closed it. On the `perf-token-machine` branch:

- each removed layer recovered about one percent;
- removing every journal saved 2.3% of instructions and no cycles;
- the branch's combined flattening reached 18% fewer instructions per loop
  iteration.

The cost is not in any single layer. It is in the shape of the machine:

- A token passes through about eight representations between storage and its
  consumer.
- A macro activation moves its body through four owner types, takes two
  reference-counted handles, and allocates a rollback identity before the body
  reaches the input stack.
- Every command opens and settles rollback marks in five owners.
- One `\begingroup\endgroup` pair touches about fifteen owners. pdfTeX pushes
  and pops one save-stack word.
- The executable's text section is twelve times the size of pdfTeX's whole
  program.

pdfTeX is fast because each of these operations is a handful of loads and stores
on a few global arrays. Umber should be that machine too: one written in safe
Rust with typed indices, keeping its checkpoints, with no per-token or
per-command work for features needed only at boundaries.

One observation makes this possible: checkpoints are only taken at quiescent
boundaries. At every full checkpoint:

- no macro body is on the input stack;
- no TeX group is open;
- no alignment, matching, or definition scan is active;
- the mode nest has one vertical level.

So input delivery, parameter storage, scanner state, and the quiescent mode
record need no checkpoint journal. TeX grouping still requires its actual
local save stack, which is distinct from checkpoint history. They are small at every checkpoint, so they can be
copied by value there.

Only long-lived global state needs an interval journal: meanings and registers,
the token heap, node memory, and output streams. It is journaled on the first
write to each cell in a checkpoint interval, which costs one test per write.

## Baseline

### Per-operation instructions

Each case runs a snippet inside a counted loop:

```tex
\def\iter{\ifnum\count255<N \advance\count255 by 1 <snippet>\expandafter\iter\fi}\iter
```

Method:

- N is 50,000 and then 250,000. The per-iteration cost is the difference in
  user instructions divided by 200,000.
- Every case except `base` then subtracts the `base` loop.
- Each run is pinned to one core.

The three engines:

| Engine        | Build                                                                            |
| ------------- | -------------------------------------------------------------------------------- |
| Umber         | The frozen `umber2-zauh` binary: optimized, frame pointers, production features  |
| Token machine | The `perf-token-machine` branch, frame-pointer release build                     |
| pdfTeX        | The clean pdfTeX 1.40.29 oracle, run as `-ini -etex`, with a minimal `texmf.cnf` |

Definitions: `\def\mzero{}`, `\def\mone#1{}`, `\def\mtwo#1#2{#2#1}`,
`\def\mdelim#1.#2\stop{}`, `\def\mbody{\relax\relax}`, and `\let\x\relax`.

| Case          | Snippet                                          |  Umber | Token machine | pdfTeX | Umber/pdfTeX |
| ------------- | ------------------------------------------------ | -----: | ------------: | -----: | -----------: |
| `base`        | the loop itself                                  | 18,949 |        15,566 |  3,814 |         5.0× |
| `relax`       | `\relax`                                         |    554 |           463 |     91 |         6.1× |
| `mzero`       | `\mzero`                                         |    356 |           245 |    216 |         1.6× |
| `mbody`       | `\mbody`                                         |  2,248 |         1,852 |    398 |         5.6× |
| `mone`        | `\mone x`                                        |  1,767 |         1,351 |    421 |         4.2× |
| `mone_brace`  | `\mone{xy}`                                      |  2,077 |         1,656 |    709 |         2.9× |
| `mtwo`        | `\mtwo\relax\relax`                              |  6,811 |         5,987 |    975 |         7.0× |
| `mdelim`      | `\mdelim a.b\stop`                               |  3,770 |         2,827 |    781 |         4.8× |
| `let`         | `\let\x\relax`                                   |  2,061 |         1,761 |    349 |         5.9× |
| `def`         | `\def\x{abc}`                                    |  4,955 |         4,196 |    866 |         5.7× |
| `edef`        | `\edef\x{\mzero abc}`                            |  6,585 |         5,481 |  1,138 |         5.8× |
| `ifx`         | `\ifx\x\relax\fi`                                |  2,179 |         1,909 |    544 |         4.0× |
| `iffalse`     | `\iffalse\relax\mzero\def\x{ab}\let\y\z\else\fi` |  3,239 |         2,750 |  1,231 |         2.6× |
| `expandafter` | `\expandafter\relax\mzero`                       |  2,615 |         2,042 |    637 |         4.1× |
| `csname`      | `\csname relax\endcsname`                        |  3,136 |         2,887 |  1,214 |         2.6× |
| `group`       | `\begingroup\endgroup`                           |  4,025 |         2,571 |    283 |        14.2× |
| `advance`     | `\advance\count2 by 1␣`                          |  6,237 |         5,319 |  1,150 |         5.4× |
| `count`       | `\count2=5␣`                                     |  5,161 |         4,316 |    876 |         5.9× |
| `toks`        | `\toks2={abc}`                                   | 17,229 |        15,592 |  1,390 |        12.4× |

`mzero` is close to pdfTeX only because an empty parameterless macro already
skips its input row.

The body frame is the real cost:

- `mbody` minus two `relax` costs is 1,140 instructions to push and retire one
  two-token body.
- pdfTeX spends 216 on the same work.

The token-machine branch's 18% base-loop reduction is the most that flattening
today's owners has achieved.

### Macro activation

`\def\mtok{\mzero}` runs 20 times per iteration for 800,000 iterations. The
profile samples user instructions every million. Self shares:

| Symbol                                         |   Self |
| ---------------------------------------------- | -----: |
| `expanded_next_hot_with_mode`                  | 27.46% |
| `activate_read_macro`                          | 16.25% |
| `push_macro_activation`                        |  8.73% |
| `admit_macro_definition`                       |  7.78% |
| `InputStack::push_row`                         |  3.94% |
| `admit_resident_macro_body`                    |  2.96% |
| `drain_depleted_input`                         |  2.91% |
| `retire_resident_ordinary_input`               |  2.35% |
| `DefinitionArena::chunk_handle`                |  2.09% |
| drop glue (`Rc` owners)                        |  1.01% |
| `classify_macro_activation`                    |  0.64% |
| loop scanning (`scan_radix_tail_from` and kin) |    ~6% |

Admission, activation, push, and retirement own about 47% of the loop. The
fused reader, resolver, and settlement own another 27%.

pdfTeX's `macro_call` (§389) does much less for a parameterless macro:

1. It saves two globals.
2. It ends exhausted token lists (§390).
3. It calls `begin_token_list`: copy the 8-field `cur_input` record into
   `input_stack`, set `start`, `loc`, and `param_start`, and add one token
   reference.

Retirement (`end_token_list`) deletes that reference and pops the record.

### Groups

There are ten `\begingroup\endgroup` pairs per iteration. Every owner below runs
for an empty group with tracing off:

| Owner and method                                                                                                         |  Self |
| ------------------------------------------------------------------------------------------------------------------------ | ----: |
| `CommandContext::roll_state_operation`                                                                                   | 4.09% |
| `CommandContext::{begin_group, end_group}`                                                                               | 5.74% |
| `CommandState::{begin_group, end_group, validate_group_payloads}`                                                        | 5.52% |
| `DurableBoxState::{begin_group, begin_unsave, begin_operation, commit_operation}`                                        | 6.52% |
| `LocalDefinitionSlots::{allocate, reclaim, lease_count, region_mut, parent}` and the arena's `begin_group`/region change | 6.68% |
| `SaveJournal::{record_group_exit_with_records, return_sparse_scratch}`, `SaveSequence::{append, truncate}`               | 4.89% |
| `leave_group_payloads`, `save_stack_words`, `trace_group_restorations`                                                   | 4.23% |
| `CommandAttempt::roll_operation`                                                                                         | 0.72% |
| libc `memcpy` and `memmove`                                                                                              | 2.88% |

pdfTeX's `new_save_level` pushes one entry and `unsave` pops to it. That is 283
instructions per pair, including both dispatches.

`trace_group_restorations` runs even with `\tracingrestores` at zero. That is
slow-tier work on the fast path.

### Token-register assignment

`\toks2={abc}` costs 17,229 instructions against 1,390.

The scanned list moves through three representations:

- the attempt arena;
- promotion: `ColdOperationPromotion::{token_root, next_token_root}` and
  `promote_resident_batch`;
- publication again: `TokenListArena::publish_words` and
  `DefinitionRegion::reserve`.

Around it runs the cold-episode protocol. Each step is a separate symbol:

- `execute_cold_episode` (4.86%), `preflight_replay_delivery` (3.18%),
  `prepare_cold_execution_episode`, `execute_scanned_cold_episode`, and
  `execute_typed_operation`;
- `observe_committed` and `settle_save_stack_usage`;
- `ModeNestStorage::commit_journal`;
- `PageRegionHistory::parts_mut`.

Heap allocation adds `malloc` (1.38%) and two `Vec::from_iter` collections
(2.2%).

### Machine-level symptoms

- `size -A` reports a 20.2 MB text section for Umber against 1.68 MB for the
  pdfTeX oracle.
- The `perf-token-machine` branch measured:
  - L1 instruction-cache misses at 65 times pdfTeX's;
  - iTLB misses at 376 times pdfTeX's;
  - a 223 KiB hot-code footprint;
  - profile-guided optimization removing 22% of instructions and 33% of cycles.
- On the full book, Umber retires 1.39 instructions per cycle; pdfTeX retires
  1.74.

Umber executes more instructions, and also executes them worse, because the hot
code does not fit in the instruction cache.

## Diagnosis

### Representation churn

Each type below is created on the hot path and consumed almost immediately:

| Type                                                     | Created                                  | Consumed                                     | Replacement                                  |
| -------------------------------------------------------- | ---------------------------------------- | -------------------------------------------- | -------------------------------------------- |
| `ResidentWord`                                           | `read_resident_run`, per token           | resolution                                   | a `u32` token in a register                  |
| `ResidentWordRead`, `ResidentColdOutcome`                | per token                                | `finish_charged_raw_read`                    | loop control flow                            |
| `TokenMeaning`                                           | `write_packed_token_command_into`        | delivery                                     | the packed `u64` meaning itself              |
| `ExpansionCandidate`                                     | `read_expansion_candidate`               | the expansion loop                           | the `(token, meaning)` pair                  |
| `HotCommand`, about 72 bytes                             | materialization, per non-macro token     | settlement and classification                | the `(token, meaning)` pair                  |
| `CurrentCommand`                                         | `finish_hot_delivery`                    | scanners and main control                    | the pair, plus cold facts computed on demand |
| `AdmittedMacroDefinition`, `ResidentMacroBody`           | `admit_macro_definition`, per activation | activation                                   | two words of the definition header           |
| `MacroBodyCursor`, `ResidentTokenRow`, `InputLevel`      | activation                               | `push_row`                                   | one 16-byte frame written in place           |
| `MacroPlan`, `MatchingContext`, `PendingArgumentSet`     | per call with parameters                 | `macro_call_scalar`                          | a definition-owned plan and locals           |
| Delimiter failure links                                  | per delimited argument                   | `scan_delimited_argument`                    | built once when the definition is sealed     |
| `InputRetirement`, `RetirementHandoff`                   | per pop                                  | `drain_depleted_input`                       | a kind test in the pop                       |
| `ColdOperation`, `OperationPreparation`                  | per cold command                         | `execute_cold_episode`                       | a direct call                                |
| `DirectOperationMark`, `DiagnosticEffects`               | per command                              | settlement                                   | nothing                                      |
| Attempt-arena words, promotion batch, published list     | per stored token list                    | the next stage                               | one copy from the collector into the heap    |
| `StateCell`, `StateWord`, `BankCell`, journal `Mutation` | per assignment                           | `read_cell`, `write_cell`, `record_mutation` | one `u64` store and a first-touch test       |

### Ownership through region handles

- Definitions live in `DefinitionArena` regions: format, global, and per-group
  local regions.
- Activating a macro takes the region by lookup and incarnation check, clones
  the region owner and the chunk handle (an `Rc` and a `RefCell` borrow), and
  drops both at retirement.
- Each group allocates and reclaims a local-definition region whether or not it
  defines anything.
- Tokens live in six storage domains, and `read_resident_run` matches on the
  domain for every word it reads.

TeX has one token memory. A definition is a span in it with a reference count,
and a group does not allocate anything.

### Rollback on the per-token and per-command path

The input stack keeps:

- an undo journal;
- rollback markers;
- slabs of displaced rows;
- a rollback identity for every pushed level, recorded in the command timeline.

Each direct command takes a `DirectOperationMark` across five owners: the state
cursor, the mode journal, active boxes, the attempt arena, and page nodes. It
commits or discards them at the end.

The eqtb assignment path serves two masters with one journal:

- TeX's save stack, for group restoration;
- checkpoint rollback, using a per-cell save serial.

It also updates a commutative state hash on every write.

Two facts make most of this unnecessary:

- Every non-fatal unwind already restores the latest full checkpoint.
  `restore_pending_resource` in `crates/umber/src/engine_session.rs` replays
  from `latest_replay_checkpoint`.
- Recoverable TeX errors are reported as diagnostics, not used as rollback
  points.

Per-command rollback units therefore discard state that is about to be thrown
away anyway. Journaling the input stack serves checkpoints that never contain a
macro frame.

### Per-token accounting

The reader does all of this for every raw token:

- charges fuel;
- clears a pending diagnostic location;
- stamps an `OriginId` into an 8-byte traced word;
- carries a source position, an identity, and a read site.

TeX does none of these. Today's fuel unit is the raw token, which forces a
decrement and a branch into the innermost loop.

### Result plumbing and out-parameters

- Per-token functions return `Result<_, CommandError>` and write their command
  through `&mut Option<HotCommand<G>>`, which is then unwrapped with `expect`.
- Every level of the reader carries a discriminant test, and every consumer
  re-tests it.
- The large command is written through memory rather than returned in
  registers.

### Code volume

- Hot paths are towers of `#[inline(always)]` generic functions that
  monomorphize over the generation coordinate and the observation flag. Cold
  facts such as alignment adjustment, source sites, and test counters are woven
  through them.
- The binary's text is twelve times pdfTeX's. Its hot code is several times the
  L1 instruction cache.
- That is why profile-guided optimization saves a third of cycles: the problem
  is layout as much as work.

## Design laws

1. **One representation per concept.** A token is a `u32` and a meaning is a
   `u64`. An input frame is 16 bytes. Nothing else is constructed per token,
   and nothing is constructed per command unless the command's semantics store
   it.
2. **Everything lives in arenas.** The hot path performs no heap allocation and
   uses no `Rc`, `RefCell`, or `Drop` glue. Storage is addressed by `u32`
   offsets into a few arrays owned by one `Machine`.
3. **Roll back only at checkpoints.** State is journaled only if it can survive
   a checkpoint, and only on its first write in a checkpoint interval. Nothing
   is journaled per command.
4. **Exceptional work is one test away.** A `slow` byte gates tracing,
   observation, unusual scanner states, and outer checks. A `budget` counter
   gates fuel, cancellation, halts, and resource misses. Both are tested only
   at frame pushes, line loads, and command dispatch.
5. **Return in registers.** Per-token functions return `(u32, u64)` or smaller.
   They take no out-parameters and return no `Result`.
6. **Keep the hot code small and contiguous.** The hot set is a budgeted list of
   functions. Everything else is `#[cold]` and `#[inline(never)]`. Production
   builds use profile-guided optimization and post-link layout.
7. **Read meanings live.** A meaning is read from eqtb when TeX would read it.
   Nothing caches a meaning across a token.
8. **Share state between tiers.** Tracing and observation run as a second
   instantiation of the same functions over the same state. Switching tiers
   costs nothing.

## The machine

The selected core has direct packed token/meaning reads, frame-owned cursors,
immutable definition/list spans, retained arguments, and one direct dispatch
loop. Its generation owns the dense banks, group stack and checkpoint undo
log. Level and host epoch metadata stay off the meaning-only read path.

The previous two-ended `mem` proposal, span `rc`/`hold` rollback epochs,
quarantine, blanket token-list arena replacement and assumed 16-byte save
record are retired. They are not an alternative implementation lane. Keep the
real immutable payload owners and specialized node/PDF storage; replace the
measured immediate-consumption paths and their bookkeeping.

### Packed meanings and assignments

The production scalar reader already resolves into final `CommandWord` from
one canonical packed row. Meanings preserve their typed font or compact macro
coordinate. Dense bank access is direct; assignment level and checkpoint
metadata are not copied into ordinary commands.

All meaning/register/parameter/code and box-binding mutations use the central
policy in [the checkpoint contract](aggregate_checkpoint_contract.md): one
local-level predicate, one indexed host-epoch comparison, independent group
and interval preimages only when required, then root-safe replacement.
Checkpoint stamps are monotone host metadata, never restored TeX state.
Dynamic definitions use stable counted slots; active frames retain the full
payload. No count or checkpoint work occurs on word reads/substitution.

The remaining frame/reader pseudocode describes hot-loop shape and layout
experiments, not a required new allocator or second engine. Typed owner
coordinates and provenance must preserve the authoritative storage contract.

### Frames

```rust
#[derive(Clone, Copy)]
#[repr(C)]
struct Frame {
    pos: u32,   // next token offset in mem, or next byte of the source line
    end: u32,   // limit
    owner: u32, // definition header, heap span, transient region, or source slot
    meta: u32,  // kind:4 | flags:4 | base:24 (parameter base or tokenizer state)
}
```

The frame kinds:

| Kind     | Reads                     | Owner                                                                       |
| -------- | ------------------------- | --------------------------------------------------------------------------- |
| Body     | heap                      | definition header; `base` is the parameter-stack base                       |
| Param    | transient                 | none; it reads one argument span                                            |
| Backup   | transient                 | region                                                                      |
| Inserted | transient                 | region                                                                      |
| Stored   | heap                      | span (`\everypar`, `\output`, template parts, `\mark`, and `\write` replay) |
| Source   | line buffer of the source | source slot; `base` holds TeX's tokenizer state                             |

Stack operations:

- `top` is TeX's `cur_input`, held directly in the machine.
- A push is `frames.push(top); top = new`, and a pop is the reverse.
- There is no identity allocation, undo journal, displaced-row slab, or
  timeline record.

The cold side table:

- `frame_cold` runs parallel to `frames`.
- Body pushes write the called control sequence into it with one store, which
  serves TeX's context display.
- Everything else is written only on cold paths: call-site origins, replay
  completion fences, and identities for the observed tier.

The flag `COLD_RETIRE` sends a pop to a cold handler. It is set for templates,
output routines, `\write` replay, and frames with completion fences. Ordinary
pops never test anything else.

### Reading

TeX's `get_next` (§341) for a list frame becomes:

```rust
#[inline(always)]
fn next_tok(&mut self) -> u32 {
    loop {
        let f = &mut self.top;
        if f.pos < f.end && f.is_list() {
            let tok = self.mem[f.pos as usize];
            f.pos += 1;
            if tok < SPECIAL {
                return tok; // character or control sequence
            }
            self.special_word(tok); // out_param pushes a Param frame; frozen marker
            continue;
        }
        if f.is_source() {
            if let Some(tok) = self.source_next() {
                return tok;
            }
            continue;
        }
        self.retire_top(); // exhausted list: §324 end_token_list
    }
}
```

The order of token kinds puts `out_param` and frozen markers above every
character and control sequence. The common case is therefore one compare.

Brace counting for `align_state` and the §342 alignment check need little
extra:

- They key off a 16-entry catcode flag table for character tokens.
- For control sequences, they key off the `tab_mark` and `car_ret` command
  range.
- Only catcodes 1, 2, 4, and 5 are touched.

The source path is TeX's §343 state machine over the current line buffer:

- one catcode table load per byte;
- names looked up through the control-sequence table, with single-character
  names indexed directly;
- lines refilled at the line boundary, which also charges fuel and polls the
  budget;
- tokenization stays lazy, so catcode changes take effect exactly where TeX's
  do.

`back_input` from a list frame usually returns the token it just read to that
same frame:

- If `mem[pos - 1]` is the token, it decrements `pos` and reverses the brace
  count. Otherwise it pushes a one-word Backup frame, as TeX does.
- The unread coordinate is recorded and cleared on pop, so `show_context` can
  still print TeX's `<to be read again>` level.
- `back_error` and `ins_error` always push real frames.

### Expansion

`get_x_token` (§380) fuses reading, meaning lookup, and macro calls:

```rust
#[inline(always)]
fn get_x_token(&mut self) -> (u32, u64) {
    loop {
        let tok = self.next_tok();
        let m = self.meaning(tok);
        if cmd(m) < FIRST_EXPANDABLE && m & (OUTER | SLOW) == 0 {
            return (tok, m);
        }
        if is_macro(m) && self.slow == 0 {
            self.macro_call(tok, m);
            continue;
        }
        if let Some(done) = self.expand_or_slow(tok, m) {
            return done; // protected, frozen endv, outer checks
        }
    }
}
```

Expandable primitives dispatch through one jump table on `chr` into warm
functions:

- the conditionals, `\expandafter`, `\noexpand`, `\csname`, `\the`,
  `\number`, and `\string`;
- everything else, which is cold.

Each primitive writes its result where it belongs:

- `\the`, `\number`, and `\string` write straight into a transient region and
  push one Inserted frame.
- `\csname` collects into the collector and pushes the resulting control
  sequence as a one-word Backup frame.
- The condition stack is a `Vec` of three-field entries.

Scanners call `get_x_token` and use `(tok, m)` directly:

- `scan_int` reads digits by comparing the token against the other-character
  range.
- `scan_keyword` compares tokens.
- Nothing materializes a command.

### Macro calls

A committed macro activation admits the full immutable definition payload once,
then borrows its packed parameter/replacement words and sealed matcher directly.
The input frame owns the payload and sole cursor. A parameterless empty body
can elide its frame under the established observed/recovery rules. TeX82 §390
conservation still retires exhausted list frames before body push.

The definition's marker index and compiled failure links provide:

- undelimited matching that skips spaces and accepts a token or trimmed balanced
  group;
- delimited matching that reuses the longest literal prefix; and
- direct comparison of leading literal text.

The accepted-token writer retains held source origins, brace depth, paragraph
and outer checks and TeX recovery. Failure links do not replace those semantics.
Arguments write once into the invocation-owned pooled region; the body owns it
and parameter frames borrow it. No per-call plan/failure allocation or
scratch-to-input second copy remains after `.5`/`.10`.

Scanner status and warning context follow ordinary synchronous cleanup. A
resource miss unwinds the match and host replay restores the full checkpoint;
it never retains a matcher or Rust caller continuation. See
[Expansion memory lifetimes](expansion_memory_lifetimes.md).

### Retirement

| Kind              | Retirement                                                      |
| ----------------- | --------------------------------------------------------------- |
| Body              | drop full immutable payload; release body-owned argument region |
| Param             | nothing                                                         |
| Backup, Inserted  | release the region                                              |
| Stored            | drop actual immutable list owner                                |
| `COLD_RETIRE` set | cold handler                                                    |
| Source            | cold handler: end of file, `\endinput`, `\everyeof`, closing    |

TeX's §390 conservation retires exhausted list frames before a macro pushes its
body, so tail recursion does not deepen the stack.

### Main control

```rust
fn main_control(&mut self) -> Result<Stop, Halt> {
    loop {
        let (tok, m) = self.get_x_token();
        self.budget -= 1;
        if self.budget < 0 {
            self.poll()?;
        }
        match dispatch(self.mode(), cmd(m)) {
            Op::CharRun => self.main_loop(tok, m), // §1034, from source bytes when possible
            Op::Relax => {}
            Op::BeginGroup => self.new_save_level(SIMPLE_GROUP),
            Op::EndGroup => self.end_simple_group(),
            Op::Prefixed => self.prefixed_command(tok, m),
            // ... the dense mode × cmd table, with cold arms #[cold]
        }
    }
}
```

The lane is the loop. Every command runs through this dispatch and changes state
in place. There is no:

- operation mark;
- cold episode preparation;
- operation promotion;
- committed observation receipt.

The existing `main_character_run` source-byte path becomes the `CharRun` arm.

### Groups and the save stack

Use the direct group representation in
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md).
Empty groups push/pop only scalar boundary/kind/source information. Actual
local first writes save old value and TeX level; same-level writes do not save
again. Global writes set level one and suppress displaced local saves.

The stack owns ordered classic/box restores, lazy e-TeX sparse-chain markers,
aftergroup tokens and actual builder payloads. Preserve reference restoration
and backup order. Unsaving restores the current epoch, emits demanded
restoration diagnostics synchronously while owners remain valid, then drops
losing roots. Delete duplicate box/command/definition group topology,
`GroupRestorations` and immediate-consumption token vectors. Definitions do not
allocate a group region or require local-to-global promotion.

### Errors and halts

Errors follow TeX: recoverable errors report and continue in place.

- `self.error(kind)` is cold. It formats the message and context from the
  frames, appends to the diagnostic stream, and counts errors.
- The caller then performs TeX's own recovery. There is no `Err` return and no
  rollback.

Halts are sticky. These conditions set `halt` and drive `budget` negative:

- fuel exhaustion;
- cancellation;
- a resource miss;
- capacity overflow;
- too many errors;
- a pdfTeX output fatal;
- an interaction stop.

The hot loops poll `budget` only where they already branch: at frame push,
source line load, and command dispatch. The poll returns `Err(Halt)` from
`main_control`. Only that loop and cold paths use `Result`.

A halt site captures its diagnostic context before it returns. It also returns
the same recovery value TeX would continue with: an empty file, `nullfont`, or
zero.

Polling bounds the time between a halt and the unwind. Every unbounded TeX
computation must push frames, load lines, or dispatch commands, because reading
without any of those consumes a finite supply of tokens.

This is sound because nothing a halted run does afterwards can be observed:

- Every halt either terminates or restores a full checkpoint.
- Every effect sink checks `halt` before committing, on its cold path. The sinks
  are log and terminal text, `\write` files, shipout, and PDF objects.

There is no `DirectOperationMark` and no per-command journal in any owner.

Cancellation is an atomic flag. It is read when the budget is refilled from the
fuel allowance, in chunks of about 64K units, so its latency is one chunk.

One case needs a different mechanism. The `PdfExtensionInDviMode` preflight
retry is the only remaining reason to roll back a command; it should become a
check before mutation.

### Fuel

Fuel stays deterministic, but its unit changes from raw tokens to work admitted
at boundaries:

| Event               | Charge             |
| ------------------- | ------------------ |
| List frame push     | the frame's length |
| Source line load    | the line's length  |
| Command dispatch    | one                |
| File and font opens | a fixed cost       |

Every token a list frame can deliver is paid for when the frame is pushed, so
the fuel spent still bounds the work done. Exhaustion is detected at the next
boundary rather than at an exact token. Corpus guards and the fuel tests must be
recalibrated to the new unit. This is a contract change, not a semantic one:
fuel is not part of TeX.

### Provenance

Tokens are bare `u32` everywhere. A token's origin depends on where it was read:

| Read from      | Origin                                                           |
| -------------- | ---------------------------------------------------------------- |
| A source frame | `(source slot, line, byte)`                                      |
| A list frame   | the frame's owner and offset, plus the call site in `frame_cold` |

Recording rules:

- Spans copied from source frames record sparse origin runs in a side table
  keyed by span offset. These are document-body arguments, `\toks` text, and
  definitions. Only the copy loop writes them, and only when its source is a
  file.
- Macro bodies already carry no per-token origins.
- Diagnostics compute origins from the frames on demand.
- Character nodes receive their origin from the main character loop, which
  holds the source cursor. Ligatures record a run, not a `Vec<OriginId>`.
- When neither diagnostics nor output provenance asks, nodes record nothing.

This replaces `TracedTokenWord` and per-token `OriginId` stamping.

### Tracing and observation

- **TeX tracing.** `\tracingmacros`, `\tracingcommands`, `\tracingassigns`,
  `\tracinggroups`, `\tracingrestores`, `\tracingifs`, `\tracingnesting`, and
  `\tracingscantokens` feed the `slow` byte.
  - Assigning any tracing parameter recomputes `slow`.
  - Each tracing site tests `slow` at a boundary it already crosses.
- **The observer.** The machine is generic over `O: Observer`, and its hooks
  sit at frame push and pop, command dispatch, assignment, group, and halt.
  `NoObserver` compiles them away.
- **Semantic tests.** The observed instantiation reconstructs the receipts that
  [semantic testing](semantic_testing.md) consumes. It computes identities
  there, in `frame_cold`, and not in the fast tier.

Both tiers share every array, so a tier switch at an episode boundary costs
nothing.

## Checkpoints

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
defines the complete `EngineCheckpoint` inventory, actual cold capture costs,
quiescence, first-touch undo, repeated roots, candidate reverse/forward swaps,
root-before-suffix restore, memory accounting, pruning and full resource
replay. This section intentionally contains no second checkpoint algorithm.

The named schedule remains JobStart, eligible outer paragraph and outermost
shipout, with no active groups/macros/scanners/builders/alignment. Open
conditionals and non-macro input may remain. Command/input captures copy their
actual cold rows and owned transient words; dense state is not snapshotted.
Capture is not uniformly O(1). Only assignments compare indexed epochs;
ordinary token/meaning reads and macro substitution perform no checkpoint
bookkeeping. Empty groups capture no other owners.

One mutable generation executes candidates while accepted alternates remain
in cold tails. Rejection undoes current and forward-swaps accepted; acceptance
drops old alternates. Restore installs all semantic roots before backing
suffixes drop, preserves the selected root for repeated restore, and advances
the host epoch. Fuel and high-water counters never rewind.

Definition slot roots follow actual live cells and group/checkpoint/candidate
owners; active frames retain one immutable payload. Payload release follows
its final real owner. Node/page/box regions and PDF version roots keep their
specialized semantics; there is no token/node quarantine or reference-count
undo scheme. Optional convergence identity remains complete and fail-closed:
missing component roots cannot be replaced with cursor/allocator identities.

Resource misses unwind ordinary calls and restore the full root/host prefix
before parsing again. No scanner, matcher, Rust caller or output continuation
crosses that boundary. See
[Resource-miss checkpoint replay](checkpoint_resource_replay.md).

The production 588-page profile has detail counters disabled. Historical
command-lane counts have no available revision-scoped census receipt; there
is no verified read/write/group/save/checkpoint ratio. Qualitative inference:
reads and macro expansion dominate body traffic; local assignments are common
within style/environment/scratch work, globals/root writes in package setup
and counters. Groups can be frequent with sparse first-write saves. Named
boundaries and replay/candidates are much rarer than token traffic. No numeric
ratio or inferred percentage is part of the design.

## Instruction budgets

Estimated cost of a parameterless macro with a non-empty body, excluding the
body's tokens:

| Step                                                          | Estimate |
| ------------------------------------------------------------- | -------: |
| Read the call token from a list frame                         |        6 |
| Load the meaning and test the expandable and slow bits        |        4 |
| Load the definition header and test its parameter shape       |        4 |
| §390 conservation test                                        |        3 |
| Hold: compare against `format_mark`, increment                |        3 |
| Push the frame: capacity compare, 16-byte store, `top` update |        8 |
| Record the call name                                          |        2 |
| Charge fuel and test the budget                               |        3 |
| Retire: kind dispatch, release, truncate `params`, pop        |       15 |
| **Total**                                                     |  **~48** |

pdfTeX spends about 216 instructions on the same work. Matching or beating it is
realistic because:

- there is no linked-list token storage;
- arguments are contiguous copies;
- there is no `check_interrupt` per pop.

Bounds checks and the budget test cost a little. The targets below are the
original targets for the microbenchmark harness (informative, not closure blockers):

| Case      |  Today | pdfTeX | Target |
| --------- | -----: | -----: | -----: |
| `base`    | 18,949 |  3,814 | ≤4,500 |
| `relax`   |    554 |     91 |   ≤100 |
| `mzero`   |    356 |    216 |   ≤150 |
| `mbody`   |  2,248 |    398 |   ≤350 |
| `mone`    |  1,767 |    421 |   ≤350 |
| `mtwo`    |  6,811 |    975 |   ≤900 |
| `mdelim`  |  3,770 |    781 |   ≤700 |
| `let`     |  2,061 |    349 |   ≤350 |
| `def`     |  4,955 |    866 |   ≤900 |
| `edef`    |  6,585 |  1,138 | ≤1,200 |
| `group`   |  4,025 |    283 |   ≤300 |
| `advance` |  6,237 |  1,150 | ≤1,200 |
| `count`   |  5,161 |    876 |   ≤900 |
| `toks`    | 17,229 |  1,390 | ≤1,500 |

Full-book targets, in order:

1. After the token machine and main-control stages:
   - at most 300G instructions, down from 644G;
   - `tex-command` and main control reduced at least fourfold.
2. After layout and node memory, within 1.5 times pdfTeX:
   - about 250G instructions;
   - about 145G cycles.
   - Instructions per cycle should reach pdfTeX's once the hot set fits in the
     instruction cache.

These are targets, not forecasts. The microbenchmark gates are measurable after
each stage.

## Code size and layout

- **Hot set.** The hot set is enumerated and budgeted at 64 KiB of machine code:
  - `next_tok`, `source_next`, and `get_x_token`;
  - `macro_call`, the two matcher loops, and the warm expandable primitives;
  - the scanner fast paths for integers, dimensions, keywords, and optional
    equals;
  - `main_control`, `main_loop`, `eq_define`, `new_save_level`, `unsave`, and
    `retire_top`.

  The harness reports the size from symbol sizes, and a size regression fails
  review.
- **Cold code.** Every other function reachable from the hot set is `#[cold]`
  and `#[inline(never)]`. That includes error reporting, tracing, observation,
  source refill, and file handling.
- **Generics.** The core is not generic over the generation coordinate. The
  observer is its only type parameter, with two instantiations.
- **Production builds.** They use profile-guided optimization, trained on the
  corpus, and BOLT or a symbol-ordering file. Profile-guided optimization alone
  measured a third fewer cycles. The development build keeps the plain
  optimized profile.
- **Recursion depth.** The engine thread's stack is sized for e-TeX's expansion
  depth limit. TeX recursion through expansion, macro calls, and scanning stays
  native recursion with small frames.

## Deletions and implementation sequence

The source-specific deletion map is in
[the checkpoint contract](aggregate_checkpoint_contract.md). Beads owns task
state; the sequence is:

1. `.4`/`.11`: immutable span activation and final packed scalar delivery,
   deleting resident token/meaning carriers and duplicated font columns.
2. `.5`: immutable definition-owned matcher, deleting per-call plans and
   failure tables. One payload owns words, matcher and origin.
3. `.14`: selected full-checkpoint/group/definition design, including concrete
   owner inventory and mutation costs; documentation only.
4. `.10`: invocation-owned pooled `ArgumentArena` vectors, body-owned and
   parameter-borrowed. Delete the scratch-to-input second copy and per-argument
   allocations/leases; preserve provenance and recovery. Arguments are empty at
   named capture; returned capacity is an allocator cache charged once.
5. `.6`: central assignment policy and direct group stack, then direct
   mode/command dispatch and integration with the existing sole mutable core
   and cold accepted tails. Preserve genuine coarse owners, slots and brands.
   Delete old save serials, stamp maps, duplicate group topology/restoration
   collections, eager owner captures, lane eligibility and cold-operation
   preparation/application protocols. Port every real consumer in the same
   logical change; no second dispatch or compatibility adapter remains.
6. `.13`: pinned format and stable counted dynamic definition slots. Delete
   append-only payload retention, local row truncation/group marks, promotion
   memo and accidental whole-row candidate-tail liveness. Root insertion/drop
   follows the central policy and memory charges follow final payload release.
7. `.12`: integrated checkpoint/replay/reclamation/provenance/halt audit,
   including repeated restore, branch epochs, candidate settlement and pruning.
8. `.7`: evidence-based hot code layout and full-book measurements. `.8` changes
   node storage only when fresh profiling identifies remaining work; preserve
   node-region and box/PDF semantics.

Each stage commits logical chunks and deletes the displaced representation
with its replacement. No old API/format fallback, shim, second engine or
unnecessary created-then-consumed carrier remains. Implementation runs owning
tests, `cargo test -q --tests`, authoritative `scripts/check.sh`, byte-exact
story and canonical verification. Preserve the exhaustive 663 ordered known
divergence signatures for semantics-preserving work. Measurements and unchanged
original budgets are informative speed goals, not implementation-closure gates.

## Risks

- **Reader and matcher fidelity.**
  - Risk: TeX's §341 to §399 cover many cases: `\outer` checks, `\par`
    insertion, the `#{` delimiter, `\noexpand` markers, template ends, and
    `\endinput`.
  - Mitigation: port section by section with `tex.web` references, and use the
    committed oracle fixtures and
    [canonical divergence workflow](canonical_divergence_workflow.md).
- **Bounds checks.**
  - Risk: direct slice reads can still carry redundant bounds checks.
  - Mitigation: iterate within a frame using slices. If a gate still fails, an
    existing audited substrate boundary can own physical storage while
    semantic callers retain safe slices and typed coordinates.
- **Diagnostic ordering.**
  - Risk: in-place recovery can change where diagnostics appear compared with
    today's rollback-and-retry.
  - Mitigation: TeX's order is the reference, so any difference is a fix, not a
    regression. Confirm it against the oracle.
- **Fuel recalibration.**
  - Risk: exhaustion points and fuel-pinned tests change.
  - Mitigation: rewrite them against the new unit and document it in
    [profiling](profiling.md) and the corpus contracts.
- **Observation coverage.**
  - Risk: semantic tests that inspect per-token identities or per-command
    receipts need equivalents.
  - Mitigation: the observed tier provides them at frame and command
    boundaries.
- **Long checkpoint retention.**
  - Risk: actual preimage owners and cold captures grow with retained checkpoints.
  - Mitigation: both are charged under the aggregate owner inventory. The
    host's checkpoint dropping policy reclaims them.
- **Concurrent work.**
  - Risk: the book refactors are being implemented in `slot-1`, and their first
    stage edits the same reader.
  - Mitigation: coordinate before Stage 4. Work that thins today's owners does
    not carry over.

## Acceptance

- **Per-operation evidence.** Report each stage on the committed harness
  against the unchanged budget table; target misses are not closure blockers.
- **Full-book evidence.** Validate exact outputs and report performance in
  [Book performance refactors](book_performance_refactors.md):
  - exact auxiliary files;
  - all-page text and raster comparison;
  - the four known reference differences preserved;
  - paired measurement of instructions, cycles, time, and RSS on prebuilt
    production binaries.
- **Checkpoint gates.** Checkpoint, resource replay, incremental, and snapshot
  gates pass at every stage.
  - Restores must reproduce identical state hashes.
  - Ownership checks must show every span and node released exactly once after
    rewind, redo, and checkpoint drop.
