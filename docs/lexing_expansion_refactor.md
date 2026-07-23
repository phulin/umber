# Lexing and Expansion Refactor

Status: proposed architecture.

## 1. Purpose

The current lexing and expansion implementation preserves a large body of
TeX82, e-TeX, and pdfTeX behavior, but its structure no longer makes that
behavior easy to recognize. Input delivery, expansion policy, conditional
state, alignment state, replay provenance, scanner recovery, snapshots, and
performance accelerators have accumulated in overlapping abstractions.

This document proposes a refactor organized around the canonical TeX
procedures:

- physical line normalization and tokenization;
- input-level delivery;
- `get_next` and `get_token`;
- `get_x_token`, `x_token`, and `expand`;
- `macro_call`;
- conditional `pass_text`; and
- the value and token-list scanners.

The central abstraction in this design is `ExpansionProcessor`. It owns the
active expansion operation by borrowing the input stack, the aggregate state
facade, and the persistent expansion-session state. The name is deliberately
narrower than the complete TeX engine: execution, modes, page building, and
output remain in `tex-exec`.

The objective is not merely to split large files. The objective is to have one
implementation of each canonical operation, with exceptional behavior derived
from explicit interpreter state instead of encoded in replay kinds and
parallel token-delivery paths.

## 2. Current problems

### 2.1 `tex-lex` has too many semantic responsibilities

`tex-lex::InputStack` currently owns or participates in:

- physical sources and line normalization;
- source-frame tokenization;
- stored and transient token-list replay;
- macro parameter substitution;
- conditional frames;
- alignment depth, cell interception, and template retirement;
- replay provenance and macro invocation provenance;
- paragraph source-transition preparation;
- snapshots and durable summaries;
- transient allocation pooling; and
- expansion profiling counters.

Some of these must be snapshot-coupled, but snapshot coupling does not require
one type to interpret all of them. In particular, a conditional frame is not
an input level, and alignment delimiter classification requires command
meanings that the lexical layer is intentionally forbidden to resolve.

### 2.2 Raw token delivery is implemented more than once

The input stack has raw, traced, read-only, and expansion-aware delivery
variants. Expansion then wraps these with:

- semantic raw delivery;
- unintercepted raw delivery;
- suppressed raw delivery;
- prepared expansion tokens;
- ordinary expanded delivery;
- protected expanded delivery; and
- restricted versus driver expansion modes.

These paths share most of their mechanics but differ in small, consequential
ways. A correction to parameter substitution, alignment accounting, replay
retirement, `\noexpand`, or provenance can therefore require auditing several
loops.

TeX instead has a strong procedural boundary: `get_next` performs raw semantic
delivery, `get_token` records the token identity needed for backup, and
`get_x_token` repeatedly invokes `expand` until an unexpandable command is
available.

### 2.3 Replay kinds combine behavior with explanation

`TokenListReplayKind` currently distinguishes macro bodies and arguments,
`\noexpand`, `\unexpanded`, `\every...` lists, marks, output routines,
inserted material, `\scantokens`, and alignment templates.

These values answer at least three independent questions:

1. How are tokens obtained from this level?
2. Does delivery have special expansion behavior?
3. Why was the level pushed, for tracing, retirement, or diagnostics?

Most replay reasons do not affect token delivery. Conversely, `\noexpand` is a
one-token expansion state rather than a kind of input level, while
`\unexpanded` and token-list `\the` are direct splices in expanding
`scan_toks`, not indefinitely suppressed input.

### 2.4 Expansion plumbing is exposed to every scanner

Scanner functions repeatedly accept:

```rust
input: &mut InputStack,
stores: &mut tex_state::ExpansionContext<'_>,
expansion: &mut ExpansionContext<'_>,
mode: &mut dyn ExpansionMode,
```

This makes scanner code responsible for choosing between subtly different
token drivers. The distinction between input-opening authority, diagnostic
recovery, protected expansion, and ordinary expansion is expressed through
one broad policy trait rather than through the canonical operation requested
by the scanner.

### 2.5 Expansion results make a round trip through `Dispatch`

Expandable commands return `Dispatch::Push` or `Dispatch::PushTransient`;
their caller then immediately translates those values back into input-stack
mutation. TeX's `expand` procedure directly changes the input. The additional
result representation creates another place where replay kind, origin,
arguments, and invocation state must agree.

