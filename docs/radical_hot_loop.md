# Radical hot loop: the flat machine

Status: proposal with a measured prototype, 2026-10-07. Tracking: Beads
`umber2-gbyl`. The prototype is `tools/flat-machine-prototype`.

This document proposes replacing Umber's token-level core with one flat
machine. The core is the reader, expansion, macro activation, scanners,
assignments, and groups. The proposal keeps the restart semantics of
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
and supersedes the token-core parts of [Minimal hot loop](minimal_hot_loop.md).

## Summary

On the interpreter microbenchmarks, Umber spends up to 3.9× pdfTeX's marginal
instructions per case, 1.7× on the base loop, and 2.3–2.9× its cycles per
iteration. On arXiv book 2606.24937 the figures are 3.6× instructions and 4.4×
cycles. A one-argument macro costs 918 marginal instructions against
pdfTeX's 421.

The gap has no hotspot. It is a representation tax paid on every token:

- generic consumer kernels;
- `Rc` and `RefCell` ownership;
- argument regions with roots and retained source lanes;
- 120-byte input rows;
- generation and host generics monomorphized into 26 MB of text;
- `Result` edges on every read.

Recent incremental stages under `umber2-66p0.8.40.208.34.10` moved the book
from 628.87G to 613.58G instructions, 2.4% over three stages. The data model is
the limit.

The flat machine stores every hot datum as a machine word in one of five flat
arrays:

1. the eqtb;
2. its epoch/level metadata;
3. token memory;
4. the input frames;
5. a LIFO bank for arguments and backed-up tokens.

It reads stored token lists in place. It builds arguments once, where they are
consumed. It pays for checkpoints only on the first write to a cell in each
epoch. Rare work goes through one attention word and one cold path.

A 1.6k-line, `forbid(unsafe_code)` prototype runs the interpreter microbenchmark
document and checks every case against a checkpoint replay. Its live path
includes first-touch checkpoint undo, provenance lanes, fuel, reference counts,
and an attention word. It achieves:

- 0.34–0.64× pdfTeX's instructions on every case;
- 0.34–0.50× pdfTeX's cycles on the four cases measured for cycles;
- IPC of 2.5–3.2, from a hot core of about 21 KB of text.

Every case replays from a checkpoint to a byte-identical eqtb, metadata, and
token memory, and re-executes to identical final meanings.

## Evidence

### Measurements

Marginal user-space instructions per microbenchmark iteration, using the
`scripts/interp-microbench.py` method. The method takes the difference between
250k and 50k iterations, then subtracts the base loop. The base row is the
whole loop.

- Umber is a release build of the working tree at `89812e66e`.
- pdfTeX is the clean 1.40.29 oracle.
- The prototype uses the `fat` profile (fat LTO, one codegen unit, abort on panic).

| Case         | pdfTeX | Umber | Prototype | Prototype/pdfTeX |
| ------------ | -----: | ----: | --------: | ---------------: |
| base loop    |   3814 |  6655 |      1697 |             0.44 |
| `relax`      |     91 |    43 |        34 |             0.37 |
| `mzero`      |    216 |    53 |        73 |             0.34 |
| `mbody`      |    398 |   527 |       233 |             0.59 |
| `mbody64`    |   6040 |  3237 |      2341 |             0.39 |
| `mone`       |    421 |   918 |       197 |             0.47 |
| `mone_brace` |    709 |  1041 |       295 |             0.42 |
| `mtwo`       |    975 |  2488 |       624 |             0.64 |
| `mdelim`     |    781 |  1861 |       488 |             0.62 |
| `let`        |    349 |  1136 |       125 |             0.36 |
| `def`        |    866 |  3150 |       479 |             0.55 |
| `group`      |    283 |   312 |       131 |             0.46 |
| `brace64`    |  19776 | 18216 |      8896 |             0.45 |
| `group_let`  |    708 |  2783 |       356 |             0.50 |
| `advance`    |   1150 |  2281 |       429 |             0.37 |
| `count`      |    876 |  1941 |       397 |             0.45 |
| `toks`       |   1390 |  5477 |       822 |             0.59 |

Cycles and instructions per whole iteration (loop included), all three engines
on the same host and CPU:

| Case        | Engine    | Cycles | Instructions |  IPC |
| ----------- | --------- | -----: | -----------: | ---: |
| `base`      | pdfTeX    |   1372 |         3814 | 2.78 |
| `base`      | Umber     |   3179 |         6655 | 2.09 |
| `base`      | prototype |    682 |         1697 | 2.49 |
| `mone`      | pdfTeX    |   1441 |         4235 | 2.94 |
| `mone`      | Umber     |   3858 |         7572 | 1.96 |
| `mone`      | prototype |    588 |         1894 | 3.22 |
| `def`       | pdfTeX    |   1733 |         4680 | 2.70 |
| `def`       | Umber     |   4968 |         9804 | 1.97 |
| `def`       | prototype |    796 |         2176 | 2.73 |
| `group_let` | pdfTeX    |   1962 |         4522 | 2.30 |
| `group_let` | Umber     |   4730 |         9438 | 2.00 |
| `group_let` | prototype |    669 |         2053 | 3.07 |

