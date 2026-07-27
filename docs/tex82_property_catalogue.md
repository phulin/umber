# TeX82 Property Catalogue

Status: implemented catalogue contract

The TeX82 property catalogue turns the pinned `tex.web` into reviewable,
executable conformance work for `tex-command` and `tex-exec`. Canonical claims
come only from the numbered modules of the source pinned by
`tests/tex82-oracle-manifest.txt`; retired Umber behavior is never evidence.

## Files And Generation

- `tests/tex82-properties/modules.json` is the generated inventory of all
  1,380 WEB modules.
- `tests/tex82-properties/dispositions.json` gives every module the generated
  default `deferred_review` disposition linked to `umber2-johp.218`.
- `tests/tex82-properties/shards/*.json` contains one domain's reviewed module
  overrides and executable properties.
- `scripts/generate-tex82-property-inventory.py` regenerates only the inventory
  and default dispositions from the pinned source.
- `crates/test-support/tests/tex82_catalogue.rs` is the hermetic completeness
  gate in the routine native test tier.

The generator verifies the pinned SHA-256. A WEB module begins at each source
line whose first two characters are `@*` or `@␣`; modules are numbered in
source order. The generated record retains its part, heading, inclusive
source-line bounds, and SHA-256. The source has exactly 1,380 boundaries.

```bash
python3 scripts/generate-tex82-property-inventory.py
```

This reads the local verified source. It never builds, runs, patches, or
rewrites the shared reference oracle.

## Parallel Domain Authoring

A domain auditor edits only one new or existing file under
`tests/tex82-properties/shards/`. The shard owns:

- `domain`, a unique descriptive domain name;
- `module_dispositions`, each containing an inclusive module range, reviewed
  disposition, semantic owner, property IDs, gap bead, and rationale;
- `properties`, the executable claims owned by that domain.

The representative `input-tokenization.json` shard demonstrates claiming and
reclassifying §§343–365 without changing the generated base. Independent
domain branches therefore do not edit the 1,380-entry file or each other's
shards.

The validator sorts shard paths before merging them. The base first classifies
all modules as deferred; exactly one shard may replace that default for a
module. A second shard claim is an error rather than last-writer-wins.
Similarly, each canonical section may belong to only one property, each
property ID to one domain, and the property's semantic owner must equal the
owner of every claimed module. This makes merge results deterministic and
ownership conflicts explicit.

## Dispositions

Reviewed overrides use:

- `property`: executable behavior represented by linked property IDs;
- `definition_only`: a declaration with no independently executable claim;
- `context_only`: explanatory material needed to interpret other properties;
- `out_of_scope`: reviewed behavior outside `tex-command` and `tex-exec`;
- `deferred_review`: deliberately postponed review with a required gap bead.

`deferred_review` is not `out_of_scope`: it avoids asserting that an unread
module is irrelevant. Non-property reviewed dispositions still name the
semantic owner responsible for the classification.

## Property Schema

Each property contains a globally unique ID; exact numeric sections; a
paraphrased claim; non-empty preconditions, stimulus, observations,
postconditions, equivalence cases, and recovery cases; semantic owner and test
level; exact coverage links; and covered/gap status with a bead for gaps.

A covered Rust link is evidence only when its repository-relative path exists,
the named function exists exactly, and its immediately preceding attribute is
`#[test]`. Pinned semantic minifixtures may later provide another test level;
live reference execution never belongs in this gate.

## Completeness Gate

The native test rejects:

- inventory count/order/source-pin drift or an unclassified base module;
- a non-deferred generated base record;
- overlapping shard disposition claims or property section claims;
- duplicate property IDs or conflicting module/property semantic owners;
- invalid citations, missing fields, or inconsistent property links;
- covered properties without exact resolvable test links;
- gap or deferred records without a linked bead.

Focused negative tests exercise overlapping disposition ownership, overlapping
section claims, conflicting property ownership, and an unclassified module.
The gate proves structural completeness and honest bookkeeping; humans remain
responsible for reviewing each canonical paraphrase.