## 3. Design principles

The refactor should follow these rules:

1. There is one semantic raw-token operation corresponding to `get_next`.
2. There is one ordinary expanded-token loop corresponding to `get_x_token`.
3. Protected expansion is a parameter of that loop, not a separate driver.
4. Input levels contain input; conditional state is maintained separately.
5. Replay delivery behavior and replay trace metadata are separate types.
6. `\noexpand` is transient command state, not a replay frame.
7. `\unexpanded` and token-list `\the` are collector splices, not sticky token
   properties.
8. Expandable commands mutate input through the active processor.
9. Scanners depend on the processor rather than on its constituent mutable
   objects.
10. Snapshot and rollback continue to capture all semantically live state as
    one aggregate executor operation.

The refactor must preserve the existing provenance, resource suspension,
incremental execution, and performance contracts. Matching TeX's procedural
model does not require adopting TeX's global variables or packed memory
layout.

## 4. Target architecture

```text
PhysicalInput
    |
LineReader and Normalizer
    |
SourceTokenizer
    |
InputStack  <---------------- stored/transient token levels
    |
ExpansionProcessor
    |-- get_next / get_token
    |-- get_x_token / x_token
    |-- expand
    |-- MacroCall
    |-- ConditionStack / pass_text
    `-- Scanners
            |
         tex-exec
```

### 4.1 Source tokenizer

The source tokenizer owns only source-local mechanics:

- physical line reading;
- TeX line normalization;
- the M/N/S lexer state;
- `^^` notation;
- Unicode or byte-oriented character delivery;
- catcode lookup;
- control-sequence spelling; and
- source-coordinate construction.

It returns a traced lexical token. It does not resolve the token's current
meaning, inspect conditionals, or decide whether an alignment delimiter starts
a template.

### 4.2 Input stack

The input stack owns an ordered stack of actual input levels:

```rust
enum InputLevel {
    Source(SourceLevel),
    Tokens(TokenLevel),
}
```

`TokenLevel` separates payload, delivery semantics, and descriptive metadata:

```rust
struct TokenLevel {
    payload: TokenPayload,
    behavior: TokenBehavior,
    trace: ReplayTrace,
    cursor: usize,
}

enum TokenPayload {
    Stored {
        tokens: TokenListId,
        origins: OriginListId,
    },
    Transient(Arc<[TracedTokenWord]>),
    MacroArgument {
        tokens: Arc<[TracedTokenWord]>,
        range: MacroArgumentRange,
    },
}

enum TokenBehavior {
    Ordinary,
    MacroBody(MacroActivation),
    MacroArgument,
    AlignmentUTemplate,
    AlignmentVTemplate,
    StopAtEnd,
}
```

The exact final variants should be driven by differences in delivery and
retirement behavior. Reasons such as `EveryPar`, `EveryHBox`, `EveryJob`,
`Mark`, and `OutputRoutine` belong in `ReplayTrace`, unless a demonstrated
semantic distinction requires otherwise.

The input stack performs stored `out_param` substitution because TeX performs
it while reading token-list input in `get_next`. It may expose the resulting
delivery event to `ExpansionProcessor`, but it does not resolve command
meanings.

### 4.3 Expansion processor

The processor groups the state required by one expansion operation:

```rust
pub struct ExpansionProcessor<'a, 'world> {
    input: &'a mut InputStack,
    state: tex_state::ExpansionContext<'a>,
    session: &'a mut ExpansionSessionState,
    policy: ExpansionPolicy<'world>,
}
```

The exact lifetimes may differ after implementation, but scanner APIs should
see one mutable processor:

```rust
impl ExpansionProcessor<'_, '_> {
    pub fn get_next(&mut self) -> Result<Option<CommandToken>, ExpansionError>;
    pub fn get_token(&mut self) -> Result<Option<CommandToken>, ExpansionError>;
    pub fn get_x_token(&mut self) -> Result<Option<CommandToken>, ExpansionError>;
    pub fn get_x_or_protected(
        &mut self,
    ) -> Result<Option<CommandToken>, ExpansionError>;
    fn x_token(
        &mut self,
        token: CommandToken,
        protection: ProtectionPolicy,
    ) -> Result<Option<CommandToken>, ExpansionError>;
    fn expand(&mut self, token: CommandToken) -> Result<(), ExpansionError>;
    pub fn back_input(&mut self, token: CommandToken);
}
```

`CommandToken` is the transient equivalent of TeX's `cur_cmd`, `cur_chr`,
`cur_cs`, and `cur_tok` state:

```rust
pub struct CommandToken {
    traced: TracedTokenWord,
    meaning: Meaning,
    expansion: ExpansionDisposition,
    delivery: DeliveryIdentity,
}

