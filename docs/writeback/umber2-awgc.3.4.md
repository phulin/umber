# umber2-awgc.3.4: Packed Token/Macro Cutover Audit

## Outcome

The packed ownership cutover is structurally complete, and its former
work-vector blocker is resolved by the versioned direct-prefix contract in
[`umber2-awgc.12`](umber2-awgc.12.md). Production token levels now have only three payload forms: a packed
token chunk, an admitted macro replacement, or an admitted argument range.
The former stored, transient, inline-transient, backed-up, inline-backed-up,
and shared-buffer owners have been deleted from production. Source-adjacent
constructors build packed chunks directly, and detachment remains a portable
projection rather than a second runtime owner.

The four-row warmed structural gate passes exactly, and the exhaustive
canonical command tracer is clean. The immutable arXiv prefix retains exact
fuel and raw-frame position. Its secondary counter changes are now attributed
to aggregate replay eliminated by the already accepted direct-operation
cutover, rather than misclassified as packed-token work.

## Ownership and lifetime repairs

Live macro meaning resolution now reads the generation-safe identifier from
the strong environment root already held by the executing meaning. This
removes the redundant weak upgrade without weakening the cold/stale lookup
APIs. Active macro diagnostic context reads parameter and replacement text
from the admitted packed macro owner, rather than consulting a definition-store
entry that may already have retired. The regression test retires that original
entry before rendering the active context.

The four weak upgrades observed during the first audit came from stored macro
meaning resolution and redundant macro observation, liveness, and provenance
lookups. Moving live reads to the existing environment and admitted-owner
coordinates removed all four. General detached and stale entry points still
validate identities and reject retired values.

## Exact warmed structural gate

`benchmarks/tex-command/src/bin/packed_cutover_gate.rs` measures ordinary
source delivery, packed backup and replay, stored replay, and macro matching,
argument replay, and expansion after warmup. Every row asserts the following
exact vector:

| Counter                      | Each row |
| ---------------------------- | -------: |
| Allocation calls             |        0 |
| Requested bytes              |        0 |
| `Arc` retains                |        0 |
| Weak retains                 |        0 |
| Weak upgrades                |        0 |
| Weak-index calls             |        0 |
| Weak-index candidates        |        0 |
| Weak-index exact comparisons |        0 |
| Weak-index content hashes    |        0 |

The gate also asserts exact delivered token values for every row. It passes at
commit `55154ac7bc95176a896af497da88958a6564b329`.

## Immutable prefix evidence

The guarded run reused the frozen 12,000,000-fuel authority with no competing
Cargo, Rust, Umber, perf, or Samply process. The source archive, selected
`ArXiv.tex`, schema-11 format, schema-3 distribution, and ordered 105-key
closure retained SHA-256 values
`05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`,
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`,
`560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`,
and `75d85bb12f8fa5eba0ae2a42daf73fd86c44852ecdc230196455b9aea24565b5`.
The 528-file cache inventory was byte-identical before and after, acquisition
was absent, the typed result was status 1, stdout was empty, and no PDF or
input record was published.

| Command-work counter  | Frozen census | Current audit |
| --------------------- | ------------: | ------------: |
| Fuel charges          |    12,000,000 |    12,000,000 |
| Token-frame steps     |    11,999,815 |    11,999,815 |
| Expanded deliveries   |     1,253,905 |     1,217,976 |
| Meaning lookups       |     3,485,522 |     3,259,209 |
| Scanner-status tokens |    10,639,582 |    10,902,207 |
| Write expansions      |         1,136 |         1,050 |

This satisfies the versioned direct-prefix gate: fuel and raw frame position
are exact, the exhaustive semantic tracer is clean, and the secondary deltas
are recorded without charging work that no longer occurs. Relative to the
historical vector, expanded deliveries are `-35,929`, meaning lookups are
`-226,313`, scanner-status tokens are `+262,625`, and write expansions are
`-86`. The scanner increase is an endpoint-mix redistribution within the exact
11,999,815 raw frames, not additional raw work. The current row took
34.07 seconds wall with 707,760 KiB peak RSS under the unchanged 120-second,
1,536-MiB guard; those timing values remain diagnostic across the transaction
contract change. The local evidence is under `target/umber2-awgc.3.4`; its
`evidence.sha256` file has SHA-256
`f467191567a0f5a85c92d394ba92bc6e2e028b36952aeec7af08e7245f80259b`.

## Earliest transition

The required exhaustive `tex-command-stream` run reported zero ordered
semantic or geometry divergences across its entire fixture set. A focused A/B
history check then established the transaction boundary:

| Revision    | Observation                                                                                                                                 |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `dd16ecb1e` | Frozen accepted six-counter vector.                                                                                                         |
| `239df80c2` | Immediately before the direct-operation cutover: every counter exact except expanded deliveries at 1,253,908, three above the frozen value. |
| `18077d0fe` | Earliest large work-vector transition: `perf(tex-exec): commit ordinary commands directly`.                                                 |
| `22c8bfb59` | Later packed work exposes the pre-existing stale macro diagnostic lookup and stops at roughly 54,692 delivery/scan boundaries.              |
| `4ab5d3362` | Integrated packed ownership has the same stale diagnostic stop.                                                                             |
| `55154ac7b` | The lifetime defect is fixed; the run reaches fuel and exposes the changed transaction/accounting vector above.                             |

`umber2-awgc.12` proved that the transition is the first resource miss after a
successful direct prefix. The old aggregate transaction rolled back and later
re-executed the whole prefix; the direct path commits it and retries only the
blocked command. That eliminated work must not be restored or synthetically
charged. Packed ownership neither owns nor caused the delta.

## Validation

The focused `tex-command` suite, the assertion-bearing packed cutover gate,
the exhaustive canonical command tracer, and `cargo test -q --tests` pass.
`scripts/check.sh` reports all four dprint, Biome, rustfmt, and declared clippy
gates passed.
