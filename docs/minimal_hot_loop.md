# Minimal hot loop

Status: proposed architecture. Microbenchmarks use the frozen binary from the
`23548ce50` full-book profile; the source was read at `ea2b720e6`. Tracking:
Beads `umber2-efje` (design) and `umber2-i0n5` (implementation).

This proposal replaces the input, expansion, and main-control core with a
machine shaped like TeX's own:

- one token memory;
- a dense table of packed meanings;
- a stack of 16-byte frames;
- a TeX save stack.

Checkpoints, resource replay, provenance, observation, and fuel move off the
per-token path. They attach to boundaries the engine already has: frame pushes,
command dispatch, and quiescent checkpoints.

It subsumes the first and third proposals in
[Book performance refactors](book_performance_refactors.md) and absorbs the
second. Those proposals keep today's owners and thin them. This one removes
them.

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

So the input stack, parameter stack, save stack, scanner state, and mode nest
need no journal at all. They are small at every checkpoint, so they can be
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

```rust
struct Machine {
    // Long-lived state, journaled first-touch per checkpoint interval.
    mem: Vec<u32>,              // token memory: heap grows up, transient grows down
    eqtb: Vec<u64>,             // meanings, registers, parameters, codes
    names: CsTable,             // append-only control-sequence names
    // Transient state, small or empty at every checkpoint.
    top: Frame,                 // TeX's cur_input
    frames: Vec<Frame>,         // TeX's input_stack
    frame_cold: Vec<FrameCold>, // call names, call sites, completion fences
    params: Vec<Span>,          // TeX's param_stack
    collector: Vec<u32>,        // token lists under construction
    save: Vec<SaveEntry>,       // TeX's save_stack
    cond: Vec<CondEntry>,       // TeX's condition stack
    nest: Vec<ModeRecord>,      // TeX's semantic nest
    // Control.
    cur_level: u16,
    align_state: i32,
    scanner: ScannerStatus,
    slow: u8,
    budget: i64,
    halt: Option<HaltReason>,
}
```

### Token memory

One `Vec<u32>` holds every stored token, as TeX's `mem` does:

- The **heap** grows up from offset zero.
- The **transient stack** grows down from the end.
- When the two meet, the vector grows. The transient region moves to the new end
  and transient offsets in frames and the parameter stack are rebased. That is
  O(stack depth), amortized by doubling.
- Reading a token is `mem[pos]` whatever owns it.

#### Heap spans

A heap span holds a definition, a token register or parameter, a mark, a
`\write` list, an alignment template part, or `\output`. Its header is four
words:

```text
[rc][hold][born:16 | logged:16][shape]   then words
```

- **`rc`** counts persistent holders: eqtb cells, save entries, interval logs,
  and checkpoints.
- **`born` and `logged`** are epoch stamps for checkpoint intervals (see
  Checkpoints).
- **`shape`** encodes the length and, for a definition, the parameter count, the
  parameter-text length, and the offset of its matcher plan.
- **`hold`** counts the frames reading the span. It is never journaled. A span
  is free when `rc` and `hold` are both zero.

Allocation and freeing:

- Spans are allocated from size-class free lists, with a bump tail.
- Spans from the loaded format lie below `format_mark` and are immortal.
  Reference operations skip them with one compare, so the format prefix stays
  read-only and shareable, as
  [fork-arena dense prefixes](fork_arena_dense_prefix_emplacement.md) need.

#### Transient stack

The transient stack holds macro arguments, backed-up and inserted tokens, and
`\the` and `\number` results.

- It is LIFO by region. Retiring the top region truncates the stack.
- Retiring a lower region marks it dead, and dead regions are reclaimed when the
  stack unwinds past them.
- A macro call in tail position compacts its new arguments down over a dead
  region beneath them. Recursive `\loop` patterns therefore run in constant
  transient space.

#### Collector

The collector is a separate LIFO `Vec<u32>` for lists under construction:

- definitions, `\edef`, `\toks`, `\message`, `\write`, `\mark`, and
  `\csname` names;
- nested collection pushes a sub-region.

Sealing a list copies it once into an exactly sized heap span. Consumers that
do not store the list read it in place: `\message`, `\csname`, and immediate
`\write`.

#### What this replaces

- `DefinitionArena`, with its regions, chunk handles, leases, and
  local-definition slots;
