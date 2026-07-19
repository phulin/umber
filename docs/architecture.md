# Engine Architecture

Status: current implementation contract.

This document describes the architecture implemented by the workspace today.
It deliberately omits completed migration plans and benchmark histories; Git
and Beads retain those records. State representation and mutation invariants
are specified in [core_state.md](core_state.md).

## 1. The big picture

Umber is a TeX interpreter split at semantic ownership boundaries:

```text
World/resources
      |
   tex-lex  ->  tex-expand  ->  tex-exec  ->  tex-typeset
                                  |               |
                                  +---- tex-state-+
                                           |
                                        tex-out
                                           |
                                     DVI / HTML / artifacts

umber / umber-wasm own host policy and persistent compile sessions.
tex-incr owns edit mapping, checkpoint selection, convergence, and reuse.
```

The pipeline crates never perform host I/O directly. They receive external
facts through `World`, mutate engine state through `Universe`, and publish
detached output only after the relevant transaction commits.

## 2. Crate map

- `tex-arith`: shared TeX scaled-point and TFM arithmetic.
- `tex-content`: versioned, domain-separated content identities.
- `tex-fonts`: immutable TFM and OpenType parsing, validation, and metrics.
- `tex-state`: all mutable semantic state, immutable content stores, history,
  effects, source/provenance data, node arenas, and snapshots.
- `tex-lex`: line normalization, tokenization, input frames, and token replay.
- `tex-expand`: expansion, expandable primitives, conditionals, and value
  scanners.
- `tex-exec`: stomach execution, mode nest, assignments, page building,
  output routines, and committed shipout lowering.
- `tex-typeset`: pure packing, line breaking, alignment planning, vertical
  breaking, and Appendix G math conversion. Packing preserves TeX's distinct
  `\badness` results: 10000 for infinitely bad adjustment and 1000000 for a
  nonempty overfull box whose normal-order glue cannot shrink far enough.
- `tex-out`: detached artifact schema 13, positioned output, HTML, DVI page
  planning, final DVI assembly, and artifact replay.
- `tex-incr`: editor revisions, named-boundary checkpoints, convergence,
  pruning, retained output, and rendered-source queries.
- `umber-vfs`: canonical virtual paths, immutable file identity and ownership
  layers, domain-qualified file requests, atomic provisioning, and file limits.
- `umber`: CLI and host-neutral `VirtualCompileSession`.
- `umber-wasm`: JavaScript/WASM representation adapter and browser package.
- `test-support` and `corpus-manifest`: test and corpus tooling support.

Pipeline dependencies point toward lower-level data contracts. `tex-state`
does not depend on expansion or execution; `tex-out` does not observe live
state; and nothing in the engine depends on `tex-incr`.

## 3. Input layer

`World` owns input bytes, stream state, terminal output, the fixed job clock,
randomness, and deferred effects. Native filesystem search and browser
resource acquisition are host policies above the engine.

Every opened input is registered with immutable content identity and source
metadata. The lexer consumes normalized lines from an `InputStack`. Persistent
sessions represent the editable root as immutable fragments plus a revisioned
piece layout, while included resources remain immutable for the accepted
session history.

Missing files and fonts in virtual sessions become typed resource requests.
They are sorted, deduplicated, and returned in one `NeedResources` batch;
Rust never invents URLs or performs asynchronous I/O.

The native CLI's resource host policy lives in `umber::cli_resource`: it
preserves `TEXINPUTS`/`TEXFONTS` candidate ordering, then consults the verified
native object cache and a pinned HTTPS or explicit local manifest. Network and
cache I/O remain above the session and engine boundaries in `umber-fetch`.
Native CLI discovery permits up to the validated hard ceiling of 128 compile
attempts so a large local TeX closure can be discovered incrementally; the
host-neutral session default remains 32, and both paths reject unbounded retry.
The CLI input receipt accumulates successfully resolved required local files
across those attempts, rather than reporting only the final resumed suffix.

`umber-vfs` owns file request identity, required-versus-hint file batches,
generic response validation, request-bound resolved origins, layered session
input storage, snapshots, generated-output transactions, and file limits.
`VirtualCompileSession` resolves TeX inputs and TFM files from an immutable
stage snapshot and asks `World` to register the selected shared bytes, rather
than retaining or seeding a parallel file map. The TeX driver owns extension
and search policy and combines the resulting file needs with font requests.
After a successful World effect commit, the driver copies complete auxiliary
files into the stage write set. Editor execution first produces an opaque
prepared `tex-incr` revision; its root bytes and generated writes live in a
private copy-on-write VFS generation until diagnostics, DVI/HTML, and output
limits validate. The driver then accepts the prepared revision and swaps the
VFS generation as one externally atomic operation. Native and WASM adapters
share the same Rust domain/kind wire vocabulary.