At whole-document scale the gap is larger, because Umber's code does not fit
the instruction caches:

- Book 2606.24937: pdfTeX retires 169G instructions in 97G cycles; Umber
  retires 614G in 429G.
- Umber's text segment is 26.2 MB, against 2.5 MB for the pdfTeX oracle.
- The historical baseline in [Minimal hot loop](minimal_hot_loop.md) recorded
  65× pdfTeX's L1i misses and 376× its iTLB misses.

The non-test sources of `tex-state`, `tex-command` and `tex-exec` total about
203k lines.

### Why the gap is structural

A line-level profile of `\mone x` (debug-info build of `89812e66e`) and code
reading show the same pattern in every case. The time goes to plumbing between
representations, not to TeX's work.

- **Generic consumer kernels.**
  - `consume_resident_input` in
    `crates/tex-command/src/processor/expand/resident.rs` takes six closures,
    one of them a sixteen-argument admission callback.
  - Each span re-matches `InputStorage` and re-derives its words, delivery
    kind, start, and suppression flag.
  - Every boundary token goes through a `finish` closure. That one call
    carries 14.6% of the `\mone x` samples.
- **Ownership by `Rc`.**
  - Macro activation clones an `Rc<DefinitionPayload>` into
    `ResidentMacroBody`.
  - Definitions are `DefinitionEntry(Rc<DefinitionPayload>)` rows inside
    `DefinitionRoots(Rc<RefCell<DefinitionBank>>)`, so every meaning write
    acquires and releases roots through a `RefCell`.
  - Stored lists are `TokenListId` values that own an `Rc<[TokenWord]>`.
- **Arguments as a general allocator.**
  - An `ArgumentArena` region carries nine argument values, source lanes,
    retained source slots, root counts, and published raw words.
  - Each parameter runs `begin_argument`, scan, `publish_argument`, and a
    tracing check. Each delimited run builds a fresh `ArgumentRunAdmission`.
- **Wide rows.**
  - An `InputLevel` is 120 bytes: a 72-byte storage enum and a 40-byte header.
    TeX's whole input state record is six halfwords.
  - `MeaningWord<G>` is 16 bytes, and every lane carries `TracedTokenWord`.
- **Monomorphization.**
  - Hot code is generic over the generation brand `G` and over
    `Facts: CommandHostFacts<G>`.
  - Each instantiation duplicates the reader, so the hot working set never
    becomes small enough to stay resident.
- **Failure edges.**
  - Every read returns a `Result`, so each token carries a discriminant test
    and an early-return path.
  - Recent work shrank `CommandError` from 48 to 16 bytes. The edges remain.
- **Split ownership.**
  - `CommandProcessor` borrows the command state, the TeX state, the host,
    the observer, fuel, and diagnostic effects separately.
  - Hot fields sit behind several pointers, and the optimizer cannot keep them
    in registers across calls.

Where Umber already has a direct resident path, it beats pdfTeX: `relax` costs
43 instructions against 91, and `mzero` costs 53 against 216. The machine is
fast when the data model gets out of the way. The proposal makes that the
_only_ path.

## Design laws

1. **One machine.** All hot state lives in one `Machine` struct in one module.
   - No generic parameters, trait objects, `Rc`, `RefCell`, `Box`, or enums
     with payloads appear in hot state.
   - Generation brands, host capabilities, and observers are checked at the
     cold API boundary, not threaded through the reader.
2. **Everything is a word.**
   - Tokens are `u32`. Meanings, register values, and save records are `u64`.
   - Every reference is a `u32` offset into a flat array owned by the machine.
3. **Nothing is built only to be consumed.**
   - Consumers read the producer's storage in place.
   - The only copies are the ones TeX itself makes: arguments, backed-up
     tokens, and newly scanned lists.
4. **Checkpoints are paid on writes, once per cell per epoch.**
   - Reads never pay.
   - A repeated write at the same level in the same epoch pays one compare.
5. **Rare is cold.**
   - One attention word gates tracing, interrupts, `\afterassignment`,
     `\globaldefs` and observers.
   - Fatal failures are in-band halts, not `Result` edges on the token path.
6. **Code size is a budget.**
   - The hot core is measured in kilobytes, with a gate: at most 64 KB of
     text, against about 21 KB in the prototype.

## The machine