- `TokenListArena`;
- the attempt arena and its promotion path;
- the command scratch argument store;
- all six storage domains.

Arguments become contiguous copies, not TeX's per-token linked lists, so
argument handling can beat pdfTeX.

### Packed meanings

eqtb is a dense `Vec<u64>` indexed by control-sequence id and by TeX's
parameter, register, and code regions. Every cell has one layout:

```text
bits  0..8   cmd     TeX command code, ordered so expandable codes sort last
bits  8..16  flags   MACRO | LONG | OUTER | PROTECTED | SLOW
bits 16..32  level   TeX eq_level (xeq_level for integer and dimension cells)
bits 32..64  equiv   chr code, register value, font, or heap offset of a definition
```

Classification:

- `cmd >= FIRST_EXPANDABLE` is TeX's `cur_cmd > max_command`.
- `flags & (OUTER | SLOW)` is the only extra test the reader needs.
- A character token does not touch eqtb. Its meaning is
  `CHAR_MEANING[cat] | ch << 32`, except that active characters read their eqtb
  cell.

Assignment is TeX's `eq_define`, `eq_word_define`, and `geq_define`, plus a
first-touch test:

```rust
#[inline(always)]
fn eq_define(&mut self, p: Cell, w: u64) {
    let old = self.eqtb[p];
    if level(old) == self.cur_level {
        self.release(old)
    } else if self.cur_level > LEVEL_ONE {
        self.save.push(SaveEntry::restore(p, old))
    }
    self.touch(p, old); // first write this interval? log (p, old), moving its reference
    self.eqtb[p] = with_level(w, self.cur_level);
}
```

- Integer and dimension cells keep their `xeq_level` semantics in the level
  field.
- e-TeX's 32,768 registers of each class are dense: about 1.5 MB in all.
- Glue cells point at reference-counted specifications in node memory, as in
  TeX.

This replaces:

- the `StateCell`, `StateWord`, and `BankCell` enums;
- the admission chain, `Universe` to core to admitted state to bank, with a
  `Result` at each step;
- `meaning_word` decoding;
- the shared save-and-rollback mutation journal.

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

```rust
#[inline(always)]
fn macro_call(&mut self, cs: u32, m: u64) {
    let d = equiv(m);
    let h = self.header(d);
    if h.parameter_text_len() != 0 {
        return self.macro_call_match(cs, m, d, h); // warm, not inlined
    }
    self.conserve_frames(); // §390: retire exhausted list frames
    if h.body_len() == 0 {
        return;
    }
    self.hold(d); // one compare against format_mark, one increment
    self.push(Frame::body(d, h.body(), self.params.len()));
    self.frame_cold_name(cs);
    self.charge(h.body_len()); // fuel; the budget test is the poll
}
```

`macro_call_match` follows the definition's matcher plan. Sealing a definition
builds the plan once, from TeX's parameter text:

- **Undelimited parameters** skip spaces, then either copy one token or copy a
  balanced group without its outer braces (§392 to §399).
- **Delimited parameters** run KMP over the delimiter words, using failure links
  stored in the plan.
- **Leading literal text** is compared word by word.

Matched arguments go straight to the transient stack and `params`:

- Copies from list frames run over contiguous words, stopping only at braces,
  special words, `\par`, or the frame end.
- The `\par` check for a non-`\long` macro is one compare against `par_token`.
- Outer validity is checked only while some `\outer` definition exists. A
  counter maintained by `eq_define` makes the usual case free.

TeX's saving of `scanner_status` and `warning_index` around the call becomes two
locals. That replaces the scanner-episode object, the builder identity, and the
`MacroPlan`.

### Retirement

| Kind              | Retirement                                                                 |
| ----------------- | -------------------------------------------------------------------------- |
| Body              | release the hold; truncate `params` to `base`; release the argument region |
| Param             | nothing                                                                    |
| Backup, Inserted  | release the region                                                         |
| Stored            | release the hold                                                           |
| `COLD_RETIRE` set | cold handler                                                               |
| Source            | cold handler: end of file, `\endinput`, `\everyeof`, closing               |

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

`new_save_level` pushes one boundary entry and increments `cur_level`. Each
entry is 16 bytes:

```text
SaveEntry { cell: u32, kind: u8, level: u16, word: u64 }
```

