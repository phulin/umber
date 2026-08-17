# Episode Deletion Boundary Audit

Status: deletion is blocked on profile-wide canonical episode coverage.

Issue `umber2-64v2.16` is the final convergence stage selected by
[`engine_architecture_decision.md`](../engine_architecture_decision.md). The
audit at `640987aa4` found that the required cutover has not happened across
the supported TeX82, e-TeX 2.6, and pdfTeX 1.40.29 profiles. Deleting the live
scalar command machine now would delete required semantics. Deleting the
packed episode instead would discard the exact `.12` performance result. A
fallback rename or an unobserved-suite zero count would conceal the gap.

## Current authorities

The following are canonical production owners and remain in scope:

- `EngineSession` is the retained host lifecycle and `MainControl` is the sole
  public execution front.
- `CommandState` and `CommandProcessor` own the complete tokenizer, source and
  token-list input stack, raw and expanded delivery, macro matching, scanners,
  conditions, and alignment delivery for all supported profiles.
- `Universe` owns the dense environment backing, typed group journal, exact
  identity, dependency state, effects, and the aggregate local-retry snapshot.
- `NodeListBuilder` is the sole mutable node builder. `NodeListRef` is the sole
  published node-list owner.
- `EpisodeCommit`, `EpisodeCommitBoundary`, `SemanticEpisodeBarrier`, and the
  fixed operational telemetry describe required boundaries of the same live
  engine.
- Committed command fixtures, property catalogues, focused adversarial tests,
  selected corpora, TRIP/e-TRIP, and artifact/DVI/PDF fixtures remain external
  evidence. They are not executable substitutes for production semantics.

## Migration-only path still live

`tex_command::NativeBatchProgram` is not yet the canonical command machine.
It is a root-only admitted program with its own `Vec<Token>`, `Frame` stack,
bump-owned macro arguments and bodies, `next_raw`, `next_expanded`, integer
scanner, conditional skipper, and `Control` dispatcher. Its complete literal
control vocabulary is:

```text
count def e f advance global ifnum else fi shipout hbox kern relax end begingroup endgroup
```

The two one-letter controls are fixed benchmark macro slots rather than
ordinary control-sequence meanings. State access is limited to the count bank
and hbox/semi-simple group journal. Node construction accepts only character
`A` and explicit kerns inside one shipped hbox. The executor then synthesizes
the root box and uses the ordinary shipout transaction.

`EngineSession` admits every root through this program. Admission or execution
refusal restores or leaves untouched the aggregate and resumes the complete
`CommandProcessor` path. That is a safe migration protocol, but it is still a
live coverage fallback between two delivery, expansion, scanning, and dispatch
implementations.

The benchmark-local whole-job scalar comparison adapter and its
`canonical|production|compare` selector are deleted by this audit. The
production workload remains as the exact 6M/12M/nested performance gate;
correctness stays with external fixtures and oracle workloads. The unused
`NodeOrFont` fallback-family counter is also deleted.

## Exhaustive fallback inventory

| Family               | Producer and present boundary                                                                                                         | Supported-profile gap                                                                                                                                                                                                 | Deletion proof required                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `CharacterProfile`   | `CharacterMode` admission refusal                                                                                                     | UnicodeExtended is refused. It is outside the three exact-profile acceptance rows but remains a supported Umber mode and cannot be silently consumed.                                                                 | Give Unicode a typed noncoverage disposition or canonical episode representation; no runtime selector.                        |
| `SourceTokenization` | source registration, invalid/unsupported character or catcode, material after `\end`, or missing `\end`                               | Only one generated exact-byte root is represented. Files, streams, terminal input, `\scantokens`, category changes, backup/template levels, and complete root framing remain in `CommandState`.                       | Zero fallback in physical-input, recovery, command fixtures, and selected corpus; one source/input frame owner.               |
| `CommandVocabulary`  | every control sequence outside the sixteen-name table                                                                                 | Nearly the complete TeX82 vocabulary and every e-TeX/pdfTeX extension reject. Some resource/effect/diagnostic names are separately classified as required barriers, but that small name list is not family coverage.  | Exhaustive static profile dispatch plus property, fixture, adversarial, and corpus zero-fallback evidence.                    |
| `ScannerOrExpansion` | malformed admitted grammar or arithmetic overflow after mutation begins                                                               | Only one-argument fixed-slot macros, unsigned decimal integers, count assignment/advance, and `\ifnum` exist. Canonical macro grammar, expansion, scanners, conditions, alignments, and their recovery remain scalar. | Exact observer/event/diagnostic parity and deletion of the private frame, macro, scanner, and conditional implementations.    |
| `GroupLineage`       | committed ordinary-operation coverage boundary                                                                                        | This is not an admission fallback. It stops a bounded scalar episode after a group mutation so it cannot replay a committed action.                                                                                   | Replace the implementation-only stop only after canonical group and builder publication can continue safely.                  |
| `RollbackLineage`    | committed-failure ordinary-operation coverage boundary; the packed state barrier maps here only after required-barrier classification | This is primarily scalar aggregate-lifecycle evidence, not permission to run a second engine.                                                                                                                         | Preserve the sole aggregate retry authority; remove only migration-only boundary accounting proven unreachable after cutover. |

`NodeOrFont` had no producer: it appeared only in telemetry enumeration tests.
It was genuinely unreachable and is removed now. A missing `A` metric is a
typed diagnostic barrier, while all other material rejects earlier as command
or grammar coverage.

Required `Resource`, `Effect`, `Diagnostic`, and `Format` admission barriers
are not coverage fallback. They remain, but their current recognition by a
small spelling list is not evidence that the corresponding semantic families
execute through the packed command machine.

## Evidence limitation

The `.12` and `.15` exact comparisons and 6M/12M/nested measurements are valid
for the closed workload above. They prove one state owner, aggregate rollback,
typed barriers, output equality, and the performance ceiling for that slice.
They do not prove supported-profile closure.

The command-semantic runner constructs `MainControl` with
`register_root_source` and drives `step_with_observer`. An observer is a
required episode barrier, so those fixtures deliberately exercise the
complete scalar `CommandProcessor` and never attempt packed admission. A zero
fallback count from that route is therefore vacuous. The property catalogue,
focused adversarial tests, and most corpus jobs likewise do not currently
assert that each supported family entered the packed implementation.

## Ordered blockers

The remaining cutover is split by semantic ownership, with dependencies in
Beads:

1. `umber2-uvfm` canonicalizes source and input frames for supported exact
   profiles.
2. `umber2-3gln` depends on it and canonicalizes expansion, scanners,
   conditions, and alignments.
3. `umber2-c1p8` depends on both and canonicalizes dispatcher, state, node,
   output, effect, resource, and retry families.
4. `umber2-64v2.16` depends on that chain and performs the final static
   deletion, zero-fallback audit, complete correctness hierarchy, and exact
   `.12` performance gates.

Until those blockers close, the valid deletion boundary is narrow: delete
only adapters with no semantic caller, retain scalar semantics, and keep every
fallback explicit and counted.
