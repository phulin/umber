# Aggregate checkpoint component contract

Status: normative target, measured pre-refactor baseline, and command-family
promotion evidence, 2026-08-27.

## Scope

This document fixes the ownership and measurement boundary for
`EngineCheckpoint`. It precedes the command, mode, page, hyphenation, and
World ownership-family migrations. It does not authorize a cache, fast path,
per-value owner, root registry, compaction, a third generation, or heap
indirection. The completed PDF scalar mark is reused unchanged.

One session owns an accepted prior generation and, only while executing an
edit, one candidate current generation. A checkpoint contains bounded scalar
cursors plus one coarse generation owner. Every live runtime value has one
owner named below; a checkpoint is a reachability root, not another mutable
state owner.

## Component matrix

| Component                                                                     | Sole live owner now and finally                                                                         | Historical baseline capture                                                                                               | Final checkpoint mark or root                                                                                                                 | Restore order after complete validation                                                                                                                                   | Checkpoint retention charge                                                                                                                                            | Complete reachable-state identity                                                                                                                                                           |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Dense command environment, definitions, durable nodes, provenance             | `Universe::core` in the current retained-generation slot                                                | `GenerationOwner` plus `JournalCursor`, durable-node cursor, and conservative page-node cursor; no dense-bank clone       | The same one coarse generation owner and bounded state/journal/arena cursors                                                                  | Acquire coarse owner; replay dense-state journal; transfer command/mode/page roots before truncating durable and page suffixes                                            | Charge the generation substrate once across every checkpoint that shares it; charge checkpoint-exclusive cursor bytes per record                                       | Include every reachable live cell after resolving macro, token, glue, font, and node values; exclude dead arena suffixes and physical ids                                                   |
| Command delivery, input, parameters, conditions, groups, replay, diagnostics  | The one `CommandState` owned by `MainControl`                                                           | `publish_summary` deep-cloned `CommandStateRoots` into one private `Rc`; clone aliased that root; restore cloned it again | One command-timeline cursor tuple rooted in generation-owned logical stacks and compact scalar undo; no per-checkpoint aggregate vector clone | Validate profile, generation, serial, all cursors, quiescence, and roots; return the sole live loan; restore command scalars and logical tops; discard the current suffix | Charge generation-owned append and undo storage once, then only the fixed cursor/profile/source-anchor record per checkpoint                                           | Include profile, future input bytes/cursors, parameters, conditions, groups, replay fences, pending semantic diagnostics, and required source/provenance roots                              |
| Mode nest and open mode lists                                                 | The one `ModeNest` owned by `MainControl`                                                               | `ModeNest::summary` clones every level and its list                                                                       | Generation-checked mode-journal cursor plus bounded level/list root coordinates                                                               | Validate every mode/font/page root; restore level scalars and roots before state arena truncation; retain the job-lifetime maximum-depth counter                          | Charge shared generation list storage once and the fixed mode cursor per checkpoint                                                                                    | Include mode order, entry lines, semantic/physical list content, pending character state, alignment/list fields, and referenced fonts; exclude journal capacity and maximum-depth telemetry |
| Page builder, lists, insertions, and marks                                    | `Universe::page` plus the generation page-node arena                                                    | Deep-clones `PageBuilderState`; explicit page handles raise the generation's monotonic retained page bound                | Fixed page-builder scalar/list cursors plus the existing generation page-arena bound                                                          | Restore page roots after dense state and before page-arena truncation; truncate only after command and mode roots transfer                                                | Charge page/node segments once per generation and fixed page cursors per checkpoint; never charge shipped detached artifacts here                                      | Include contribution/current/discard lists, page dimensions/integers, insertions, marks, best break, and fire-up state                                                                      |
| Hyphenation                                                                   | `Universe::hyphenation`                                                                                 | Deep-clones language maps, trie vectors, exceptions, hyphen-code maps, and dependency projections                         | One frozen initialized pattern-trie root shared by the generation, plus journal cursors for mutable exceptions and saved hyphen codes         | Validate frozen-root identity and mutable marks; install the frozen root; reverse mutable journals; truncate exception/code suffixes                                      | Charge the frozen trie once per generation; charge journal blocks once while any checkpoint needs them and fixed marks per checkpoint                                  | Include initialized patterns, exceptions, saved codes, capacities that affect TeX overflow, and `patterns_open`; exclude memoized dependency projections                                    |
| World effects, streams, artifacts, clocks, randomness, and host-visible state | The one `World` inside `Universe`                                                                       | `World::snapshot` clones maps and reconstructs effect-root ancestry while sharing several `Arc` vectors                   | Fixed effect, stream, artifact, input, publication, clock/random, and file-framing cursors rooted in one coarse World generation              | Validate forkable/retained ancestry; restore effects and stream buffers; restore artifact/input/publication roots; then release abandoned suffix owners                   | Charge retained effect/artifact/input blocks once per generation or output owner and fixed cursor bytes per checkpoint; detached accepted output is charged separately | Include every fact that can change future TeX behavior or emitted effects/artifacts, including stream partial lines, clocks/random state, shell policy, publication order, and file framing |
| PDF                                                                           | The one `PdfStateSlot` inside `Universe`                                                                | Allocation-free fixed `PdfStateSnapshot`; image/form payload is one coarse prefix plus a private candidate delta          | The existing scalar PDF cursor, absolute general/color undo positions, and coarse payload position; no redesign                               | Reverse general/color undo, restore scalar roots and lookup selections, truncate canonical rows, then truncate payload delta last                                         | Fixed mark per checkpoint; charge the shared image/form prefix once and candidate delta to current                                                                     | Include every canonical PDF scalar, row selection, mutable value, color stack, object/order fingerprint, and payload identity exactly as specified in `pdf_backend.md`                      |
| Dependencies                                                                  | The one `DependencyRuntime` inside `Universe`                                                           | Clones a snapshot that shares the changed-at map through `Arc`                                                            | Fixed invalidation epoch/root cursor in the generation dependency journal                                                                     | Restore after semantic owners and before publishing any observation; no active recorder may cross a checkpoint                                                            | Charge the shared changed-at root once and a fixed tracker mark per checkpoint                                                                                         | Include changed-at facts only where they affect future incremental validation; active recorder state and telemetry are excluded                                                             |
| Sources and fonts                                                             | `SourceMap` and `FontStore` inside `Universe`; command input owns its live source frames                | Existing scalar `SourceMapMark` and `FontStoreMark`; the historical command-root clone separately retained live backing   | Keep the fixed source/font watermarks and one coarse generation owner                                                                         | Validate all source/font coordinates and every command/mode/page/PDF font carrier; transfer roots; truncate font runtime, fonts, then sources before arena suffixes       | Charge immutable source/font payload once per generation or exact external carrier; charge only fixed marks per checkpoint                                             | Include reachable source descriptors/bytes and immutable font recipes plus mutable font-runtime state; exclude unused registered suffixes                                                   |
| Execution and fuel counters                                                   | `MainControl`/session-owned `ExecutionBudgetCounters`, command fuel ledger, and TeX job-lifetime maxima | The two revision budget counters are copied into `EngineCheckpoint`; fuel and stack maxima remain outside semantic roots  | Keep the fixed revision counters only when restart must continue the same configured budget; no heap owner                                    | Restore counters at fork construction before execution resumes; same-generation semantic restore never refunds fuel or TeX high-water diagnostics                         | Exactly `size_of::<ExecutionBudgetCounters>()` per checkpoint; no payload charge                                                                                       | Excluded. These are monotonic operational evidence, not future TeX semantic state                                                                                                           |