`unsave` pops entries until it reaches the boundary. For each entry:

- If the cell was not set globally in the meantime, it releases the current
  value and restores the saved word.
- `\aftergroup` tokens sit in the save stack and are pushed as a Backup region
  when the group ends.

Definitions made inside a group are ordinary heap spans. A group allocates
nothing and touches no definition storage.

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

The three checkpoint kinds and their triggers do not change: job start, eligible
outer paragraph end, and completed shipout. Neither do:

- [aggregate checkpoint](aggregate_checkpoint_contract.md) marks and restore
  order;
- [resource replay](checkpoint_resource_replay.md);
- the host's `CheckpointSink`.

What changes is how each component is captured.

### Quiescence

Today's predicates already guarantee the following:

- **Expansion and scanning:** no active expansion depth, no macro bodies on the
  input stack, no replay completions, `ScannerStatus::Normal`, and no pending
  diagnostics. See `validate_summary_quiescence` and
  `validate_format_dump_quiescence` in
  `crates/tex-command/src/snapshot/boundary.rs`.
- **Builders and alignment:** no token builders, attempt state, rollback roots,
  or alignment.
- **Groups:** no active groups, from `checkpoint_eligible` in the save journal.
- **Mode:** one vertical level, from `restart_checkpoint_is_quiescent` in
  `crates/tex-exec/src/mode.rs`.

Two things may remain at a checkpoint:

- open conditionals;
- non-macro token lists, such as a backed-up token or an inserted `\par`.

### What a checkpoint holds

| Component                                 | At a checkpoint                          | Capture                                                   | Hot-path cost                                           |
| ----------------------------------------- | ---------------------------------------- | --------------------------------------------------------- | ------------------------------------------------------- |
| Frames and `frame_cold`                   | source frames and a few non-macro lists  | by value, with the transient words those frames reference | none                                                    |
| `params`, collector, save stack           | empty                                    | asserted empty                                            | none                                                    |
| Condition stack                           | a few entries                            | by value                                                  | none                                                    |
| Mode nest                                 | one vertical record                      | by value; its list belongs to node memory                 | none                                                    |
| Scanner, `align_state`, `cur_level`, misc | fixed                                    | by value                                                  | none                                                    |
| eqtb                                      | arbitrary                                | interval log of first-touch pre-images                    | one test per write                                      |
| Control-sequence names, strings, fonts    | append-only                              | high-water marks                                          | none                                                    |
| Token heap                                | arbitrary                                | epoch state, a first-touch reference log, and quarantine  | one compare per `rc` change and per free of an old span |
| Node memory                               | page contents, box registers, insertions | the same epoch scheme                                     | the same                                                |
| Output, log, and diagnostics              | buffered effects                         | length marks, as today                                    | none                                                    |

The capture is O(open files + open conditionals + live transient words), a few
hundred bytes. Restoring copies the same bytes back.

### Interval logs

Each checkpoint begins an interval. Within an interval:

- **eqtb.** A dirty bitmap marks touched cells. The first write to a cell pushes
  `(cell, old word)` to the interval log, and the log entry takes over the old
  word's heap reference. Later writes in the interval are plain stores. A new
  interval clears exactly the bits its predecessor's log names.
- **Heap and node references.** The first `rc` change to an old span (`born`
  earlier than the current epoch) pushes `(span, rc)` and stamps `logged`.

Frame holds never change `rc`, so macro activation touches no log. That is why
`hold` and `rc` are separate counts. TeX folds them together, but at a
checkpoint every hold not owned by a captured frame is zero.

### Heap and node epochs

Allocation in interval `k` prefers, in order:

1. young free lists, holding blocks born and freed in `k`;
2. the per-class stacks of old free blocks;
3. the bump tail.

The interval records the old stacks' low-water marks and the tail at its start.

Freeing:

- A block born in `k` returns to a young free list immediately.
- An older block goes to `quarantine[k]` with its contents intact.

At the next checkpoint:

- young free lists merge into the old stacks;
- the entries the merge overwrites are saved with interval `k`, which costs
  O(allocations from the old stacks).

A quarantined block becomes reusable only when the host drops every retained
checkpoint that could resurrect it.

Quarantine grows by at most the number of old spans freed in each interval. That
is bounded by the first-touch log volume the aggregate contract already charges.
LaTeX's churn of temporaries such as `\reserved@a` frees mostly young spans,
because only the first redefinition in each interval frees an old one.