```rust
struct Machine {
    // Input: the current frame lives inline, below-frames in a stack.
    cur: Frame,
    stack: Vec<Frame>,
    mem: Vec<u32>,  // token memory: every stored list
    lifo: Vec<u64>, // arguments and backed-up tokens: token | origin << 32
    ps: Vec<u32>,   // per invocation: region start, then (start, end) pairs
    lifo_pin: u32,
    dead_floor: u32,
    align_state: i32,
    scanner_status: u8,
    // Equivalents: one bank, parallel metadata.
    eqtb: Vec<u64>,
    meta: Vec<u64>, // epoch << 32 | level
    cur_meta: u64,  // current epoch << 32 | current level
    cur_level: u32,
    cur_group: u8,
    cur_boundary: u32,
    save: Vec<Save>, // 16-byte records
    // Checkpoint history.
    epoch: u32,
    undo: Vec<Undo>,
    mark: u32,
    free: Vec<u32>,
    deferred: Vec<u32>,
    // Conditionals, budgets and rare-feature gate.
    cond: Vec<Cond>,
    if_limit: u8,
    cur_if: u8,
    fuel: u64,
    attention: u32,
    // Cold owners (nodes, fonts, page, PDF, world) are reached through
    // `#[cold]` calls with explicit arguments, never through generics.
}
```

TeX's own globals map onto these fields almost one to one:

- `cur_input` is `cur`, and `input_stack` is `stack`.
- `mem` for token lists is `mem`.
- `param_stack` becomes `lifo` plus `ps`.
- `eqtb` with `xeq_level` becomes `eqtb` plus `meta`.

The machine adds three fields TeX lacks: the epoch, the undo log, and the token
memory mark. They exist for checkpoints.

## Tokens and meanings

### Token words

A token is one `u32` in TeX's own order:

| Token              | Encoding                                | Notes                                          |
| ------------------ | --------------------------------------- | ---------------------------------------------- |
| Character          | `cat << 21 \| ch`                       | `ch` up to 21 bits; categories 1–4, 6–8, 10–12 |
| Out-parameter `#n` | `5 << 21 \| n`                          | Only inside stored lists                       |
| Match `#n`         | `13 << 21 \| n`                         | Only inside parameter texts                    |
| End of match       | `14 << 21`                              | Only inside parameter texts                    |
| Control sequence   | `CS_FLAG \| p` with `CS_FLAG = 1 << 25` | `p` is the eqtb index of its meaning           |

Every character token sorts below every control sequence. Braces sort below
`3 << 21`. So TeX's range tests (`t < right_brace_limit` and
`t >= cs_token_flag`) are single compares. Categories 0, 5, 9, 13, 14 and 15
never occur in character tokens, so codes 5, 13 and 14 are free for the
list-internal markers, as in TeX.

Frozen control sequences (`\endtemplate`, the frozen `\relax` and `\fi`,
`\inaccessible`, the halt token) get reserved eqtb slots. They are ordinary control
sequences, and need no separate token kind.

### Meaning words

An eqtb word is one `u64`:

- bits 0–7: command code, in TeX's order;
- bits 8–31: flags;
- bits 32–63: operand.

| Meaning              | Operand                                              |
| -------------------- | ---------------------------------------------------- |
| Macro, `\toks` value | Token-memory offset of the list header               |
| Primitive            | TeX's `chr_code`                                     |
| Character `\let`     | Character code; the command is the category          |
| Register reference   | Register index                                       |
| Font, glue, box      | Index into the font table, glue table, or node arena |

`\long` and `\outer` are distinct command codes, as in TeX. `\protected` is a
list-header flag.

The command byte decides everything the reader needs:

- `cmd <= max_command` returns to the caller.
- `cmd >= call` activates a macro.
- `cmd >= outer_call` diverts to one cold handler. That handler covers outer
  macros, `\endtemplate`, `\noexpand` markers, and halts.

This replaces `MeaningWord<G>` (16 bytes) and the definition-key indirection.

### One eqtb

TeX's §222 regions become one `Vec<u64>`:

1. active characters;
2. single-character and hashed control sequences;
3. frozen control sequences;
4. glue, token-list and box parameters;
5. box registers;
6. fonts and families;
7. code tables;
8. integer and dimension parameters;
9. registers 0–255.

Slots below `RAW_BASE` hold meanings that may own references. Slots above hold
raw values. Extended registers 256–32767 live in 256-slot pages, allocated on
first write and reached through a page table. Their cells and metadata follow
the same protocol.

`meta[p]` packs the cell's checkpoint epoch above its TeX level. The machine
keeps `cur_meta = epoch << 32 | cur_level`, so `meta[p] == cur_meta` answers
both questions an assignment asks with one compare:

- Is a group save needed? Only if the level differs.
- Is a checkpoint preimage needed? Only if the epoch differs.

A shadow `[u8; 256]` catcode table serves the source tokenizer. The
`\catcode` assignment path updates it, and restore rebuilds it. It is derived, never
authoritative.

## Token memory

Every stored token list lives in one `Vec<u32>`: macro definitions, token
registers and parameters, marks, write texts, and the format's lists. That
includes `\everypar` and `\output`.

