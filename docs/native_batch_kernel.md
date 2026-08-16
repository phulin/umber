# Native Batch-Kernel Ceiling

The evaluation branch for `umber2-64v2.8`, based on
`fef7f55637842721d9ec86dc4b40c475c3a80f25`, establishes a production-shaped
direct-execution ceiling. It is evidence for replacing the production hot path,
not a supported second engine and not an integration candidate by itself.

## Covered semantic slice

The direct workload is a complete INITEX job. It initializes count registers,
defines a parameterized macro, and calls it inside a shipped `\hbox`. Every
call performs a local assignment, two global assignments, an `\ifnum` with
both arms exercised, character emission through a synthetic TFM font, and an
explicit `\kern`. Closing the box proves local rollback while preserving the
global values. The page is shipped and the job ends normally.

The structurally different `nested` workload defines a second parameterized
macro. Each outer call forwards its argument into two inner calls. This adds a
macro-body source level, nested argument capture, and two expansion returns
without changing the observable operation mix.

The native kernel has one byte lexer and direct control-sequence lookup, packed
32-bit tokens, a packed source/token cursor stack, bump-scoped macro bodies and
arguments, fixed mutable count and level arrays, and a first-local-write save
stack. Its fused loop performs lexing, expansion, numeric scanning, condition
selection, assignment, and dispatch. It constructs character and kern nodes
directly. Checkpoint, provenance, identity, reachability, and observation
objects do not cross that loop.

The comparison meets production at canonical boundaries. Both paths return the
three count values, validated `PageArtifact`, exact serialized artifact bytes,
a successful parse of those bytes, serialized DVI, effect records, terminal
text, log text, and emitted-call count. The differential tests compare every
field byte-for-byte or structurally. Production command-work counters are the
only extra diagnostic field.

## Measurement contract

`Workload` source construction occurs before the measured region for both
implementations. The region includes fresh engine state, synthetic-font
construction and installation, complete execution, direct output construction,
artifact validation and serialization, artifact parsing, DVI lowering and
serialization, and retention of the complete result. No production-only output
is omitted. The executable black-boxes the retained result before leaving the
region.

Each sample is a fresh process of the same release binary. Times and allocator
counters come from one `stats_alloc::Region`; peak RSS is Linux `VmHWM`. Every
reported value is the median of three isolated samples. Commands use one Cargo
job, one test thread where applicable, a 600-second watchdog, and a 4 GiB RSS
ceiling. The host was an Intel Xeon E5-2650 v4 on Linux 6.8 with Rust 1.93.0.
No native pdfTeX process was run or profiled.

The two direct endpoints are calibrated exactly by production command fuel:
89,551 calls plus 10 `\relax` tokens is 6,000,000 fuel; 179,103 calls plus 26
tokens is 12,000,000 fuel.

| Workload             |     Engine | Median time |    Peak RSS | Allocations | Allocated bytes |
| -------------------- | ---------: | ----------: | ----------: | ----------: | --------------: |
| Direct, 6M fuel      | Production |     8.214 s | 225,368 KiB |   8,423,813 |   1,855,401,315 |
| Direct, 6M fuel      |      Fused |  121.497 ms |  47,600 KiB |          74 |      71,000,297 |
| Direct, 12M fuel     | Production |    16.351 s | 421,532 KiB |  16,841,943 |   3,707,077,387 |
| Direct, 12M fuel     |      Fused |  245.275 ms |  92,004 KiB |          75 |     141,967,817 |
| Nested, 20,000 calls | Production |     1.983 s |  56,008 KiB |   1,845,969 |     412,422,391 |
| Nested, 20,000 calls |      Fused |  189.926 ms |  13,144 KiB |          72 |      16,009,759 |