[Node word arena](node_word_arena.md) storage adopts the same contract for
nodes:

- page building and `\vsplit` first-touch the old page nodes they mutate;
- shipped pages enter quarantine.

### State identity

The commutative state hash moves from every write to checkpoint time. For each
log entry of the closing interval, the checkpoint adds the cell's hash under its
current word and subtracts its hash under its old word.

For a cell that refers to a span, the hash uses the span's content hash rather
than its offset. Sealed spans are immutable, so the content hash is computed on
demand and memoized in the header.

The cost is proportional to the cells touched in an interval, not to the writes.
A cell that is written and then restored to its old value contributes nothing.

### Restore and redo

Rewinding interval `k` takes five steps:

1. Swap each eqtb log entry with its cell, in reverse. References move with the
   words in both directions, so no reference counts change.
2. Swap logged `rc` values back.
3. Return quarantined blocks to life. Their counts are restored and their
   contents never changed.
4. Reset the bump tail and old-stack lengths to their interval-start values.
   This frees every block born in `k` at once, with no per-block work.
5. Copy the captured transient components back.

Because restoration swaps, the same logs serve redo of the accepted head. Redo
is valid until new work runs, the same rule as today's aggregate head.

Rewinding several intervals applies these steps per interval in reverse order.
That is the positional-mark model, with fewer and cheaper logs.

### Resource replay

A resource miss:

- sets `halt`;
- returns an empty file or `nullfont` to its caller;
- is reported at the next poll.

The host then restores the latest full checkpoint and applies the answer, as
`restore_pending_resource` does today. No per-command unit is needed: the
operation in progress is discarded with everything else after the checkpoint.
The synchronous provider path still resolves in place without halting.

### Soundness

Three invariants carry the contract:

1. **Every component that can differ between a checkpoint and a later state is
   captured or logged.** Transient components are empty or copied by value at
   quiescence. Long-lived components log the first pre-image of each cell and
   span reference in an interval. Append-only components keep high-water marks.
2. **Every byte a restore needs is preserved until no retained checkpoint can
   need it.** Interval logs own the references they saved. Quarantine keeps old
   freed blocks intact. Reclamation follows checkpoint retention, which the host
   already manages.
3. **Nothing a halted run does is observable.** Effect sinks refuse commits
   after `halt`, and every halt ends the job or restores a checkpoint.

The tests for these invariants already exist as owner checks: restore cursors,
releasing each owner exactly once, and resource replay. They move from
per-component journals to the interval log and the by-value capture.

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
acceptance thresholds for the microbenchmark harness:

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

## What is deleted

- **Input:** `InputLevel` and its rows, `MacroBodyCursor`, `ResidentTokenRow`,
  `InputStack` history, displaced-row slabs, input-level identities and their
  timeline records, `ResidentWord`, `ResidentWordRead`, `ResidentColdOutcome`,
  `InputRetirement`, and `RetirementHandoff`.
- **Commands:** `TokenMeaning`, `ExpansionCandidate`, `HotCommand`, and
  `CurrentCommand` as a hot-path value, with its settlement and classification
  stages.
- **Storage:** `DefinitionArena` regions, chunk handles, region leases,
  `LocalDefinitionSlots`, `TokenListArena`, the attempt arena and its
  promotion path, and `TracedTokenWord` on stored paths.
- **Macro calls:** `MacroPlan`, scanner episodes for matching, argument builder
  identities, and per-call failure links.
- **Commands and checkpoints:** `DirectOperationMark`, cold-episode preparation
  and promotion, per-command journals in the mode nest and durable boxes, the
  combined save and rollback mutation journal, and the per-write state hash.
- **Timeline marks:** the command timeline's marks for input, conditions,
  groups, after-group tokens, and alignment.

## Migration

Each stage changes an owning representation and deletes its predecessor within
the same stage. Adapters that keep old shapes alive are what held earlier work
to about one percent per layer. A stage may keep a temporary materializing shim
for unported consumers, but it must delete that shim before it closes.

**Stage 0. Harness.**

- Commit the microbenchmark harness, `scripts/interp-microbench.py`. It needs:
  - the case table and the oracle environment;
  - a JSON receipt;
  - the per-operation report against the targets above.