| Offset   | Word                                                                                      |
| -------- | ----------------------------------------------------------------------------------------- |
| `L + 0`  | Reference count                                                                           |
| `L + 1`  | Checkpoint epoch of the last reference-count write                                        |
| `L + 2`  | Length in words after the header                                                          |
| `L + 3`  | Info: parameter count, `SIMPLE` (undelimited only), `HAS_MATCH`, `PROTECTED`, body offset |
| `L + 4`  | Definition origin                                                                         |
| `L + 5…` | Parameter text, end-of-match, body                                                        |

**References.** References are counted exactly where TeX counts them:

- eqtb cells and save records own their references;
- frames add one while they read the list;
- node payloads (`\mark`, `\write`, `\special`) own theirs.

Ownership moves with the word, so `eq_save` copies a word without touching its
count, as TeX §283 does.

**Allocation.**

- `scan_toks` builds a list at the tail of `mem`.
- `finish_list` then moves it into a freed block of the same size class, if one
  exists.
- Lists up to 128 words use exact-size free lists. Larger lists use
  power-of-two classes.
- Freeing a list at the tail truncates `mem`.

Expanding scans (`\edef`, `\xdef`, `\message`, `\write`) build in a scratch
vector, then copy once. That way nothing expansion does can interleave with a
list under construction.

**Format prefix.** Loading a format copies its token-memory image into the
prefix of `mem`. Lists below `format_end` are immortal: reference operations
skip them with one compare, so format pages are never written. Format macros
carry a large share of activations in LaTeX documents, and their counts would
otherwise be written twice per call.

**What changes.** This is the arena the earlier designs never finished. It
replaces:

- `DefinitionRoots`, `DefinitionBank`, `DefinitionEntry`, and
  `ResidentMacroBody`;
- `TokenListId` and its `Rc<[TokenWord]>`;
- the definition key in meaning words.

A macro meaning is the list offset itself. Activation reads the header through
one bounds check.

## Input

### Frames

A frame is 28 bytes, down from 120:

- `loc`, `end` and `start`;
- `params`, the invocation's `ps` index;
- `name`, the control sequence, kept for `show_context`;
- `origin` and `kind`.

The current frame is a field of the machine, as TeX's `cur_input` is. Reading
it needs no vector access or bounds check. A push copies it into `stack`; a pop
copies it back.

Frame kinds select one of two banks:

| Bank          | Frames                                                            | Origin                                         |
| ------------- | ----------------------------------------------------------------- | ---------------------------------------------- |
| Token memory  | Macro bodies, token registers, `\every…`, `\output`, mark texts   | The frame's origin: the invocation or the list |
| LIFO bank     | Macro parameters, backed-up tokens, inserted lists, `\the` output | Per-token origin in the high half of each word |
| Source buffer | Files and pseudo-files                                            | File identity and position, computed on demand |

### Delivery

The prototype's reader is TeX's §357 with the outer check folded into the
command-code test:

```rust
fn get_next(&mut self) -> (u32, u64) {
    loop {
        let loc = self.cur.loc;
        if loc < self.cur.end {
            self.cur.loc = loc + 1;
            let t = if self.cur.kind & LIFO == 0 {
                self.mem[loc as usize]
            } else {
                self.lifo[loc as usize] as u32
            };
            if t >= CS_FLAG {
                let w = self.eqtb[(t - CS_FLAG) as usize];
                if cmd(w) < OUTER_CALL {
                    return (t, w);
                }
                return self.special_cs(t, w);
            }
            let cat = (t >> CAT_SHIFT) as u8;
            match cat {
                LEFT_BRACE => self.align_state += 1,
                RIGHT_BRACE => self.align_state -= 1,
                OUT_PARAM => {
                    self.insert_param(t & CHR_MASK);
                    continue;
                }
                _ => {}
            }
            return (t, word(cat, t & CHR_MASK));
        }
        self.end_frame();
    }
}
```

The result is two registers: the token and its meaning word. A control
sequence costs:

- a cursor compare and a store;
- a bank select, a load, and a flag test;
- one bounds-checked eqtb load and a command compare.

Nothing else happens per token:

- no origin is written;
- no row is matched against a storage enum;
- no closure runs.

`last_origin()` recovers a token's origin lazily from the current frame. Only
argument capture, back-up, list building, and glyph construction call it.

### Back-up and the §390 pop

`back_input` follows TeX §325:

1. Pop exhausted frames.
2. Push the token and its origin as one LIFO word.
3. Push a one-word frame.

`macro_call` performs the §390 tail pop before pushing a body, so tail
recursion such as `\expandafter\iter\fi` keeps the stack flat. In the prototype,
popping a frame does three things:

- truncates the LIFO bank for backed-up and argument regions;
- releases the macro's list reference;
- copies the frame below into `cur`.

### Source frames

The prototype tokenizes its document ahead of time, so source reading is not
measured. The design keeps TeX's §343 state machine:

- One shared line buffer holds a byte segment per open source frame.
- Catcodes come from the shadow table.
- Control-sequence names resolve through an open-addressed table keyed by the
  name bytes. The string pool belongs to the format image.

A source frame keeps its token cursor exhausted (`loc == end`), so the fast
path falls through to the frame-end handler, which dispatches on `kind` to the
tokenizer. Token-list delivery never pays for source frames. A source token's
origin is the frame's file identity plus the token's
starting byte offset, which the tokenizer records in a machine field.

## Macro activation

### Parameterless macros

A macro whose header has no `HAS_MATCH` bit takes the fast path:

1. Read the length and info words.
2. If the body is empty, return.
3. Otherwise pop exhausted frames, push the current frame, and install the body
   frame.
4. Add one reference.

In the prototype this path costs 73 marginal instructions for `\mzero`,
including the read of `\mzero` itself. `\mbody`, whose body is two `\relax`
commands, costs 233 including their execution.

### Arguments in the LIFO bank

Arguments are the only tokens TeX copies, and the flat machine copies each one
exactly once, into its final place:

1. `scan_args` records the LIFO length as the invocation's region start in
   `ps`.
2. It appends each argument token with its origin.
3. It records a `(start, end)` pair per parameter.
4. `#n` in the body pushes a LIFO frame over that pair.
5. Popping the body frame truncates the LIFO bank to the region start and
   `ps` to the invocation.

| Matcher                | Used for                    | Behavior                                                                  |
| ---------------------- | --------------------------- | ------------------------------------------------------------------------- |
| Undelimited (`SIMPLE`) | Macros like `\def\a#1#2{…}` | Skip spaces, take one token or one balanced group, never store the braces |
| General                | Everything else             | TeX §392–§397 verbatim, with indices into the stored parameter text       |

The general matcher strips the outer braces of a single-group delimited
argument by moving its boundaries inward, with no copying. The `\par` test
compares against the fixed `\par` token, as TeX does. The `\outer` and
`\long` checks live in the cold `special_cs` path and the meaning byte.

This replaces `ArgumentArena` regions and their roots, retained source lanes,
holdback, `ArgumentRunAdmission`, and the per-parameter begin/publish protocol.

### Pinning and sliding

A frame below the macro can run out while its arguments are being scanned. A
backed-up token or an inserted list is the usual case. Popping that frame would
normally truncate the LIFO bank through the new arguments.

While `scan_args` runs, `lifo_pin` holds the region start. `end_frame` then
records the lowest released start in `dead_floor` instead of truncating. After
the §390 pop, `unpin` slides the new arguments down over the dead suffix and
adjusts their pairs. This path is cold.

Released regions always form a suffix below the pin, because frames pop in
LIFO order. The bank therefore never fragments.

### Parameter frames in place

`mtwo` (624 instructions) is the costliest macro case. Its time goes to pushing
and popping a frame for each one-token parameter.

A next step would deliver a one-token argument directly from the LIFO bank
without pushing a frame. This would be observably identical to TeX, as long as
two things are reconstructed lazily from the out-parameter word before `loc`:

- `show_context` must synthesize the `<argument>` line;
- `last_origin()` must return the argument's own origin.

The prototype does not do this yet.

## Expansion and main control

`get_x_token` is TeX's §380 loop, inlined. It returns tokens with
`cmd <= max_command`. Expandable tokens go to the out-of-line `expand_one`,
which:

1. charges fuel;
2. dispatches to `macro_call`, or to the §367 switch for `\expandafter`,
   conditionals, `\csname`, `\the`, `\number`, and the rest.

Conditionals push a small record (limit, condition code, and the line for
e-TeX diagnostics) onto `cond`. `pass_text` and `fi_or_else`
are TeX §494–§510.

Main control is TeX's big switch. One `match` on the mode-offset command forms
a dense jump table. Every iteration does four things:

1. Fetch with `get_x_token`.
2. Decrement fuel and the driver's operation budget, each with one
   compare-and-branch.
3. Test the attention word.
4. Dispatch.

Prefixed commands test the attention word once more for `\afterassignment` and
`\globaldefs`.

TeX's main control is not recursive. Boxes, alignments, output, and insertions
nest through semantic stacks, never through Rust stack frames. The top of the
big switch is therefore a complete resumption point.

**Yields.** The operation budget returns to the driver only at the top of the
big switch. All state is in the machine, so resuming is just calling
`main_control` again. No coroutine, saved closure, or replay of a partial
command is needed.

**Fuel.** Fuel bounds runaway expansion inside a single command. Its cold
refill handler:

- charges the session ledger;
- samples cancellation;
- halts once the job budget is exhausted.

Cancellation latency is bounded by one refill quantum, which the host
chooses.

## Assignments and groups

All assignments go through four functions, TeX's §277–§279: `eq_define`,
`eq_word_define`, `geq_define`, and `geq_word_define`. The word form's hot path:

```rust
fn eq_word_define(&mut self, p: usize, w: u64) {
    let old = self.eqtb[p];
    if old == w {
        return;
    }
    let m = self.meta[p];
    if m != self.cur_meta {
        if m as u32 != self.cur_level {
            self.save.push(Save { p: p as u32, level: m as u32, word: old });
        }
        if (m >> 32) as u32 != self.epoch {
            self.log_eqtb(p, m);
        }
        self.meta[p] = self.cur_meta;
    }
    self.eqtb[p] = w;
}
```

`eq_define` adds TeX's destroy rule. A displaced word whose command code owns a
list reference releases it. A list whose count reaches zero is freed, or
deferred if a checkpoint still needs it. Glue and box references follow the
same rule through their own tables.

e-TeX's reassignment shortcut (`old == w`) runs first. Tracing assignments
lives behind the attention word.

**Save records.** A save record is 16 bytes: index, old level, and old word. A
group boundary record stores:

- the previous boundary;
- the group code;
- the line, for e-TeX diagnostics.

`\aftergroup` tokens and sparse-register markers are tagged records on the same
stack.

**Leaving a group.** `unsave` is TeX §282:

- if the cell is now at level one, destroy the saved word;
- otherwise destroy the current word and reinstall the saved word and its
  level.

Restored cells keep the current epoch. Checkpoints are never admitted inside a
group, so every cell a group restores was already stamped when the group
wrote it. In production this check is a debug assertion.

**What changes.** This replaces `StateCell` and `StateWord`, the root
acquire/release on meaning writes, and the per-bank assignment plumbing.

## Checkpoints

Admission is unchanged from the aggregate contract. Capture happens only at
`JobStart` and eligible `OuterParagraphEnd`. At capture there are:

- no open groups, macro frames, scanners, or alignments;
- only outer vertical mode;
- possibly open conditionals and backed-up or stored non-macro input.

### What capture records

| State                         | Capture                                                                     |
| ----------------------------- | --------------------------------------------------------------------------- |
| eqtb and metadata             | Nothing. Start a new epoch; later first writes log preimages.               |
| List reference counts         | Nothing. Header epochs make the first count write per epoch log a preimage. |
| Token memory                  | The tail mark; freeze the free lists.                                       |
| Input stack, LIFO bank, `ps`  | Copies. They are short at an outer paragraph end.                           |
| Conditional stack and scalars | Copies.                                                                     |
| Save stack                    | Nothing; it is empty by admission.                                          |
| Fuel and high-water marks     | Nothing; they stay outside rollback, per the contract.                      |

### What the hot path pays

| Operation                       | Checkpoint cost                                            |
| ------------------------------- | ---------------------------------------------------------- |
| eqtb read, token read           | None                                                       |
| Repeated eqtb write in an epoch | The `meta == cur_meta` compare it already needs for levels |
| First eqtb write in an epoch    | One 24-byte undo record                                    |
| Reference count change          | Format check, then header-epoch compare; first write logs  |
| Free of a list below the mark   | One compare; the block goes to `deferred`                  |
| Allocation                      | None; only blocks above the mark are reused                |

### Restore and retirement

Restore:

1. Replay the undo log backward. That restores eqtb words, levels, epochs, and
   list counts.
2. Truncate token memory to the mark.
3. Discard the post-mark free lists and `deferred`.
4. Reinstall the copied stacks and scalars.

Its cost is proportional to the cells and lists touched since capture, plus the
small copies. There is no eqtb image and no token-memory copy.

When no live checkpoint needs a mark any more, retirement runs:

- deferred blocks go back on the free lists;
- frozen lists merge back.

### Candidates and resource misses

Accepted and candidate histories are consecutive epochs of one machine:

- Candidate admission is a capture.
- Rejection restores the candidate's capture. Restoring an older capture
  replays every later segment first.
- Acceptance retires the segments no live capture needs.

The undo log is segmented by epoch. Marks are monotone, so a deferred block is
freed once no live mark lies above it.

A resource miss halts in band (next section), and the driver restores and
replays. Partial state after a halt is never observed, so the halt needs no
unwinding discipline beyond halting effects.

### Verification in the prototype

The prototype's `--verify` mode runs every case through a full round trip:

1. Run the document to `\end`.
2. Fingerprint the meanings, expanding list references into their tokens.
3. Restore the checkpoint taken after the definitions.
4. Assert that the eqtb, metadata, and token memory are byte-identical to a
   deep copy taken at capture.
5. Re-execute from the restored input.
6. Assert an identical final fingerprint.

All 17 cases pass. The approach scales into a property test: random documents,
random capture points, and random restores.

## Provenance, tracing and observation

**Provenance.**

- Stored-list tokens take their frame's origin, which is the invocation or the
  definition.