## 4. Lexer

`tex-lex` implements TeX line normalization, `^^` processing, Unicode-aware
character delivery, catcode lookup, control-sequence scanning, endline rules,
and input-stack precedence. Tokens carry packed provenance.

Replay frames have two representations:

- stored token lists for durable definitions and registers; and
- pooled transient `TracedTokenWord` buffers for arguments, pushback,
  inserted tokens, and rendered expansion results.

Transient execution does not intern token lists merely to re-enter the input.
Checkpoint summaries own the remaining transient words by value.

## 5. Expansion engine

`tex-expand` owns the gullet loop, macro argument matching, replacement replay,
expandable primitives, conditionals, and integer/dimension/glue scanners.
Expansion mutates state only through `Universe` and pushes resulting tokens
through `InputStack` APIs.

Macro bodies and parameter patterns are durable content. Macro arguments and
temporary rendered values remain transient unless a durable state or future
memoization boundary explicitly publishes them.

Each top-level request for an expanded token has a fixed work budget. Nested
expansion performed while satisfying that request shares the same remaining
budget, so a recursive macro or primitive cannot reset the guard. Once a token
is delivered, the next independent request receives a fresh budget; ordinary
long-running jobs therefore are not charged for unrelated, already-completed
expansion chains.

Execution-driver scans preserve tokens protected by `\unexpanded` for their
enclosing expansion call. Scanner points that begin a fresh command demand
opt in explicitly; this includes the post-character-constant lookahead in
integer scanning, which must resume a replayed macro before deciding whether
to consume an optional space.

End of source input is accepted only when the expansion loop is quiescent. If
a source-origin macro argument scanner reaches root EOF while matching a call,
its incomplete-call error crosses both ordinary and restricted expansion
boundaries. The legacy clean-EOF recovery is limited to calls originating in
an inserted token-list replay, which preserves format-loaded TRIP behavior
without hiding a prematurely exhausted source job.

## 6. Execution engine

`tex-exec` owns primitive dispatch, grouping, assignments, modes, box building,
alignments, math-list construction, page building, output routines, and
shipout transactions. Its mode nest is explicit and snapshot-capable.

Assignments use the aggregate state API. Group restoration, global writes,
register overflow, code-table updates, and effect recording all pass through
the same barriered ownership boundary. Recursive scanners may use ordinary
Rust locals, but only executor-named quiescent boundaries are restartable.

Shipout decodes compact node words sequentially and drives artifact encoding
and DVI page-plan construction without an ordinary-path owned page tree. A
localized owned payload remains permitted where DVI leader replay requires
repetition. Artifacts, effects, and prepared plans publish only after the
shipout transaction commits.

## 7. Typesetting kernels

`tex-typeset` is state-free. Callers copy the required parameters and expose
fonts, glue, and nodes through narrow immutable traits. Kernels return owned
results or drive a narrow execution-owned sink.

Packing and line breaking preserve TeX.web arithmetic exactly. Appendix G
math conversion builds one span-backed `MathLayout`; `FrozenHList` values are
handles into that arena, not recursive owned vectors. Execution lowers the
completed layout through `MathLayoutSink` into state-owned node lists.

## 8. Page builder and output routine

The page builder lives in `tex-exec` with its mutable roots owned by
`Universe`. It tracks the contribution queue, page dimensions, insertion
classes, best break, marks, and output-routine state in TeX order.

When the best break fires, execution packages the page into `\box255`, enters
the output routine when configured, and resumes page building afterward.
Outermost completed shipout is both an effect commit boundary and one of the
named incremental checkpoint opportunities.

## 9. Fonts and metrics (`tex-arith`, `tex-fonts`, `tex-state`)

Classic TeX layout uses immutable TFM metrics and exact scaled arithmetic.
Font selection is state, while parsed metrics and validated OpenType programs
are immutable resources.

Native sessions accept OTF/TTF containers and browser sessions accept WOFF2.
The host chooses resources; `tex-fonts` validates them, derives canonical
program identity and metrics, and records the selected program/instance in
committed artifacts. HTML reuses retained accepted font bytes rather than
performing a second delivery phase.