- Record the starting receipt.
- Counter access needs privileges, so the harness is a measured review
  requirement, not part of the routine test suite.

**Stage 1. Error and halt contract.**

- Make every recoverable error recover in place.
- Introduce the halt reasons and the budget poll.
- Route fuel, cancellation, capacity, and resource misses through halt.
- Gate effect sinks on halt.
- Delete `DirectOperationMark`, per-command rollback in every owner, and the
  attempt arena's rollback roots.
- Gates:
  - resource replay and incremental tests pass;
  - canonical fixtures match;
  - `group` at most 2,000;
  - `toks` at most 8,000.

**Stage 2. By-value transient capture.**

- Capture frames, conditions, the mode top, and scanner fields by value at the
  three checkpoint kinds.
- Delete the input history journal, displaced rows, input identities, and the
  timeline marks for transient components.
- Gates:
  - checkpoint, aggregate, and snapshot-performance gates pass;
  - `mbody` at most 1,200.

**Stage 3. Token memory and packed meanings.**

- Introduce `mem` with its heap, transient stack, and collector.
- Introduce the `u64` eqtb, holds and reference counts, epochs, quarantine,
  first-touch logs, and checkpoint-time hashing.
- Move definitions, registers, marks, templates, and output routines into the
  heap.
- Delete the definition arena's regions, `TokenListArena`, promotion, and the
  per-write hash.
- Gates:
  - `def`, `let`, `toks`, and `group` reach their targets;
  - the aggregate retention charges hold.

**Stage 4. Frames, reader, and expansion.**

- Introduce 16-byte frames, the fused reader, `macro_call`, matcher plans, and
  the new scanner interface over `(tok, meaning)`.
- Port consumers family by family behind the shim, then delete the shim and the
  `ResidentWord`-to-`HotCommand` chain.
- Gates:
  - the macro, `relax`, `ifx`, `expandafter`, `csname`, and `base` targets.

**Stage 5. Main control.**

- Introduce the dense dispatch, the save stack, and direct prefixed commands.
- Delete the cold-episode protocol and the lane's eligibility split.
- Gates:
  - `group`, `advance`, and `count` targets;
  - the full-book exact-output gate.

**Stage 6. Layout.**

- Introduce the hot-set budget, cold attributes, removal of the coordinate
  generic, and profile-guided and post-link layout in production.
- Gates:
  - L1 instruction-cache and iTLB misses within four times pdfTeX's;
  - full-book cycles.

**Stage 7. Node memory epochs.**

- Bring [node word arena](node_word_arena.md) storage under the same checkpoint
  contract.
- Delete per-command page and box journals.

The ordering is deliberate:

- Stages 1 and 2 remove the contracts that force journals before the storage
  they protect is replaced, so Stage 3 does not reimplement journals it is
  about to delete.
- Stages 3 and 4 are the core of the rewrite.
- Stages 5 to 7 rest on them.

Every stage runs the owning tests, `cargo test -q --tests`, `scripts/check.sh`,
and the canonical divergence workflow for any semantic difference.

## Risks

- **Reader and matcher fidelity.**
  - Risk: TeX's §341 to §399 cover many cases: `\outer` checks, `\par`
    insertion, the `#{` delimiter, `\noexpand` markers, template ends, and
    `\endinput`.
  - Mitigation: port section by section with `tex.web` references, and use the
    committed oracle fixtures and
    [canonical divergence workflow](canonical_divergence_workflow.md).
- **Bounds checks.**
  - Risk: every `mem` read in safe Rust carries a check.
  - Mitigation: iterate within a frame using slices. If a gate still fails, an
    audited substrate crate under the existing `unsafe` exception can own the
    token memory, as `tex-dense-prefix` owns its storage.
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
  - Risk: quarantine and logs grow with retained checkpoints.
  - Mitigation: both are charged against retention like today's journals. The
    host's checkpoint dropping policy reclaims them.
- **Concurrent work.**
  - Risk: the book refactors are being implemented in `slot-1`, and their first
    stage edits the same reader.
  - Mitigation: coordinate before Stage 4. Work that thins today's owners does
    not carry over.

## Acceptance

- **Per-operation gates.** Each stage meets its gates on the committed harness,
  and the final state meets every target in the budget table.
- **Full-book gates.** The full book passes the gates in
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