- Argument, backed-up, and inserted tokens carry their own origin in the LIFO
  word. That is the same 8-byte token-plus-origin word as today's
  `TracedTokenWord`, kept only where per-token origins exist.
- Source origins are computed from the frame.

Nothing writes an origin per delivered token. Glyph construction calls
`last_origin()`.

**Attention word.** One `attention: u32` has a bit for each rare feature:

- a pending interrupt;
- positive `\tracingcommands`, `\tracingmacros`, `\tracingassigns`,
  `\tracinggroups`, `\tracingrestores`, or `\tracingifs`;
- a pending `\afterassignment`;
- nonzero `\globaldefs`;
- an attached observer;
- single-step debugging.

Assignments to those parameters recompute their bits. The parameters live in
one eqtb sub-range, so the test is one range compare in the parameter path.
Restore also recomputes the bits.

A `CommandObserver` is a cold callee behind the attention test, not a type
parameter. Unobserved runs execute no observer code and instantiate no observer
generics.

## Failure and halting

Hot functions return plain values. Recoverable TeX errors print and recover at
their reference site in cold code, exactly as TeX does.

Fatal errors, resource misses, exhausted fuel, cancellation, and capacity
overflow do two things:

- set the machine's halt reason;
- replace the input with a frame that yields the frozen halt token forever.

The halt token's command lies in the `cmd >= outer_call` range, so
`special_cs` sees it first. It behaves like TeX's end-of-file runaway recovery:

| Scanner status        | Halt token yields                      |
| --------------------- | -------------------------------------- |
| Defining or absorbing | A closing brace                        |
| Matching              | `\par`, to end the argument            |
| Aligning              | `\cr` and a closing brace              |
| Skipping              | The frozen `\fi`                       |
| Otherwise             | The halt command; main control returns |

Every scanner therefore terminates within a bounded number of steps without a
`Result` edge per token. Error printing and every external effect (writes,
shipouts, opens) check the halt reason first, so a halted machine publishes
nothing. Its state is discarded or restored from a checkpoint.

## Code size and build

- The hot core is one module with no generic parameters.
- Cold subsystems are `#[cold]` and `#[inline(never)]`.
- Diagnostics, tracing, interaction, and error recovery live in separate cold
  modules.
- The shipping CLI and WASM profiles use fat LTO, one codegen unit, and
  `panic = "abort"`.
- A size gate sums the text of the core symbols with `nm` and fails above
  64 KB.
- The prototype's core is about 21 KB. Of that, about 6 KB is expansion
  dispatch and 5 KB is main control.

## Budgets

The production budgets are the prototype's numbers plus about 25% for features
the prototype omits: alignments, e-TeX tracing parameters, the full primitive
switch, and source frames.

| Case         | pdfTeX | Budget |
| ------------ | -----: | -----: |
| base loop    |   3814 |   2100 |
| `relax`      |     91 |     45 |
| `mzero`      |    216 |     90 |
| `mbody`      |    398 |    290 |
| `mone`       |    421 |    250 |
| `mone_brace` |    709 |    370 |
| `mtwo`       |    975 |    780 |
| `mdelim`     |    781 |    620 |
| `let`        |    349 |    160 |
| `def`        |    866 |    600 |
| `group`      |    283 |    165 |
| `group_let`  |    708 |    450 |
| `advance`    |   1150 |    540 |
| `count`      |    876 |    500 |
| `toks`       |   1390 |   1030 |

For book 2606.24937, the budgets are:

- the token core (reader, expansion, activation, scanning, assignment) at or
  below pdfTeX's instructions for the same work;
- whole-book cycles at or below 1.5× pdfTeX after the cutover;
- whole-book cycles at or below 1.0× pdfTeX after the node follow-up.

## Beyond the token core

Box and page work is the next wall:

- `setbox` costs 7.9× pdfTeX's marginal instructions;
- checkpoint, fork, and history infrastructure is about 15–20% of the book
  profile.

The flat machine's three primitives apply to nodes unchanged:

1. A flat word arena with `u32` links.
2. First-touch undo keyed by epoch.
3. A tail mark with deferred frees.

One linked node arena holds TeX's variable-size nodes. A box register holds an
offset, and `\setbox` moves a reference instead of copying a region.

Nodes are mutated after construction, by packaging, line breaking, and the page
builder. So the undo for pre-mark nodes needs a per-node epoch word rather than
per-list headers. That follow-up gets its own design document.

Until then, the token core hands nodes to `tex-typeset` through a narrow
builder interface: append character, append glue, open and close a box. That
interface takes plain values.

## Migration

The replacement is one engine, built beside the old core and cut over once.
There is never a runtime fallback, a compatibility adapter, or two interpreters
in production. This relaxes the cutover rule in
[Minimal hot loop](minimal_hot_loop.md) only in one respect: the new core is
built and tested in-tree before it is wired in.

1. **Prototype (done).** `tools/flat-machine-prototype` measures the design
   and verifies checkpoint replay.