The direct endpoints are 67.6x and 66.7x faster, use 4.7x and 4.6x less peak
RSS, allocate about 26.1x fewer bytes, and reduce allocation calls by five
orders of magnitude. The nested scanner workload remains 10.4x faster with
4.3x less peak RSS and 25.8x fewer allocated bytes. Doubling direct command
fuel preserves the ratio, so this is steady-state scaling rather than fixed
setup amortization.

## Cycle attribution

Frame-pointer `perf` captures used the profiling Cargo profile. The final
6M-fuel production capture retained over 9,000 samples with none lost and
approximately 23.40 billion cycles. The corresponding fused capture retained
over 1,000 samples with none lost and approximately 308 million cycles. The
76.0x cycle ratio is the same order as the isolated wall-time result despite
different sampling rates and profiler perturbation.

Production's largest flat symbol was `memmove` at 17.8%. Token delivery,
control-sequence lookup, stored-token traversal, expansion, operation dispatch,
and scanners were spread across many symbols. Allocator entry points were also
distributed, consistent with 8.42 million allocations. Snapshot/checkpoint
creation and retirement, state and vector clones and drops, rooted origin and
command resolution, rollback capture, and semantic-dependency observation each
appeared separately. The production counters recorded 5.82 million raw-token
steps, 4.57 million expanded deliveries, and 1.79 million meaning lookups for
the 6M-fuel job.

The fused capture instead concentrated in `next_raw` (35.4%), `next_expanded`
(16.6%), the fused run loop (8.2%), and numeric scanning (4.3%). Artifact
parsing, traversal, construction, validation, and DVI traversal account for
over 20% in aggregate. Canonical output work is therefore paid on both sides
and is already a material share of the direct ceiling. The remaining delta is
not one removable helper: it is the multiplicative cost of representation
movement, allocation, layered delivery and lookup, snapshots, rollback roots,
and provenance/observation work around each semantic action.

## Single-engine migration

Migration must preserve one state owner, one input stack, and one dispatcher.
Temporary differential implementations are evaluation or test scaffolding and
must be removed with the layered predecessor they validate. An unsupported or
observable operation exits a direct episode through a typed barrier into the
same canonical state; it must not transfer into another live engine.

1. Introduce a private batch-episode interface inside `tex-command` and
   `tex-exec`. An episode runs until an effect, resource, observation, or named
   checkpoint barrier. The existing public driver remains the only lifecycle
   owner.
2. Make direct scalar banks, level tags, and the save stack the sole backing in
   `tex-state`, beginning with count and integer families. Existing state APIs
   become accessors over that storage; delete the migrated environment-map and
   rollback path in the same change.
3. Replace the command machine's source/token levels internally with packed
   cursor frames and bump-scoped expansion values. Preserve canonical token,
   meaning, scanner, and diagnostic primitives. Add explicit barriers for
   mutable category codes and external input, then delete each superseded frame
   representation as its cases pass.
4. Move the existing main-control aggregate loop onto fused delivery,
   expansion, scanning, and dispatch episodes. Slow commands call canonical
   helpers against the same mutable state and resume the same loop; there is no
   engine selector or parallel command implementation.
5. Give mode lists mutable native builders and construct ordinary character,
   kern, glue, rule, and box nodes directly. Freeze once at page, checkpoint,
   or externally observed boundaries, retaining `PageArtifact` and DVI as the
   canonical output seams.
6. Materialize provenance, observations, reachability summaries, and durable
   checkpoints from compact sidecars only at their named barriers. Incremental
   execution consumes those committed boundaries, not per-command persistent
   wrappers in the batch loop.
7. Remove this benchmark's fused implementation once production uses the same
   storage, cursors, loop, and builders. Retain only production regression
   workloads and the semantic/performance gates.

Every stage requires exact result, diagnostic, effect, artifact, and DVI
differentials; adversarial grouping and scanner tests; 6M/12M scaling; allocation
and RSS gates; and incremental named-boundary recovery tests. Promotion proceeds
by semantic family, and a family is complete only when its old hot path is
deleted. This preserves canonical primitives while converging on one engine.
