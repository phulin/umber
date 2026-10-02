# Native command ownership

The native interpreter fetches, scans and applies a command, then goes directly
to its next fetch label. There is no successful-command settlement or cleanup
stage. This supersedes the boundary-clear scratch and repeated demand-checking
policy described by earlier hot-loop documents. Beads `umber2-i0n5.16` tracks
the replacement and its semantic and performance evidence.

## Scratch ends at its consumer

A scratch collector and its sealed list occupy the same arena row. The sealed
coordinate is a move-only capability; reading borrows it, and publication,
insertion, rendering or splicing consumes it. Consumption returns the actual
chunks and retires the row. Consumed holes below another live collector stay
vacant until the live suffix retires; no separate free-row table is necessary.
Monotone row serials reject stale coordinates after suffix reuse.

A scanner that fails discards only its own unfinished suffix. Child-scope
marks use a serial frontier, not a vector length that consumption can reduce.
Thus consuming a child or an older finished list does not invalidate an active
scanner's scope. Successful consumers release their lists without visiting
unrelated scanner owners. A halted job can retain failing owners until job
destruction or full checkpoint replacement; that is an actual coarse lifetime,
not a successful-command cleanup pass.

## Save statistics belong to stack changes

The native save journal already maintains current word usage and the newest
checked push. It also records the maximum checked usage at actual save pushes.
It retains the classic and e-TeX source-line projections as scalar maxima,
because those profiles can assign different maxima to the same stack history.
Native box entry and exit update the same owner's box-spec contribution; an
executor-side duplicate word counter is unnecessary.

These job maxima never rewind on unsave, checkpoint restore or candidate
rejection. Host statistics read them at their own reporting boundary. A token
read, a macro invocation or an unrelated terminal command does no save-stack
accounting and does not traverse stack owners.

## Trace state belongs to tracing

The processor's observation loan and persistent scanner/alignment regimes have
their own begin/end transitions. Tracing demand is read where a trace or a bulk
read needs it. It is not recomputed by a post-command reset. The actual main
trace entry initializes its prefix and printed flag. The count advances only
at a real trace and is monotone throughout its processor loan; an untraced command
does not reset absent trace state.

## Semantic events own their continuation

An assignment inserts its pending afterassignment word at its actual
assignment completion. Unsave inserts aftergroup words while saved owners are
live. Input publication consumes its actual token owner. These actions precede
the next fetch and do not pass through a generic operation-commit function.

Reports retain their TeX chronology: scanner reports publish before the
semantic action that consumes the operand; semantic reports publish at the
reporting action. A synchronous error marks its processor loan at the error
producer, so its actual reporter captures the cause before another command.
Ordinary commands do not poll the World error counter. Output construction, shipout, page-region succession and
mode changes retain their real ownership cuts. Observation and dependency
tracking publish their requested evidence at their actual instrumentation
boundary. None authorizes an ordinary post-command sweep over page, World,
input, scratch, trace and save-stack state.

The bounded host driver still counts native commands and returns for its
budget, cancellation, terminal, resource, named-checkpoint or demanded-evidence
boundary. Those host units are not command transactions. Full checkpoint
admission and restore retain the inventory and root-before-payload rules in
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md).

## Scalar scanners use ordinary calls

Integer, dimension, glue and expression scans share one synchronous scalar
grammar. A dimension keeps its coefficient and bounded decimal digits in its
actual call, reads its unit, converts it and returns. It needs no progress enum,
numeric stage cursor, resumable product or duplicate unit implementation.
Ordinary dimensions, glue coefficients and expression shortcuts use that same
decimal and unit grammar. Resource needs unwind those local values before
full-checkpoint replay.

This consolidation preserves TeX82 §§443–460's read and backup chronology:
fractional terminators absorb one spacer; failed keyword prefixes and their
offending token retain separate input levels; internal units skip the trailing
optional-space scan; font-relative units consume their own optional space;
and magnification errors report before the physical unit is read. Removing
stages does not replace those semantic transitions with speculative lookahead.

Keywords borrow their existing ASCII spelling. Their only local collection is
the bounded matched prefix that a failed match must replay. They do not copy
the keyword into a second inline string or attach a generation brand to
temporary spellings. Character tests project character and category directly
from the resolved native word; character aliases still use their resolved
meaning, and control sequences remain ineligible as keyword letters.