All validation is mutation-free. The normative application order is: validate
every owner/cursor/root; acquire the target coarse owner; restore dense state;
restore PDF; restore command, mode, page, hyphenation, World, dependency, and
counter roots; truncate font/source and command/durable/page suffixes; then
release replaced owners. A failed validation leaves source and destination
unchanged.

## Identity decision

Commit `fb81261388a608c16fe26f2cc44084d4574ac2eb` deletes the ignored
ordinary-versus-exact selector, `CheckpointSink::wants_exact_state_identity`,
and `capture_checkpoint_with_exact_identity`. Both paths previously captured
the same state and stored only `ModeNestSummary::semantic_fingerprint` under
the misleading `state_hash` name. That mode fingerprint remains explicitly
mode-only evidence; it is not sufficient for suffix adoption.

A later implementation child must add one explicitly named optional
`reachable_state_identity`, calculated only when the incremental sink requests
it. It is a versioned, domain-separated, fixed-seed 64-bit identity over all
future-reachable semantic components in the final matrix. Ordinary checkpoint
capture omits it. Incremental convergence must fail closed while it is absent;
it must never substitute the mode fingerprint. Identity computation may use
journal-maintained component roots, but may not add a cache, root registry, or
second ownership graph.

## Representative baseline

Run:

```bash
cargo run --release --manifest-path benchmarks/tex-exec/Cargo.toml \
  --bin aggregate_checkpoint_baseline
```

