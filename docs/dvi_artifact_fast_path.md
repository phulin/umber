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
