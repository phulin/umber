# Native Batch-Kernel Ceiling

The evaluation branch for `umber2-64v2.8`, based on
`fef7f55637842721d9ec86dc4b40c475c3a80f25`, established a production-shaped
direct-execution ceiling. Issue `umber2-64v2.10` independently reproduced that
ceiling and migrated its first bounded vertical slice into production-owned
`tex-command` and `tex-exec` code. The result is a migration seam, not a
supported runtime engine choice.

## Independent audit and migration result

The independent audit rebuilt the original release executable from
`d3128fdf17b5ee9c158eb72ab49e8e077209b6c6`, ran the exact differential, and
then collected three fresh guarded processes per row. The direct 6M-fuel
median was 8.397 s and 225,308 KiB for production versus 133.935 ms and 47,600
KiB for the prototype: 62.7x faster with 4.73x lower peak RSS. The nested
median was 2.010 s and 55,892 KiB versus 192.839 ms and 13,144 KiB: 10.42x
faster with 4.25x lower peak RSS. The smaller direct ratio than the original
capture is ordinary host variance; exact state, artifact, bytes, DVI, effects,
terminal, and log comparison still passed.

The audit therefore confirms the claimed order of improvement under the
stated measurement contract. It does not reinterpret the direct loop as a
drop-in engine. The prototype deliberately has no per-step checkpoint,
provenance, reachability, or observation publication inside its coarse
episode. Those are barriers to batch around or materialize compactly, not work
that may be silently omitted from an observable incremental episode.

The migrated production seam is split by existing authority:

- `tex-command::NativeBatchProgram` uses the canonical exact-byte tokenizer,
  current category codes, and end-line policy for a complete pre-mutation
  admission pass. It then owns the packed macro, expansion, scalar scanner,
  conditional, count-assignment, group-save, and node-command episode.
- `tex-exec::run_native_batch_episode` owns font-metric projection, canonical
  `PageArtifact` validation and serialization, artifact reparsing, and DVI
  compilation and serialization.
- `NativeBatchFallback` is the exact boundary for an unsupported character
  mode, category, control sequence, malformed supported episode, or missing
  font character. Admission and execution mutate no `Universe` or host output,
  so a caller can enter canonical stepping from its original state. Effects,
  resources, observations, and checkpoints are not admitted.
- The benchmark-local lexer and kernel were deleted. The differential and
  process runner now call the production seam, preventing benchmark code from
  becoming a production dependency.

After migration, three fresh guarded samples gave a direct median of 8.055 s
and 225,284 KiB for canonical stepping versus 178.308 ms and 51,628 KiB for
the shared production episode: 45.2x faster with 4.36x lower peak RSS. The
nested medians were 1.902 s and 56,200 KiB versus 204.186 ms and 16,196 KiB:
9.32x faster with 3.47x lower peak RSS. The direct production-routed slice
therefore clears the 10x end-to-end gate. The nested reduction from the
prototype is explained by paying the canonical tokenizer during admission;
that cost remains inside the measured region rather than being hidden in
workload construction.

## Canonical count and group migration

Issue `umber2-64v2.11` removed the episode's private 256-word count table,
per-cell level table, first-local-write save vector, and group-mark vector.
`tex-state::CountGroupEpisode` now borrows the live `Universe`: count reads and
writes address `Env`'s canonical fixed bank, while `\hbox`, `\begingroup`, and
their matching exits use the ordinary typed group markers and undo journal.
The scalar `Universe` assignment/group APIs and the packed episode therefore
observe one value and one restoration history. There is no synchronization
between semantic stores because there is no second semantic store.

The coarse borrow coalesces dependency and exact-identity publication only
until the next group or episode boundary. Tracked observations and observable
group/restoration tracing are typed admission barriers. The enclosing
`tex-exec` operation owns a canonical local-retry snapshot, so a semantic
barrier or artifact/DVI failure restores the count bank, group stack, journal,
dependency tracker, and state-hash roots atomically. Completed state passes
adversarial nested local/global restoration, format round-trip, exact state
hash, and snapshot rollback tests.

A fresh guarded 6M-fuel differential remained exact after the migration.
Canonical stepping took 8.226 s and 225,272 KiB peak RSS; the shared episode
took 241.339 ms and 85,612 KiB. The canonical-state path is therefore 34.1x
faster and uses 2.63x less peak RSS. Its 234 allocation calls remain five
orders of magnitude below canonical stepping's 8,423,813. The increased bytes
and peak versus the private-array ceiling are the real cost of retaining the
canonical global-assignment journal until the enclosing hbox closes, rather
than an omitted rollback obligation.

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

The original native kernel had one byte lexer and direct control-sequence
lookup, packed 32-bit tokens, a packed source/token cursor stack, bump-scoped
macro bodies and arguments, fixed mutable count and level arrays, and a
first-local-write save stack. The migrated slice replaces its lexer with the
canonical source tokenizer; the later count/group migration replaces all four
private count/restoration structures with `tex-state`'s direct bank and
journal. Checkpoint, provenance, reachability, and observation objects do not
cross the bounded loop; their canonical publications occur at its barriers.

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

1. The first production batch-episode interface now lives in `tex-command` and
   `tex-exec`. Its closed admission surface stops before an effect, resource,
   observation, or named checkpoint barrier. The existing public driver
   remains the only general lifecycle owner.
2. The count family now uses direct `tex-state` bank and journal storage for
   both scalar and packed execution, and its private rollback path is deleted.
   Continue the same delete-as-migrated rule when later integer families enter
   coarse episodes.
3. Replace the command machine's source/token levels internally with packed
   cursor frames and bump-scoped expansion values. The first slice already
   shares canonical source tokenization and refuses unsupported category-code
   behavior before mutation. Next, make the packed token, meaning, scanner, and
   diagnostic primitives the canonical implementations, add external-input
   barriers, and delete each superseded frame representation as its cases pass.
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
7. The benchmark-local fused lexer and executor are removed. Retain the
   production regression workloads and semantic/performance gates; once main
   control uses the shared storage, cursors, loop, and builders, remove the
   comparison adapter and the old layered hot path family by family.

Every stage requires exact result, diagnostic, effect, artifact, and DVI
differentials; adversarial grouping and scanner tests; 6M/12M scaling; allocation
and RSS gates; and incremental named-boundary recovery tests. Promotion proceeds
by semantic family, and a family is complete only when its old hot path is
deleted. This preserves canonical primitives while converging on one engine.
