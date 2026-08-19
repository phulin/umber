# umber2-awgc.5.3.5: Fused expansion command delivery

## Outcome

Ranked expansion no longer constructs an `ExpansionCommand` before it knows
that a real immutable-resource barrier exists. The persistent command
interpreter borrows the live `CurrentCommand`, performs macro or primitive
expansion in place, and moves the command into a typed continuation only when
expansion actually suspends. Rare unranked expansion stays in the same
interpreter and takes the same continuation path rather than entering a second
executor.

The executor-side preflight boundary now keeps ordinary delivery, expansion,
command tracing, ranked assignment scanning, and direct hot apply inside one
`CommandProcessor` borrow. A typed hot operation crosses the mutable-state
aliasing boundary only after the processor borrow ends. Transaction admission
still precedes scanning/apply, and resource, diagnostic, fuel, provenance,
observation, and retry ordering remain canonical.

This reduces persistent-interpreter operation entries by about 59% and leaves
`ExpansionCommand` materialization only for an exact finite cold barrier list.
It also fixes two physical-owner defects exposed by the stronger loaded-format
and allocation gates: reused packed macro coordinates are validated against
their complete current meaning without entering the weak index, and uniformly
packed token payloads retain an explicit TeX82 cell-ownership class instead of
conflating stored replay with new allocation.

## Fused boundary

The ranked expansion classifier covers macro expansion plus exactly
`ExpandAfter`, `Fi`, `IfX`, `IfNum`, `If`, `CsName`, `NoExpand`, `Detokenize`,
`String`, `IfFalse`, `RomanNumeral`, `Else`, `Expanded`, `IfCsName`, `Number`,
and `The`.
The live command remains borrowed for the successful hot path. Suspension
captures it once in the existing continuation, including exact fuel,
provenance, observations, diagnostic queues, and nested expansion state.
Resuming a host-dependent expandable primitive suppresses only the already
emitted TeX82 section 367 trace; it does not suppress fuel or expansion work.

Ranked definition, `let`, catcode, and group operations use the sibling fused
scan/apply boundary. The structural control
`ranked_assignments_use_one_processor_borrow_each` proves that delivery,
expansion, tracing, and scanning take one interpreter borrow per operation.
Cold scanned operations and true semantic barriers retain the ordinary typed
path.

## Exact fixed-clock measurement

The final feature-enabled profiling binary was built from `cefd56ab2` and has
SHA-256
`c9de6aee3b03fcef7197b023cdfc1dcda90aeb2ce11784e0851de09eb8aaf988`.
Both measurements used `SOURCE_DATE_EPOCH=1787080434`, the restored schema-11
pdfLaTeX format, schema-3 distribution, exact 105-key offline closure, and
`ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`.

| Counter                             | Frozen 6M |  Final 6M | Frozen 12M |  Final 12M |
| ----------------------------------- | --------: | --------: | ---------: | ---------: |
| Fuel charges                        | 6,000,000 | 6,000,000 | 12,000,000 | 12,000,000 |
| Token-frame steps                   | 5,999,815 | 5,999,815 | 11,999,815 | 11,999,815 |
| Expanded deliveries                 |   507,410 |   507,410 |  1,177,349 |  1,177,349 |
| Meaning lookups                     | 1,718,333 | 1,718,333 |  3,506,292 |  3,506,292 |
| Scanner-status tokens               | 5,352,087 | 5,352,087 | 10,599,869 | 10,599,869 |
| Write expansions                    |       588 |       588 |      1,182 |      1,182 |
| Interpreter operation entries       |   190,107 |    78,140 |    392,501 |    158,474 |
| `ExpansionCommand` materializations |   445,714 |    10,497 |    930,475 |     23,055 |

Operation entries fall 58.9% at 6M and 59.6% at 12M. Expansion DTOs fall
97.65% and 97.52%; the remaining 10,497 and 23,055 are the finite cold
suspension/fallback list, not hot ranked expansion. The final guarded runs took
7.22 seconds and 14.13 seconds with 323,448 KiB and 453,156 KiB peak RSS. These
wall/RSS values are diagnostic; the exact work and structural counters are the
acceptance authority.

Every remaining materialization is accounted for by the complement of the
ranked classifier. The measured finite cold list is:

| Cold expandable opcode |   6M count |  12M count |
| ---------------------- | ---------: | ---------: |
| `Meaning`              |        579 |        869 |
| `Input`                |         65 |        121 |
| `EndInput`             |         30 |         57 |
| `JobName`              |          1 |          1 |
| `IfTrue`               |        360 |        731 |
| `IfCat`                |      2,133 |      4,395 |
| `IfDim`                |        112 |        117 |
| `IfOdd`                |      2,819 |      7,063 |
| `IfCase`               |      1,341 |      3,175 |
| `IfVMode`              |         25 |         28 |
| `IfHMode`              |         91 |        501 |
| `IfMMode`              |          1 |          1 |
| `IfVoid`               |          1 |          1 |
| `IfVBox`               |          1 |          1 |
| `IfEof`                |        187 |        187 |
| `Or`                   |      1,291 |      3,118 |
| `Unexpanded`           |        278 |        515 |
| `Unless`               |          2 |         39 |
| `Scantokens`           |          0 |          6 |
| `IfDefined`            |        206 |        315 |
| `FileSize`             |        115 |        224 |
| `StringCompare`        |        857 |      1,588 |
| `ShellEscape`          |          1 |          1 |
| `CreationDate`         |          1 |          1 |
| **Exact sum**          | **10,497** | **23,055** |

Both runs report zero `CommandState` and step-snapshot clones. The warmed
packed cutover gate reports zero allocations, requested bytes, `Arc`/`Weak`
retains, weak upgrades, weak-index work, and content hashes for ordinary source
delivery, backup/replay, stored replay, and macro matching/expansion. The
warmed HotCore mark gate completes 10,000 cycles with zero allocations and a
192-byte snapshot.

## Loaded-format owner repairs

Loaded TRIP first exposed a stale admitted packed macro owner after rollback
reused a stored definition coordinate. Validation now compares flags and both
token roots against the current packed meaning. The current meaning is read
from the already rooted packed record, so a warmed macro cache hit does not
upgrade or search the weak value store. Packed macro replacement also splits a
same-record token-root coordinate before mutating one of two formerly shared
parameter/replacement roots.

The final TRIP memory mismatch had a separate physical cause. Every token
payload had already moved to one packed host representation, but the payload
retained only a boolean backed-up marker. Consequently
`transient_dynamic_words` treated an immutable stored replay as newly
allocated TeX82 one-word cells. At canonical box 254 the measured composition
was incorrectly `457 + 61 + 77`; the proven physical owner composition is
`457 + 1 + 77 = 535`. Earlier copies similarly charged 205, 307, and 409 stored
cells, raising the high-memory extent from 535 to 726 and the final report from
3,556 to 3,747 words.

`PackedTokenChunk` now distinguishes `Stored`, `Transient`, and `BackedUp`
physical ownership independently of its packed bytes. A nonempty stored replay
control proves that replay adds no dynamic cells merely because its host form
is packed. Canonical ignored TRIP now passes completely, including the exact
`3556/250000` memory row, 22 command events, 432 geometry events, transcript,
log, and normalized DVI SHA-256
`6420f3461dec8e5feed4b03bfc3717d00c8a36fae4fe9226f6d53a4db7592bb9`.
No expected value or comparator was changed.

## Validation

The final tree passes the complete `cargo test -q --tests` routine suite and
`scripts/check.sh` reports all four gates passed. The optimized exhaustive
`tex-command-stream` diagnostic reports `CLEAN`: every registered fixture was
compared to exhaustion with zero gating semantic divergence and zero advisory
geometry divergence. Focused controls cover processor-entry fusion, packed
definition-slot reuse, exact-meaning owner validation, resource-retry trace
continuity, Batch diagnostic selection, and stored/transient physical token
ownership.

The blocker diagnosis also restored exact e-TRIP without changing an oracle or
comparison policy. TeX82 sections 366, 370, and 380 require `undefined_cs` to
be reported and discarded inside `x_token`; preflight had instead preserved
four expanded deliveries for a second stomach-owned recovery path. The sole
command projection is now exact at 297,154 records after the oracle's two
typed `protected_delivery_suppression` records are excluded as designed.
Geometry is exact at SHA-256
`7bad29074d9f62af2895c1673356dae79329113a6aadacd2af59f0a2ba75e2a3`,
the transcript is exact at
`dc7ab0835d868d7f1c79cbf8f4ce6f23c052b6be7348e36a7a5a2c8fb1247ad7`,
and normalized DVI is exact at
`700e8ed48c34c84b8c40e0730a3b9864186f32c7c187b29b7c6866d3adc6d67d`.
Allocator-memory rows remain advisory under the existing conformance contract.

Final TRIP exposed one related trace-state seam after command/geometry/DVI had
already converged: section 367 consumed the mode prefix while tracing an
undefined command, but the diagnostic barrier failed to advance the
executor's `shown_mode` before a fresh facade traced the following command.
Commit `cefd56ab2` moves that state transition before the reporting barrier;
the focused positive control, exact TRIP, and exact e-TRIP all pass.