## 10. Output drivers (`tex-out`)

Artifact schema 13 is the durable, content-addressed page representation.
`tex-out` owns its validation, encoding, replay, positioned-event projection,
HTML schema 1, DVI page plans, and final DVI assembly.

Fresh shipout builds canonical artifact bytes and a detached `DviPagePlan` in
one execution-driven traversal. Durable replay validates the requested content
identity and schema before producing output. `tex-out` never receives
`Universe`, node handles, or mutable store access.

DVI remains the exact compatibility output. HTML preserves exact TeX page,
box, rule, baseline, and run-anchor coordinates while browser shaping owns
glyph placement inside a text run.

Generated pdfTeX font instances retain their immutable construction and
original-source parameter records in format images, keeping downstream
artifacts independent of live engine state.

## 11. Incremental engine (`tex-incr`)

`tex-incr` retains accepted editor revisions, named checkpoints, artifacts,
effects, source fragments, and pruning metadata. V1 restart boundaries are
`JobStart`, eligible `OuterParagraphEnd`, and outermost `ShipoutComplete`.
Checkpoint publication remains executor-controlled.

`VirtualCompileSession` composes resource acquisition with revision-checked
root patches. Acceptance is atomic across incremental history, the synthetic
VFS root, generated files, diagnostics, artifacts, and rendered output: a
resource miss or failed revision does not replace prior accepted state.
Full canonical-state convergence can reuse the accepted suffix. When page
state does not converge, changed-content reuse uses the aligned ordered
paragraph cursor in [incremental_memoization.md](incremental_memoization.md).
Paragraph replay validates recorded semantic observations before mutation and
fails closed; it uses stable source mapping and has no reverse suffix hash or
tierarchical execution trace. Accepted hlists and finished lines retain only
compact piece-relative provenance recipes for reachable output origins; replay
mounts those origins through ordinary survivor-node traversal rather than
retaining an expanded-token trace. The separate schedule-aligned suffix-adoption fast path
treats a deterministic fixed-seed 64-bit aHash over canonical current-state
projections as authoritative: it has no SHA-256 or structural fallback, and the
rare possibility of collision-induced incorrect reuse is an accepted
session-local performance tradeoff. Durable
content, artifact, format, file, and font identities retain their cryptographic
contracts and do not inherit this weaker trust model.

Rendered-source queries validate output identity and revision, then resolve
artifact provenance through the current fragment layout. Removed content is a
typed deletion, not an aliased coordinate.

## 12. JIT

`tex-jit` is not in the workspace. If introduced, it is the only sanctioned
consumer of a sealed state-layout surface and must preserve the same write
barriers, effect capabilities, validation, and deoptimization semantics as the
interpreter. No existing crate exposes raw store mutation in anticipation of
it.

## 13. Cross-cutting invariants

1. All semantic mutation flows through `Universe`/aggregate store APIs.
2. All I/O, clocks, randomness, and observable effects flow through `World`.
3. Runtime handles are owner- and generation-validated and cannot be forged.
4. Immutable content is built privately, validated, then frozen.
5. TeX arithmetic and ordering rules are ported exactly.
6. Output crosses the live-state boundary only after transactional commit.
7. Durable identities exclude allocation order, provenance, and host paths.
8. Incremental reuse is optional; cold execution defines correctness.
9. Native and WASM hosts observe the same engine and session semantics.
10. Production crates contain no unsafe code; a future sealed JIT is the only
    possible exception.

## 14. Portable format images

`tex-state` owns a fixed-width little-endian frozen-format container beginning
at schema 10. Its canonical section directory, alignment and zero-padding
rules, whole-image checksum, exact ABI and lookup-configuration fingerprints,
relative/index reference model, validation order, literal hash-table layout,
and immutable-versus-job-local state boundary are specified in
[frozen_format.md](frozen_format.md).

Schema 10 installs authoritative fixed-width sections for names, lookup
indexes, token lists, macros, glue, fonts, code tables, hyphenation,
environment cells, and reachable node graphs. The historically named section
1 contains only Universe-level interaction and PDF configuration metadata; it
is not a schema-9 compatibility payload. Schema 9 itself is rejected rather
than guessed or reinterpreted, and formats are regenerated from source.
Loading never publishes partially validated state, persists Rust heap layout,
or mutates frozen bytes. Job-local clocks, input, page state, journals,
effects, and mutable overlays are constructed fresh.