2. **Core crate.** `tex-machine` holds:
   - tokens, eqtb, token memory, frames, and the LIFO bank;
   - the source tokenizer and the format image;
   - the reader, expansion, all §367 expandables, and conditionals;
   - the scanners, assignments, and groups.

   The gate is identical command streams against the pinned pdfTeX oracle on
   the command-semantic corpus, through `tools/tex-command-stream`, under
   [Canonical Divergence Working Contract](canonical_divergence_workflow.md).
   The microbenchmark budgets are also checked.
3. **Main control and modes.** The big switch for all modes runs over the
   builder interface into `tex-typeset`. The gate is byte-identical DVI and PDF
   on the native suite.
4. **Checkpoints.** The machine implements the command, input, and dense
   environment rows of the aggregate inventory; other owners keep their
   contracts. The gate is the checkpoint, replay and edit-restart suites, plus
   the random capture-restore property test.
5. **Provenance, observation, tracing, and interaction.** These ship behind the
   attention word. The gate is identical logs, including tracing, on the
   canonical baseline.
6. **Cutover.** Wire `umber` and `umber-wasm` to `tex-machine`, and delete the
   old reader, processor, and state banks in the same series.
7. **Nodes.** The follow-up design.

## Risks and open questions

- **Feature surface.** The prototype implements about a dozen primitives. Some
  of the rest need care inside this model:
  - alignments: template frames and `align_state`;
  - `\noexpand` markers;
  - `\scantokens` pseudo-files;
  - `\detokenize`, `\unexpanded`, and `\showtokens`;
  - sparse registers;
  - pdfTeX's expandable string primitives.
- **Fragmentation.** Size classes may fragment token memory over long
  interactive sessions. Token lists contain no list references, so relocation
  only rewrites eqtb words, frames, and node payloads. A compaction pass at a
  capture boundary is straightforward if it is ever needed.
- **Memory lag.** Frozen and deferred blocks are reused only after retirement.
  The lag is bounded by the work between captures.
- **Epoch exhaustion.** `u32` epochs fail at the cold boundary before
  mutation, as the contract requires. Four billion captures per job is not a
  practical limit.
- **Unicode.** pdfTeX is 8-bit. The encoding leaves 21 bits for character
  codes, but Unicode active characters would need a hashed active region.
- **Error context.** `show_context` must reconstruct TeX's exact input-stack
  display from 28-byte frames and the parameter pairs. In-place one-token
  parameter delivery would add a synthesized line.

## What this deletes

| Today                                                                       | Flat machine                                 |
| --------------------------------------------------------------------------- | -------------------------------------------- |
| `TokenWord` kinds, frozen and parameter tokens                              | `u32` tokens in TeX order                    |
| `MeaningWord<G>` (16 bytes), definition keys                                | `u64` eqtb word with a direct list offset    |
| `MeaningBank`, `StateCell`, `StateWord`, per-bank assignment                | One eqtb, one `meta`, four define functions  |
| `DefinitionRoots`, `DefinitionBank`, `DefinitionEntry`, `ResidentMacroBody` | Token memory with header reference counts    |
| `TokenListId` with `Rc<[TokenWord]>`                                        | Token-memory offsets                         |
| `InputLevel` (120 bytes), `InputStorage`, `MacroBodyCursor`, `SourceSlot`   | 28-byte `Frame` with a bank bit              |
| `ArgumentArena`, `ArgumentRunAdmission`, retained source lanes              | LIFO bank and `ps`                           |
| `consume_resident_input` and `ExpandedReaderConsumer` closures              | `get_next` and `get_x_token`                 |
| `CommandProcessor` borrows across command, state, host and observer         | `Machine`                                    |
| `G` and `Facts` generics in hot code                                        | None; brands are checked at the API boundary |
| `Result` on every read                                                      | In-band halt token                           |

## Prototype

`tools/flat-machine-prototype` is a standalone crate, excluded from the
workspace. It defines one `Machine` with the layout above. Its modules cover:

- input and delivery;
- macro activation;
- scanners and conditionals;
- eqtb and token memory;
- main control;
- checkpoints.

It implements this subset:

- `\relax`, `\def`, `\let`, `\count`, `\advance`, and `\toks`;
- `\begingroup`, `\endgroup`, and simple braces;
- `\expandafter`, `\ifnum`, `\fi`, and `\end`;
- a `\checkpoint` command that captures after the definitions.

It tokenizes the microbenchmark document ahead of time and panics on errors.

```sh
cd tools/flat-machine-prototype
cargo build --profile fat
target/fat/flat-machine-prototype mone 1000 --verify
python3 measure.py --sudo --cpu 3 target/fat/flat-machine-prototype
```

`measure.py` prints marginal instructions with the interp-microbench method.
`scripts/interp-microbench.py --pdftex --umber LABEL=PATH` measures the
reference engines.