The recorded run used commit
`fb81261388a608c16fe26f2cc44084d4574ac2eb`, `rustc 1.93.0
(254b59607 2026-01-19)`, Cargo 1.93.0, release profile, Linux
`x86_64-unknown-linux-gnu`, one process, and the profiling features selected by
the standalone manifest. Fixture construction and capacity growth are outside
the measurement regions. Times are single-run diagnostics; allocations and
requested bytes are deterministic promotion evidence.

| Fixture               | Boundaries | Capture ns / allocations / bytes | Clone ns / allocations / bytes |   Fork ns / allocations / bytes | Restore ns / allocations / bytes |          Semantic checksum |
| --------------------- | ---------: | -------------------------------: | -----------------------------: | ------------------------------: | -------------------------------: | -------------------------: |
| Minimal, 1 unit       |          1 |              26,315 / 29 / 8,967 |             7,720 / 24 / 7,383 |         450,428 / 138 / 351,551 |             27,343 / 29 / 44,887 |     35,406,262,337,344,433 |
| Minimal, 1 unit       |         32 |          224,137 / 897 / 286,944 |        211,523 / 737 / 236,256 |  3,490,032 / 4,416 / 11,249,632 |        167,714 / 928 / 1,436,384 |  6,640,767,404,918,139,627 |
| Accumulated, 64 units |          1 |           62,772 / 469 / 116,544 |          33,925 / 370 / 84,728 |       435,840 / 1,304 / 596,284 |           78,422 / 438 / 151,968 | 12,141,055,338,598,652,146 |
| Accumulated, 64 units |         32 |   3,600,448 / 14,977 / 3,729,408 | 2,532,809 / 11,809 / 2,711,296 | 9,656,388 / 41,728 / 19,081,088 |   2,326,954 / 14,016 / 4,862,976 | 15,155,759,750,561,970,511 |

For the accumulated one-boundary row, the exact fixture payload charges
printed by the binary are: command 133,120 bytes; mode 24,168; page 158,208;
hyphenation 7,168; World 7,168; PDF payload 0; dependencies 1,024;
sources/fonts 2,048; execution counters 16. The PDF mark itself is fixed scalar
metadata; its independent payload-retention and zero-allocation authority is
the completed `pdf_checkpoint_gate` evidence from `umber2-66p0.23.1`.

At the recorded baseline, capture and restore copied accumulated command, mode, page,
hyphenation, and World state. Checkpoint clone repeats most of those copies.
Fork performs unavoidable construction of one destination generation but also
copies the same accumulated families. The first destination mutation may
currently trigger additional World copy-on-write; this is a deferred
first-mutation copy and is not misreported as capture. PDF payload mutation
already appends to its private delta and copies no prefix.

## World, source, and font ownership outcome

`umber2-pei0.2.4` replaces the World/source/font portion of that historical
baseline with fixed marks and coarse accepted blocks. The concrete ownership
classification is:

| Column or family                                  | Representation at a retained boundary                                                                                                                                | Retained-byte owner and release                                                                                                                       |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Effect records and aligned publication sidecars   | One immutable accepted block per admitted lineage plus an empty destination-private suffix; the checkpoint stores absolute base and aligned scalar lengths           | Each block charges its record payload and six aligned sidecar columns once to the lineage that produced it; a rejected suffix dies with its candidate |
| Publication and semantic counters                 | Current-lineage maps plus one undo journal; the checkpoint stores only the journal length and scalar identity frontiers                                              | Accepted next ordinals are recovered from immutable aligned effect columns, so no counter map crosses or is cloned at fork                            |
| Stream state                                      | Fixed stream-slot state plus effect-position marks; numbered streams retain no unused partial-line mirror because it never changes later TeX behavior                | Terminal/log partial lines and the fixed open/read slots belong to the live World; stream-open context suffixes truncate with their effect positions  |
| Input records and immutable bytes                 | Coarse accepted record/content blocks plus an empty destination-private record/content suffix; the checkpoint stores record and identity cursors                     | Input bytes are charged once to the block containing their record; accepted identities are shared as run blocks and new identities use the candidate  |
| Reduced input dependencies                        | Coarse accepted maps plus a private override map and undo journal; the checkpoint stores the journal length and exact effective count                                | Accepted observations remain in their source lineage block; rollback deletes candidate-only overrides and terminal detachment materializes once       |
| Artifact commits                                  | A retained artifact cursor; a fork starts an empty candidate commit ledger at that cursor                                                                            | Verified/committed bytes cross once into the durable artifact owner; provisional page receipts are quiescent transaction state and cannot checkpoint  |
| Sources                                           | Immutable accepted registration/backing blocks plus private vectors and sparse index for the candidate; `SourceMapMark` remains fixed                                | Descriptor bytes, line indexes, regions, and generated backing are charged once to their accepted block; logical positions and marks are scalar       |
| Fonts                                             | Immutable accepted loaded-font/index/hash blocks plus a private loaded-font suffix; identifier and expansion changes use candidate overrides with bounded undo marks | Loaded metrics and recipes are charged once to their accepted block; mutable identifier/expansion values roll back without copying immutable fonts    |
| Dependency tracker and execution/identity scalars | Revision/invalidation epochs and run-compressed accepted identity metadata                                                                                           | No changed-at map or per-value identity table is cloned; job/revision counters remain fixed scalar state                                              |

Only prior and current lineages are mutable authorities. Accepted blocks are
immutable row storage, not additional generations, and no block registers
roots or compacts/relocates values. Reads select accepted prefix or current
suffix directly; mutation appends to the current suffix or its explicit undo
journal. Restore first validates every mark, reverses mutable journals, resets
scalars, and then truncates suffix columns. A fork shares the exact marked
prefix, excludes source rows after the mark, and allocates only the new
lineage's empty containers.

The enforced `world_checkpoint_gate` compares both 1 versus 64 payload units
and 1 versus 32 retained boundaries. Its recorded optimized result is zero
allocations for capture, checkpoint clone, and no-op restore; World fork is
26 allocations / 1,120 requested bytes at both payload sizes, source/font fork
is 20 / 984, and retained-boundary-only forks are respectively 22 / 888 and
16 / 680 at both boundary counts. First candidate mutation is also flat:
World is 15 / 2,627 and source/font is 26 / 4,706. The fixture reports logical
retained-payload ownership separately (World 848 versus 48,476 bytes and
source/font 2,820 versus 97,068 bytes), demonstrating that real retained data
scales while checkpoint representation and fork setup do not.

## Command ownership outcome

The command family has since passed its standalone promotion gate:

```text
COMMAND_CHECKPOINT_GATE capture=Counts { allocations: 0, bytes: 0 } clone=Counts { allocations: 0, bytes: 0 } restore=Counts { allocations: 0, bytes: 0 } first_mutation=Counts { allocations: 0, bytes: 0 } fork=Counts { allocations: 0, bytes: 0 } fork_first_mutation=Counts { allocations: 0, bytes: 0 }
```

The gate compares one live command unit with 64 accumulated source and
stored-token units, requires identical counts, and verifies rollback plus
repeated-fork isolation. Capture and clone retain fixed marks only. Main-control
drop returns the exclusive command-root loan; fork restores the selected marks
and gives the sole current candidate that root. A rejected candidate returns
the same storage after discarding its current logical suffix.

## Promotion thresholds

Later ownership-family children must retain this baseline and meet these final
gates without weakening any standing gate:

- warmed capture, checkpoint clone, and same-generation no-mutation restore:
  zero allocation calls and zero requested bytes for minimal and accumulated
  fixtures;
- capture, clone, and restore per-operation allocation counts and requested
  bytes are identical at 1 and 64 accumulated units and at 1 and 32 retained
  boundaries;
- one restart fork performs only one destination-generation construction and
  bounded per-revision setup; its allocation/requested-byte counts are
  independent of accumulated units and retained-boundary count;
- no capture copy or deferred first-mutation copy walks an accumulated prefix;
  mutation opens only the destination's private journal/arena/log suffix;
- median optimized elapsed time per operation across at least 31 samples is at
  most 1.25 times the minimal-fixture median for each accumulated fixture; and
- semantic checksums, cold/incremental parity, PDF scalar-mark gates, routine
  tests, and quality gates remain unchanged.

The time ratio is a promotion diagnostic, not a flaky routine-CI assertion.
The zero-allocation and flat requested-byte assertions are deterministic and
belong in the standalone enforced gate when the final family lands.
