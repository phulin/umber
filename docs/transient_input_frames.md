# Transient Input Frames

Design for bounding permanent token-list interning to durable definitions.
Tracked by the `umber2-ii83` epic.

## Problem

LaTeX preamble runs die on execution-proportional token-store interning. The
root cause is representational: `InputStack` token-list replay frames only
accept a `TokenListId` + `OriginListId` pair, so *every* transient token flow
must pass through the permanent hash-consed token store and the append-only
provenance arena before it can re-enter the input.

The per-event intern traffic today:

1. **Macro arguments** — `tex-expand/src/args.rs` freezes every argument of
   every call through `finish_traced_token_list`: an O(n) semantic hash (with
   an interner atom lookup per control sequence), an O(n) content compare even
   on a hash-cons hit, and a fresh origin list appended to the provenance
   arena every call (never deduplicated, never freed).
2. **`back_input`** — TeX's most common micro-operation (every value scanner
   reads one token too far and puts it back). When the frame-rewind and
   source-pending fast paths miss, it permanently interns a one-token list
   plus an origin list (`tex-expand/src/lib.rs`).
3. **`\expandafter` / `\noexpand` delivery** — `push_inserted_token` and
   `push_noexpand_token` intern a one-token list per use.
4. **Rendered expansions** — `\the`, `\number`, `\romannumeral`, `\string`,
   `\meaning`, `\jobname` intern their output per expansion
   (`tex-expand/src/values.rs::push_rendered_tokens`) plus a synthesized
   origin list per expansion.
5. Smaller repeat offenders: `insert_input`, alignment `end_template`
   interned per cell, and per-call re-parsing of macro parameter text.

Consequences: time scales with total expansion activity rather than distinct
content; the provenance origins arena grows with roughly every token that
ever flows through an argument or insertion; both stores are u32-capped and
panic at identity exhaustion. TeX82, by contrast, keeps all of these flows in
refcounted transient token memory and never touches permanent storage.

The design intent behind the store is sound *for durable state*: hash-consing
gives shared storage, O(1) checkpoint hashing via interned semantic ids, and
`\ifx` macro equality as raw id comparison (`macro_store.rs::semantic_eq`).
The mistake is that the checkpoint requirement — "frames must be
summarizable" — was strengthened into "frames must at every instant consist
only of durable store ids", paying the durable-identity cost per
micro-operation instead of per checkpoint.

## Invariant

Permanent token-store and provenance growth is proportional to **durable
definitions** (`\def`/`\edef` bodies, `\toks`, marks, `\output`, format
content), never to execution volume. Interning happens only when content
crosses into durable state, or lazily at a memoization/checkpoint boundary.

## Core change

Token-list replay frames gain a second payload kind that owns its tokens:

```rust
enum ReplayPayload {
    Stored    { token_list: TokenListId, origin_list: OriginListId },
    Transient { tokens: PooledBuf }, // Vec<TracedTokenWord> from a pool
}
```

- `TracedTokenWord` is a packed 8-byte `Copy` word carrying its origin, so
  transient frames need **no** `OriginListId` at all; provenance rides along
  inline.
- Buffers come from an `InputStack`-owned pool; popped frames return their
  buffer (including `StableFrames` mid-stack removals). Steady-state transient
  traffic performs zero allocation and zero hashing.
- `TokenListInputFrame` stops being `Copy`.

### Checkpoint summarization

`InputFrameSummary` gains a `TransientTokenList` variant carrying its
remaining words as `Arc<[TracedTokenWord]>` — the same by-value pattern
`SourceFrameSummary` already uses for its pending queue. Semantic hashing of
transient words must project control sequences through interner semantic
atoms (the `TokenSemanticIdBuilder` projection), not runtime `Symbol` keys.
This changes the input-summary hash shape and takes a version bump with a
migration note in `docs/core_state.md`.

### Macro arguments

A macro call captures all matched arguments into one pooled buffer plus nine
ranges, owned by the macro-body frame's `MacroArguments`. `Param(slot)`
replay copies the slot's words into a child transient frame's pooled buffer —
a memcpy of 8-byte words, negligible next to today's hash + compare + two
arena appends. Sharing instead of copying is a later optimization only if
profiles demand it.

### Provenance

Synthesized origins for rendered output become one `OriginId` per rendering
event, repeated inline in each word, replacing per-event origin-list
allocation. Origin-list allocation remains only for durable stores.

### Memoization interface (forward-looking)

Incremental memoization of substitution episodes (`umber2-vfqs.6`) interns
transient content lazily when an episode is recorded — once per distinct
episode, not per call. Transient frames deliberately have no durable identity
until that moment.

`tex_incr::TransientTokenEpisode` is the publication boundary for that future
recorder. It owns an immutable packed-token snapshot without touching either
permanent store. `publish` calls `finish_traced_token_list` only on its first
explicit recording request and returns the cached durable pair thereafter.
The type is deliberately not `Clone`: its cached handles belong to the state
timeline that accepted the episode. Expansion does not construct or publish
these records, so eager interning cannot leak back into the hot path.

## What does not change

Token store internals, semantic ids, watermark rollback, stored-list replay
(`\toks`, marks, macro **bodies**), condition frames, alignment machinery,
`\ifx` id-equality, and the hash-consing invariant for durable lists. The
`literal_span_cache` and replay markers key on `TokenListId`; their domain is
macro bodies, which stay stored.

## Phases

Tracked as children of `umber2-ii83`; each is separately committable.

1. **`umber2-ii83.1`** — transient payload, buffer pool, by-value summaries,
   aggregate validation, hash version bump.
2. **`umber2-ii83.2`** — convert `back_input`, `insert_input`,
   `\expandafter`/`\noexpand` delivery, and rendered-expansion output to
   transient frames; single synthesized origin per rendering event.
3. **`umber2-ii83.3`** — transient macro arguments; adds the zero-intern
   profiling-stats regression test.
4. **`umber2-ii83.4`** — pre-parse macro parameter text at definition time
   (independent).
5. **`umber2-ii83.5`** — lazy interning hook for the memo layer.

## Risks

- **Summary/validation churn**: `Universe` aggregate validation and
  `InputFrameSummary` equality/hash need the new variant; fiddly but
  precedented by source-frame pending tokens.
- **Frame lifecycle**: pool returns on every removal path, including
  mid-stack `StableFrames` removals.
- **Origin-list assumptions**: diagnostics and `MacroReplaySite` consumers
  that assume Inserted/NoExpand replay carries an origin list need auditing.

## Validation

- Existing parity fixtures (story, gentle, TRIP/e-TRIP).
- `benchmarks/tex-state` `state_budgets` and `benchmarks/plain-tex` show no
  regression; the LaTeX preamble workload improves.
- New profiling-stats regression test: a macro-heavy loop performs zero
  token-store interns and zero origin-list allocations after warmup, pinning
  the invariant against silent regression.
