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

The ranked expansion classifier covers macro expansion and the measured
`ExpandAfter`, conditional, `CsName`, and converted-token primitive families.
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

The final feature-enabled profiling binary was built from `cb7524152` and has
SHA-256
`c98d7174c3cab3dc386483d7ebf8cd9db13b209e21c814f12d16799e5fa4821c`.
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
7.75 seconds and 14.52 seconds with 322,984 KiB and 453,240 KiB peak RSS. These
wall/RSS values are diagnostic; the exact work and structural counters are the
acceptance authority.

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

The ignored e-TRIP run retains an older independent observer-oracle mismatch:
expected 297,156 command events and actual 297,158, first diverging at event
57,934. Its terminal transcript and normalized DVI are byte-exact at SHA-256
`700e8ed48c34c84b8c40e0730a3b9864186f32c7c187b29b7c6866d3adc6d67d`.
This mismatch predates the physical-owner repair and is not hidden by changing
the comparator or expected fixture.
