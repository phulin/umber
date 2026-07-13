# DVI artifact fast path

## Goal and boundary

Fresh in-process DVI generation should not reread, rehash, recursively decode,
and revalidate a page that just crossed the shipout commit barrier. The durable
currency remains the canonical, content-addressed page artifact. DVI must never
observe live engine nodes or bytes from a failed shipout transaction.

The implementation has two downstream paths:

- A shipout commit receipt carries the authoritative content id and the exact
  immutable canonical bytes after artifact storage and effect commit succeed.
- ID-only replay reads the durable object and verifies its requested content
  identity before constructing the same validated byte view.

Both paths feed the same DVI traversal and must emit identical bytes.

## Evaluated version-10 indexed view

Version 10 is a prefix-encoded node tree. The evaluated design was a borrowed
`PageArtifactView` that would scan the bytes once and build a compact
structural index:

- job strings, font names, effect strings, and special payloads remain borrowed
  slices;
- decoded scalar fields live in flat node/font/effect records;
- child relationships live in one flat edge array rather than one allocation
  per node list;
- node ids are dense indices, never byte pointers or live-engine handles.

That scan would perform wire validation and semantic validation together. It
would check
magic/version, UTF-8, every tag and length, collection/node/depth budgets,
canonical glue ratios, root box shape, unique and referenced font ids, effect
references, character/token scalars, streams, and trailing bytes. A view is
therefore a capability proving the borrowed bytes are safe for driver access.
No unchecked public constructor would exist.

The existing owned `PageArtifact::from_bytes` remains the compatibility API.
The indexed view was not adopted after profiling: following the committed-byte
and font-loop changes, the complete fresh DVI phase was 254/6068 samples (4.2%)
in a 20-run Gentle profile, including 143/6068 samples (2.4%) in recursive
decode and validation. The prior profile put artifact-to-DVI work near 11%, so
the implemented changes already reduced that phase's share by roughly 2.6
times. A second traversal implementation and validation surface was not
justified for at most about 2.4% additional whole-run upside.

## Failure and lifetime rules

- A commit receipt is recorded only after the aggregate transaction succeeds.
- Any future view cannot outlive its artifact bytes.
- Store replay verifies content identity before view validation.
- Parse or validation failure poisons the current DVI stream exactly as an
  owned-page failure does; no partial page is flushed.
- Receipt bytes are downstream notification history and do not participate in
  rollback or semantic convergence hashes.

## Adoption gate

Reconsider the indexed view only if all DVI fixtures and available Story, Gentle,
TRIP, and e-TRIP corpora remain byte-identical. Reprofile Gentle after the
commit-receipt and indexed-view stages. A new flat artifact version is justified
only if indexed v10 parsing or indexing becomes a material hotspot; the July
2026 Gentle result retained v10 and the owned decoder.

## Precompiled page-plan architecture

The `umber2-xp0` implementation keeps canonical v10 as the only durable page
format. Fresh shipout lowers one direct root child at a time into a typed event
stream consumed simultaneously by the canonical v10 encoder and incremental
DVI compiler. Each temporary child is released immediately; production no
longer constructs, serializes, or retraverses an owned whole-page
`PageArtifact`, and it does not decode the v10 bytes it just made. The
ephemeral `DviPagePlan` is complete before commit. It contains job identity,
page counts and extents, page-local
maximum stack depth, detached font resources, an encoded DVI body, and the
byte ranges occupied by first-use font definitions.

The body excludes `bop`, `eop`, and job framing. Movement-register selection,
glue rounding, leaders, rules, characters, and specials are final page-local
bytes. Font definitions are the sole relocations: final assembly copies a
definition at its recorded first-use position only when an earlier page has
not already defined the same resource. This preserves TeX's exact first-use
ordering while allowing plans to compile independently of prior-page byte
offsets. The assembler owns preamble, `bop` backpointers, global font identity,
postamble maxima, page count, and final padding.

Plan construction happens before commit, but publication happens only after
`ShipoutTransaction::commit` succeeds. The execution result carries plans in
artifact-commit order; no plan contains a live `Universe`, store handle, node
id, or borrowed input. Paths that commit from a nested scanner and cannot yet
propagate their prepared plan reconstruct it from that commit's validated v10
receipt as an explicit compatibility fallback. Failed transactions publish
neither an artifact receipt nor a plan.

Durable ID replay uses the same v10 decoder. It retains metadata and effects,
then decodes, validates, emits, and drops one direct root child at a time rather
than allocating the recursive page tree. Nested material is bounded by the
current child, preserving exact leader repetition and TeX traversal semantics.

Artifact bytes cross the commit boundary as a `VerifiedArtifact`: its private
identity/payload pair computes the artifact-domain content id once. Storage,
commit receipts, and prepared-plan alignment reuse that identity. A real
`World` remembers immutable objects it has already verified, avoiding repeated
file reads and hashes on warm publication while other worlds and durable reads
continue to verify independently. Derived plans never participate in content
identity, snapshots, rollback, or durable artifact equivalence.

## Matched result

The final gate compared this branch with v10 commit `49d8bb3` using 50
alternating Gentle runs on the same host. Total elapsed time fell from 16.35 s
to 15.15 s; mean time fell from 327 ms to 303 ms and median time from 235 ms
to 220 ms. This is a 7.3% mean and 6.4% median whole-run improvement, including
the complete engine and parity harness rather than an isolated DVI kernel.

Matched 50-iteration symbolized Samply profiles retained 12,753 baseline and
11,741 new samples. The baseline profile includes durable artifact read,
identity verification, owned `validate_artifact`, and `DviStreamWriter::write_page`.
Those frames disappear from the fresh path; its corresponding frames are the
root-child v10 builder, incremental `DviPagePlanBuilder`, verified store, and
`write_page_plan`. Snapshot allocation budgets and all exact Story, Gentle,
TRIP, and e-TRIP DVI comparisons pass.
