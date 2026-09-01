# `umber2-66p0.8.40.128`: dispatch the resident row directly

## Selection authority

The integrated `.127` authenticated 20,000,000-command capture is the sole
broad selection authority. It ran commit `bdb8ba4e8` at exact work vector
`(20000000, 19907047, 2216876, 6018541, 16781945, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Raw delivery comprised 463,197 source words, 11,520,843
stored/body words, 7,922,916 macro-argument words, and 91 synthetic end-v
commands. `advance_resident_command_into` led application self time at 12.14%.
No new broad profile or corpus execution was run.

The resident front first called `select_resident_top`, which discriminated the
authoritative `InputLevel` to capture first-touch state, encoded its mutable
borrow as a widest-variant `ResidentInputTop`, and returned that carrier to a
second match. Successful branch settlement then returned a broad `Result`
which the same function decoded only to return immediately. Neither carrier is
part of TeX82 §§24--25 semantics.

## Architectural simplification

`advance_resident_command_into` now reads the top index once and matches the
authoritative `InputLevel` row directly. Each concrete source, replay,
attempt, durable, macro-body, or macro-argument arm records its own exact
first-touch inverse, advances its sole cursor, and writes the caller's reusable
`CurrentCommand` destination. Successful delivery returns directly from that
arm; only actual stored-token exhaustion reaches the common resident
retirement continuation.

The universal `ResidentInputTop` and `select_resident_top` no longer exist.
The architecture test rejects their return, a separate inline-state
discrimination, and a result encode/decode transition around the direct row
match. The concrete source owner remains out of line because source
tokenization needs its checked slot and source-specific inverse fields; it is
not present in stored or macro branch frames.

This changes no cursor or storage owner established by `.125` and `.126`.
Replay still advances its one logical/physical resident coordinate, macro body
still advances its store-owned cursor, and macro argument still advances its
absolute scratch coordinate and provenance run. Source context, provenance,
suspension, rollback/redo, retirement, replay completion, acceptance, and
diagnostics retain their existing owners and order.

## Focused before/after gate

The exact baseline is `.127`'s accepted production-profiling
`mixed_macro_resident_pipeline` binary. The final binary ran the same row once
under `perf stat` and the checked public-copy interposer. Both report 2,000,000
macro-body transitions, 1,000,000 parameter deliveries, 1,000,004 replay
words, 2,000,004 raw frame steps, 1,000,000 expanded deliveries, 1,000,001
macro expansions, zero suspension moves, zero command copies, and zero warmed
allocations or requested bytes.

| Counter                              |      Baseline |         Final |                Delta |
| ------------------------------------ | ------------: | ------------: | -------------------: |
| User instructions                    | 2,452,753,956 | 2,434,755,137 | -17,998,819 (-0.73%) |
| User cycles                          |   980,735,053 |   975,947,863 |  -4,787,190 (-0.49%) |
| Resident function bytes              |        11,619 |        11,597 |         -22 (-0.19%) |
| Resident stack-frame bytes           |           376 |           328 |        -48 (-12.77%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                0 / 0 |
| Public `memcpy` calls / bytes        | 130 / 344,169 | 130 / 344,169 |                0 / 0 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                0 / 0 |

The instruction reduction is about nine instructions per raw delivery and is
the primary CPU result. Cycles moved in the same direction under concurrent
host load. Internal elapsed time was noisy and is not used as evidence. Exact
public-copy attribution reconciles both APIs with zero collision overflow or
probe-internal calls; no copy or allocation was introduced.

## Evidence

Ignored evidence is under `target/umber2-66p0.8.40.128/focused-gate/`.
Baseline and final binary SHA-256 values are
`c26b5b3a173807a83979c834fa59fc869431756d2c276a539dd11486bd7c463b` and
`13787725294efd58d0cdd2ff6ae57ba8eb1be9b98d3af91148baf8a072ff2f04`.
Their counter receipts are
`1ce870bf8a60b5c3bb7284d38efe0dd62f256c660ff3220bcde94f607f73c20e` and
`03fd99ed3b17957f563a08befaacbf8dde4b3b467b4c814dfb37dfcd893bde59`;
their symbolized copy reports are
`8bbeafa3fb07109bb262afa48c08e3b3cac139e2969b9195accd1ec9da1cb8cf` and
`2a69c2ea6825d7d648740e32c228cc3d409c0d94031f5010def089ccb08b3630`.

## Validation

The complete `cargo test -q --tests` routine suite passes, including 384
`tex-command` unit tests and 23 command architecture tests. The focused
production benchmark passes its exact semantic, allocation, command-copy,
suspension, and public-copy invariants.
