# umber2-vgjr.5.3 — canonical artifact-derived DVI

Issue: `umber2-vgjr.5.3`

Implementation tree: `dcb9fbda7f920cc7b8909f5f10e29f78e7a134d5`

TeX82 §§638--642 define shipout and list-output behavior. Umber now lowers
live page state once into canonical artifact bytes, then derives fresh and
memo-hit DVI through the same `DviPagePlan::compile_v10` authority and the
same executor error conversion. DVI-disabled fresh and memo publications both
retain artifacts while omitting page plans.

The retired predecessor was the paired live artifact/DVI emission path in
`tex-exec`, including DVI parameters threaded through every node emitter,
streamed-plan selection, and the complete 336-line live-node leader
materializer. No fallback or shadow comparison path remains. Positioned PDF
effects still lower the committed artifact for saved-position and snapping
state, preserving their existing state publication boundary.

Production Rust changes by +34/-538, a 504-line net reduction. Active Rust
proof changes by +79/-1; guidance changes by +2/-2. Before this writeback and
the combined-plan update, the complete tracked issue changes by +115/-541, a
426-line net reduction. No fixture or serialized artifact changed.

Verification on the implementation tree:

- `tex-exec`: 20 passed under `MemoryMax=512M`; `tex-out`: 145 passed under the
  same cap.
- Exhaustive `tex-command-stream`: `VERDICT: CLEAN`, zero semantic and zero
  advisory geometry divergences.
- Byte-exact Story and canonical Gentle DVI corpus gates passed.
- The 1,024-node shipout stop gate improved ordinary throughput about 12% and
  deferred-math throughput about 42% against its stored predecessor baseline;
  maximum RSS was 49,836 KiB under `MemoryMax=512M`.
- `cargo test -q --tests`: complete native suite passed under `MemoryMax=1G`,
  maximum RSS 315,388 KiB.

## Maximum-depth canonical replay repair

Program closeout found that the canonical-byte adapter retained recursive
semantic validation, box replay, and boxed-leader reconstruction even though
the owned geometry walker was iterative. A valid artifact at the 4,096-level
codec limit therefore exhausted a 512 MiB scope before DVI compilation could
finish.

Canonical list slices now create owned byte cursors. Explicit reader frames
validate every ordinary and DVI-ignored descendant in depth-first wire order;
explicit replay continuations balance box entry and exit; and explicit
materialization continuations reconstruct the localized leader payload. The
temporary leader tree is also dismantled iteratively after replay. The codec's
node/depth ceilings, first-error order, font/effect checks, artifact bytes, DVI
bytes, and `DviPagePlan` ownership boundary are unchanged.

Active regressions compile both an ordinary canonical artifact and a boxed
leader whose deepest node is exactly level 4,096. Together they pass in 4.43 s
at 43,992 KiB maximum RSS under `MemoryMax=512M`; the complete 148-test
`tex-out` suite passes under the same cap at 265,448 KiB maximum RSS. A
separate malformed-artifact case proves depth-first missing-font error order
through a discretionary list. TeX82 §§638--642 remain the semantic authority;
this repair changes only the bounded implementation of the artifact adapter.

Final repair verification also passed all 466 active `tex-exec` tests at
362,488 KiB RSS and all 40 active `tex-incr` tests at 166,328 KiB RSS under
`MemoryMax=512M`. The exhaustive tracer remained `CLEAN`; Story and canonical
Gentle remained byte-exact. The shipout stop gate measured 1.129 ms ordinary
and 6.842 ms deferred-math midpoint estimates with 230,652 KiB maximum RSS.
The complete native suite passed under `MemoryMax=1G` at 312,152 KiB maximum
RSS, and uncapped `CARGO_BUILD_JOBS=6 scripts/check.sh` passed all four gates.

## Nested-leader geometry repair

A second closeout challenge found that a leader payload containing another
leader still re-entered the DVI and positioned list walkers recursively. Both
backends now represent repetition and exact post-box coordinate restoration as
continuation frames on their existing explicit work stacks. DVI frames retain
only the box scalars used during traversal instead of cloning the remaining
owned subtree. Leader start, inclusive edge tests, rounding compensation,
glue accumulation, synchronization, event and byte order, effects, and
provenance remain the TeX82 §§638--642 behavior.

Active regressions use the unchanged maximum-depth contract: one root hlist,
4,093 nested hlist leader payloads sized to emit exactly once, and a terminal
rule. Canonical-byte DVI replay passed in 2.96 s at 43,988 KiB RSS and
positioned lowering passed in 0.17 s at 43,928 KiB RSS under
`MemoryMax=512M`. Companion cases prove depth-first box/event exit order,
owned-versus-canonical DVI equality, and first malformed-font error order
inside a leader before a later sibling.

Final bounded verification passed all 151 `tex-out` tests at 87,180 KiB, all
466 `tex-exec` tests at 315,916 KiB, and all 40 active `tex-incr` tests at
161,888 KiB. Fresh/memo DVI and DVI-disabled parity passed. Story and the
explicit canonical Gentle gate remained byte-exact at 75,088 KiB and
262,984 KiB. The exhaustive tracer was `CLEAN` with zero semantic and zero
advisory geometry divergences at 424,960 KiB. The repeated shipout benchmark
reported 1.1324 ms ordinary and 6.8957 ms deferred-math midpoint estimates at
248,028 KiB. The complete native suite passed under `MemoryMax=1G` in 29.72 s
at 309,800 KiB, and uncapped `CARGO_BUILD_JOBS=6 scripts/check.sh` passed all
four gates.

## Final program closeout audit

The final audit at `560be7d695b28debc807d7ac63b6ef32a12104c4`
confirmed one artifact grammar and one page-plan authority. Owned emission,
owned decode, canonical scan and validation, canonical DVI replay, localized
leader reconstruction and retirement, node ordinals, DVI traversal, and
positioned traversal use bounded explicit work stacks. Nested leaders do not
re-enter either backend list walker. The executor contains one
`DviPagePlan::compile_v10` call, shared by fresh and memo publication, with one
error conversion and one DVI-disabled omission policy. The deleted executor
builder, streamed-plan branch, coordinate oracle, and live materializer remain
absent.

The cumulative program changes are 2,225 authored additions and 2,182
deletions, a 43-line net increase. Production Rust accounts for 1,505
additions and 2,082 deletions, a 577-line net reduction; active proof, guidance,
and durable documentation account for the remaining 720 additions and 100
deletions. This measured result replaces the original 1,450--1,900-line
production forecast.

Fresh verification compiled the full native suite and standalone shipout
benchmark uncapped with `CARGO_BUILD_JOBS=6`. Under `MemoryMax=512M`, all 151
`tex-out`, 466 `tex-exec`, and 40 active `tex-incr` tests passed. The
maximum-depth canonical-box and nested-leader matrix passed at 45,136 KiB RSS,
including depth-first order and malformed-font precedence; fresh/memo DVI and
DVI-disabled parity passed. Story and selected canonical Gentle remained
byte-exact. The exhaustive tracer reported `CLEAN`, zero semantic divergences,
and zero advisory geometry differences at 424,708 KiB. Repeated shipout
midpoints were 1.0987 ms ordinary and 6.7585 ms deferred math at 249,736 KiB.
The full native suite passed under `MemoryMax=1G` in 27.91 seconds at 308,156
KiB. The final uncapped `scripts/check.sh` passed all four gates.