enum ExpansionDisposition {
    Normal,
    SuppressedOnce,
}
```

This state is stack-local and is not serialized into input summaries.
`SuppressedOnce` exists only for the manual's `dont_expand` behavior. Tokens
copied by `\unexpanded` do not retain this disposition after leaving the
expanded-token-list collector.

### 4.4 `get_next`

`get_next` is the only raw semantic delivery path. It:

1. asks `InputStack` for the next traced lexical token;
2. completes token-list parameter substitution;
3. resolves the current command meaning;
4. performs literal-brace `align_state` accounting;
5. intercepts top-level alignment delimiters and template boundaries;
6. enforces scanner-status rules for outer commands; and
7. returns `CommandToken`.

This matches the important property of TeX's `get_next`: callers see command
semantics, not merely a packed token. Conditional skipping and `\ifx`
operands call this same operation, so their alignment behavior cannot drift
from ordinary input.

The durable alignment state may remain snapshot-coupled with `InputStack`.
Meaning-based delimiter classification belongs to `ExpansionProcessor`,
however, because the lexical crate must not resolve meanings.

### 4.5 `get_token`, backup, and delivery identity

`get_token` wraps `get_next` and retains the exact ephemeral delivery identity
needed by `back_input`. The existing allocation-free rewind optimization can
remain, but it becomes an implementation detail of a single backup path.

Backing up a literal brace undoes the corresponding `align_state` change
before the token is reinserted. Callers do not independently classify and undo
alignment delivery.

### 4.6 Expanded-token loop

`get_x_token` is one loop:

```rust
loop {
    let token = self.get_next()?;
    if token.expansion == ExpansionDisposition::SuppressedOnce {
        return Ok(Some(token));
    }
    if !token.meaning.is_expandable() {
        return Ok(Some(token));
    }
    self.expand(token)?;
}
```

The real implementation also performs fuel accounting, undefined-control
recovery, frozen-command conversion, meaning-read recording, and provenance
updates. Those are decorations on this loop, not reasons to create alternative
loops.

`get_x_or_protected` invokes the same internal loop with
`ProtectionPolicy::StopAtProtectedMacro`.

### 4.7 Expansion policy

Driver and restricted operation differ in capabilities:

- whether `\input` may consult an input resolver;
- whether an undefined command is recoverable in the current execution
  context;
- whether reads are recorded; and
- which job enquiries are available.

Represent these as data and optional capabilities on `ExpansionPolicy`.
Do not implement a second token interpreter through `ExpansionMode`.

### 4.8 Direct expansion mutation

`expand` dispatches on the resolved meaning and mutates the active processor:

```rust
fn expand(&mut self, token: CommandToken) -> Result<(), ExpansionError> {
    match token.meaning {
        Meaning::Macro { .. } => self.expand_macro(token),
        Meaning::ExpandablePrimitive(op) => self.expand_primitive(op, token),
        _ => unreachable!("expand receives an expandable command"),
    }
}
```

Macro expansion pushes a macro-body level directly. Text-producing primitives
push their result directly or append it to an active collector. This removes
`Dispatch::Push`, `Dispatch::PushTransient`, and the separate
`apply_dispatch_push` translation.

A small internal outcome may still be useful for primitives that either
continue or deliberately return an unexpandable token. It should not duplicate
the complete input-level representation.

### 4.9 Conditional stack

Move conditional state out of `InputStack`:

```rust
struct ConditionStack {
    frames: Vec<ConditionFrame>,
    next_identity: u64,
}
```

It belongs to persistent expansion-session state and participates in executor
savepoints and durable summaries as required. Stable frame identities remain
necessary because operand expansion may push a nested condition before the
outer condition's `if_limit` is updated.

`pass_text` operates on `ExpansionProcessor` and repeatedly calls `get_next`.
It therefore inherits the same alignment accounting, outer-command checking,
provenance, and end-of-input behavior as every other raw semantic consumer.

### 4.10 Macro calls

`MacroCall` remains a focused component implementing TeX.web §§391--399:

- fixed parameter-text matching;
- undelimited and delimited argument collection;
- overlapping-prefix recovery;
- brace stripping;
- paragraph and outer-command checks; and
- one shared packed argument buffer.

It receives `&mut ExpansionProcessor`, eliminating separate input, state, and
expansion parameters. Macro activation remains attached to the macro-body
input level so nested stored `out_param` tokens can find the nearest live
parameter owner.

### 4.11 Scanners

Scanners become ordinary processor clients:

```rust
pub fn scan_int(
    processor: &mut ExpansionProcessor<'_, '_>,
) -> Result<ScannedInt, ScanIntError>;
```

Each scanner explicitly chooses among a small canonical vocabulary:

- `get_next` for an unexpanded semantic token;
- `get_token` when it may back up that exact token;
- `get_x_token` for normal expanded input; and
- `get_x_or_protected` only where the manual requires it.

Scanner helpers should not dispatch meanings or push expansion results
themselves.

### 4.12 Expanded token-list collection

Expanding `scan_toks` owns a collector state. Results from token-list `\the`
and e-TeX `\unexpanded` append directly to that collector, matching the
manual's `the_toks` splice. Ordinary expansion of those same tokens after the
collector finishes uses normal `get_x_token` behavior.

This removes the need for:

- `TokenListReplayKind::Unexpanded`;
- `TokenListReplayKind::NoExpand`;
- `TracedExpansionToken::expand_in_ordinary_context`; and
- replay-sensitive exceptions spread across the general expansion loop.

The collector must retain the existing special handling for protected macros,
`\expandafter`, compact `out_param` tokens, the inaccessible replacement-text
boundary, and live alignment depth.

## 5. Proposed module layout

```text
crates/tex-lex/src/
    lib.rs
    source.rs
    lines.rs
    tokenizer.rs
    input.rs
    replay.rs
    snapshot.rs
    provenance.rs
    tests/