Input admission accepts its actual source owner: one traced word, an immutable
stored list, or a move-only scratch list. A one-word backup does not construct
a general source enum, carry a duplicate length, or dispatch the other source
families. Static source admission writes the same final input row and word
suffix. Genuine transient iterators and keyword-prefix lists retain only their
actual replay iterator and ownership policy. Source context, level identity,
stack maxima, brace reversal and retirement remain the input stack's authority.

## Verification

Owner tests cover consumption, nested scopes, stale coordinates, failed
collectors and borrowed immutable publication. Alignment preamble collectors retain their actual live owners across host
steps and release them when the preamble finishes. Native save-stack tests cover
mixed group/box words, both source-line profiles and nonrefundable maxima.
Trace, afterassignment, aftergroup, errors, output and observation retain their
semantic tests and ordered canonical comparison. Matched release profiles and
microbenchmarks measure the deleted command-boundary work separately from
the older baseline's already completed refactors.

## Measured result

The implementation is `8b52e20c9207e8c09de850aca0bf5c6e48b6c884` (the main
ownership change is `0c29b01f2`). The frozen release executable is
`target/umber2-i0n5-16/no-settlement/umber`, SHA-256
`0f0484ce0755b10a0eb7da30c45697516a111c636c582022ce8748e3c4e29a5a`.
It uses the same release recipe and frozen format as the earlier comparison.
`micro.json` and `book-receipt.json` in that directory retain the exact engine,
format, input, argument, environment, output and counter evidence.

The 22-case microbenchmark measures short/long instruction differences with
startup removed. Its marginal costs against the immediately preceding
`d44cff87a` build include:

| Fixture                   | Previous instructions | Current instructions | Change |
| ------------------------- | --------------------: | -------------------: | -----: |
| Relax                     |                 523.5 |                389.8 | −25.5% |
| Macro with literal body   |               1,586.3 |              1,318.9 | −16.9% |
| Ordinary group pair       |               1,841.5 |              1,658.8 |  −9.9% |
| Let assignment            |                 1,448 |                1,325 |  −8.5% |
| Definition                |               3,460.9 |              3,339.2 |  −3.5% |
| Token-register assignment |               7,240.2 |              7,134.1 |  −1.5% |
| Dimension fixture         |              22,773.8 |             23,062.1 |  +1.3% |

The empty macro remains at 216 instructions, equal to the pinned pdfTeX
fixture. The one-parameter fixture remains at 1,081 (pdfTeX: 421). Thus deleting
command settlement does not itself remove the remaining reading, matching and
replay costs. There is no numerical acceptance threshold for this design.

A fresh unobserved, CPU-pinned book pair uses the partially refactored Claude
control `eb754fc2`, the same 74 sources and frozen format, detached working
and cache directories, and the existing wall/RSS/fuel bounds. Both engines
finish the unrepaired 588-page document with all four PDF/aux/toc/out hashes
unchanged. The six PMU events report 100% enabled time; the conclusions below
use instructions and cycles.

| Metric            |  Claude control |   Current build |
| ----------------- | --------------: | --------------: |
| User instructions | 510,737,414,107 | 469,724,518,571 |
| User cycles       | 340,429,560,066 | 293,723,025,634 |
| User seconds      |          107.25 |           92.68 |
| Wall seconds      |          113.63 |           98.13 |
| Maximum RSS, KiB  |         852,016 |         831,100 |

This is 8.0% fewer instructions and 13.7% fewer cycles than that Claude
control. Relative to the earlier `d44cff87a` book receipt, the additional
instruction reduction is 1.4% and the cycle reduction is 4.4%. The modest
whole-document instruction gain and larger noop gain distinguish command
bookkeeping from the reader/matcher work left in the workload.

Verification passes 4,237 routine native tests (980 ignored; 85 groups), all
four authoritative `scripts/check.sh` gates, and the focused Story and
immediate-write trace regressions. The exhaustive canonical comparison is
byte-identical to the existing 18,573-byte worklist: 663 known ordered
semantic divergences at 17 root sites and zero geometry differences. It is
still a DIVERGED verdict, not a clean oracle comparison. Optional profiling
command/execution tests pass 1,319 tests with three ignored; the three selected
profiling tool checks and two selected wasm checks pass, including Firefox.
