# Compact node-word arena

Status: measurement baseline; representation design is the next phase.

This document records the empirical baseline for replacing the epoch arena's
`Vec<Node>` with a compact word stream. The state and ownership contract is
unchanged: frozen lists are immutable, snapshots capture aggregate arena
watermarks, rollback truncates through `Universe`, and survivor promotion
preserves the bottom-up graph.

## Baseline layout

On `aarch64-apple-darwin` with Rust 1.93.0, the current Rust layouts are:

| Type | Size |
| --- | ---: |
| `Node` | 72 bytes |
| `BoxNode` | 44 bytes |
| `UnsetNode` | 44 bytes |
| `Whatsit` | 48 bytes |
| `NodeListId` | 16 bytes |

The `tex-state` node-arena test asserts these values so representation changes
must update the baseline deliberately.

## Node-kind measurement

The measurement build used `cargo build --release -p umber --features
node-stats` at commit baseline `217878e1`. The feature adds relaxed,
process-local counters at `NodeArena::append`; it does not add fields to
`NodeArena` or `Universe`, and the counters are absent from normal builds,
snapshots, rollback, replay, and semantic hashes. Each node-producing input in
`benchmarks/plain-tex` was run in a fresh process with the committed Computer
Modern metrics and its emitted DVI artifact enabled. Counts therefore include
every epoch-arena append, including intermediate math lists that are later
lowered or released. A fixed `expand.tex` run was bounded at five minutes
after its 100,000 recursive expansion iterations had still not completed.
Inspection shows that the loop does not construct nodes; its only
node-producing suffix is `\hbox{expand 100000\ 0}\benchmarkbye`. That exact
suffix was measured in a fresh process after the same committed preamble,
contributing 56 nodes. This separates the node-frequency question from an
unrelated expansion-throughput wait while still accounting for every
node-producing command in the corpus.

| Workload | Appended nodes |
| --- | ---: |
| `dvi.tex` | 4,331,406 |
| `expand.tex` node-producing suffix | 56 |
| `paragraph-wide.tex` | 1,275,891 |
| `paragraph-narrow.tex` | 655,891 |
| `math.tex` | 6,040,698 |
| `math-nested.tex` | 4,161,034 |
| `pages.tex` | 1,611,063 |
| **Total** | **18,076,039** |

| Node kind | Count | Share |
| --- | ---: | ---: |
| char | 8,224,274 | 45.50% |
| ligature | 9,502 | 0.05% |
| kern | 2,130,236 | 11.78% |
| glue | 1,608,403 | 8.90% |
| penalty | 61,117 | 0.34% |
| rule | 374,702 | 2.07% |
| hlist | 2,857,357 | 15.81% |
| vlist | 561,392 | 3.11% |
| whatsit | 1,001 | 0.01% |
| mark | 8,043 | 0.04% |
| math on/off | 60,012 | 0.33% |
| math noad | 1,960,000 | 10.84% |
| fraction noad | 190,000 | 1.05% |
| math style | 30,000 | 0.17% |

No unset, discretionary, insertion, math-choice, math-list, nonscript, or
adjust nodes were appended by these fixed workloads.

## Scope finding

Char, glue, kern, and penalty nodes account for 66.52% of all appends. They are
the majority, and 94.30% of the DVI-heavy workload, but they are not an
overwhelming majority of the complete suite because the two math workloads
append large numbers of intermediate math noads and hlist/vlist boxes. The
representation design must therefore not justify itself solely from the four
original hot kinds. It should keep the proposed inline forms for those kinds
while explicitly measuring the cost of sidecar-backed boxes and math nodes;
if those sidecars erase the expected kernel benefit, the scope must be revised
before implementation rather than relying on the DVI-only distribution.