crates/tex-expand/src/
    lib.rs
    processor.rs
    command.rs
    expand.rs
    macro_call.rs
    conditions.rs
    scan_toks.rs
    scanners/
        mod.rs
        integer.rs
        dimension.rs
        glue.rs
        helpers.rs
    primitives/
        mod.rs
        tex.rs
        etex.rs
        pdftex.rs
    tests/
```

Module boundaries should follow canonical operations and independently
testable state machines. The proposed names are not a requirement if the same
ownership boundaries are achieved.

## 6. State, snapshots, and provenance

The refactor must preserve these existing contracts:

- `InputStackSnapshot` retains all live source backing and replay buffers for
  infallible executor-step rollback.
- Durable `InputSummary` can reopen physical sources and reconstruct token
  levels without retaining host capabilities.
- `ExpansionSessionSnapshot` includes condition state, meaning caches,
  diagnostics, fuel, dependency recording, and collector depth.
- Aggregate executor savepoints capture input, expansion session, universe,
  modes, execution state, generated writes, and output together.
- Direct source positions and arena-backed origins remain opaque to expansion.
- Macro invocation chains and exact replay origins survive input-level
  retirement as currently specified.
- Allocation pooling and shared macro-argument buffers remain valid
  implementation optimizations.

Moving a field between types must not allow it to be snapshotted or restored
independently of the aggregate executor state.

## 7. Migration plan

### Phase 1: Introduce the processor facade

Add `ExpansionProcessor` over the existing `InputStack`,
`tex_state::ExpansionContext`, and expansion context. Convert scanner
signatures and primitive helpers to accept the processor without changing
delivery behavior.

Exit criteria:

- scanner call sites no longer pass three or four expansion-related mutable
  arguments;
- driver and restricted behavior remain byte-for-byte compatible; and
- focused `tex-expand` and `tex-exec` scanner tests pass.

### Phase 2: Establish one `get_next`

Move alignment classification and raw semantic delivery behind
`ExpansionProcessor::get_next`. Route macro calls, condition skipping,
`\ifx`, scanners, and lookahead through it. Retain a crate-private lexical
delivery operation below this boundary.

Delete parallel raw-delivery helpers once all callers use the canonical
operation.

Exit criteria:

- every semantic raw consumer uses `get_next` or `get_token`;
- alignment brace and delimiter accounting has one implementation; and
- read-only inspection is clearly separated from production semantic
  delivery.

### Phase 3: Extract condition state

Move condition frames and frame identities from the input frame collection to
`ConditionStack` in expansion-session state. Update durable summaries and
diagnostic rendering together.

Exit criteria:

- `InputLevel` contains no condition variant;
- conditional skipping still uses stable outer-frame identity; and
- aggregate snapshot, format, and resource-retry tests pass.

### Phase 4: Normalize replay representation

Split token payload, delivery behavior, and trace reason. Collapse replay
reasons that have identical delivery behavior.

Preserve macro-body, macro-argument, alignment-template, stop-at-end, and
output-routine retirement behavior where the distinction is semantically
required.

Exit criteria:

- adding a new `\every...` source does not require modifying token delivery;
- trace labels do not control expansion; and
- summaries serialize semantic behavior separately from optional trace data.

### Phase 5: Remove replay-based expansion suppression

Implement `\noexpand` as `ExpansionDisposition::SuppressedOnce`. Implement
`\unexpanded` and token-list `\the` as direct expanded-collector splices.

Exit criteria:

- no input replay kind represents `\noexpand` or `\unexpanded`;
- ordinary expansion has no historical-suppression flag; and
- TRIP's nested `\message` and token-register cases remain byte-identical.

### Phase 6: Make `expand` mutate directly

Convert macro and primitive expansion to processor methods that push input or
append to collectors directly. Remove push-bearing `Dispatch` variants and
`apply_dispatch_push`.

Replace `ExpansionMode` with explicit processor policy and capabilities.

Exit criteria:

- there is one ordinary expanded-token loop;
- expansion results no longer mirror the input-level representation; and
- restricted helpers cannot acquire input-opening authority.

### Phase 7: Split modules and tests

After ownership has stabilized, split the large source files according to the
target module layout. Reorganize tests around canonical routines rather than
around historical regression accumulation.

Do not combine mechanical file movement with semantic changes. Each movement
should preserve history and leave the relevant focused test suite passing.

## 8. Validation

Each phase should run focused tests first:

```text
cargo test -q --tests -p tex-lex
cargo test -q --tests -p tex-expand
cargo test -q --tests -p tex-exec
```

Then run:

```text
scripts/check.sh
```

Milestones that change expanded-token-list behavior, macro calls, conditions,
alignment delivery, summaries, or format restoration must also run the
relevant `umber` integration tests. Before removing the old suppression and
replay paths, run the pinned TRIP and e-TRIP workflows described in
`docs/trip.md` when their local oracle inputs are available.

Performance validation should use the established in-process Gentle profiling
workflow. The refactor should reduce duplicate branching and wrapper traffic,
but performance claims must be based on the controls in `docs/profiling.md`,
not on source-line reduction.

## 9. Non-goals

- Reproducing TeX's global variables or memory-word representation.
- Removing typed errors, resource suspension, expansion fuel, provenance, or
  incremental snapshots.
- Combining expansion and stomach execution.
- Pre-tokenizing source lines across catcode changes.
- Replacing the shared macro-argument buffer with per-parameter copies.
- Introducing a generic scanner trait hierarchy.
- Treating fewer files or fewer lines as sufficient evidence of a simpler
  architecture.

## 10. Expected result

After the migration, a reader should be able to map the implementation to the
manual directly:

| Canonical operation                          | Primary implementation           |
| -------------------------------------------- | -------------------------------- |
| line normalization and lexical states        | `tex-lex::lines` and `tokenizer` |
| input-level selection and `out_param` replay | `tex-lex::input`                 |
| `get_next` and `get_token`                   | `tex-expand::ExpansionProcessor` |
| `get_x_token`, `x_token`, and `expand`       | `tex-expand::ExpansionProcessor` |
| `macro_call`                                 | `tex-expand::macro_call`         |
| `cond_ptr` and `pass_text`                   | `tex-expand::conditions`         |
| `scan_toks`                                  | `tex-expand::scan_toks`          |
| numeric and glue scanners                    | `tex-expand::scanners`           |

The most important simplification is that new correctness fixes have one
obvious owner. A change to raw semantic delivery modifies `get_next`; a change
to recursive expansion modifies `get_x_token` or `expand`; a change to
conditional skipping modifies `pass_text`; and a change to macro argument
matching modifies `macro_call`.
