# TeX82 Property Catalogue

Status: implemented catalogue contract

The TeX82 property catalogue turns the pinned `tex.web` into reviewable,
executable conformance work for `tex-command` and `tex-exec`. It does not use
retired Umber behavior as evidence. Canonical claims come only from the
numbered modules of the source pinned by `tests/tex82-oracle-manifest.txt`;
coverage claims resolve to committed Rust tests.

## Files

- `tests/tex82-properties/modules.json` is the generated inventory of all
  1,380 WEB modules.
- `tests/tex82-properties/dispositions.json` gives every module exactly one
  disposition.
- `tests/tex82-properties/shards/*.json` contains reviewed executable
  properties grouped by semantic domain.
- `scripts/generate-tex82-property-inventory.py` regenerates the inventory and
  initial dispositions from the pinned source.
- `crates/test-support/tests/tex82_catalogue.rs` is the hermetic completeness
  gate in the routine native test tier.

The generator verifies the pinned SHA-256 before reading the source. A WEB
module begins at each source line whose first two characters are `@*` or
`@␣`. Modules are numbered in source order. The generated record retains its
part, heading, inclusive source-line bounds, and SHA-256. The source contains
exactly 1,380 such boundaries.

Regenerate only after intentionally changing the source pin:

```bash
python3 scripts/generate-tex82-property-inventory.py
```

This command reads the local verified source. It does not build, run, patch,
or rewrite the shared reference oracle.

## Dispositions

Every module has one of these explicit dispositions:

- `property`: reviewed canonical behavior represented by linked property IDs;
- `definition_only`: a declaration with no independently executable claim;
- `context_only`: explanatory material needed to interpret other properties;
- `out_of_scope`: reviewed behavior outside `tex-command` and `tex-exec`;
- `deferred_review`: not yet classified, with a required gap bead.

`deferred_review` is deliberately different from `out_of_scope`: it makes the
remaining audit visible without asserting that an unread module is irrelevant.
The first reviewed shard covers §§343–365. The rest are explicitly deferred
to `umber2-johp.218`.

## Property Schema

Each property contains:

- globally unique `id` and exact numeric `sections`;
- a paraphrased canonical `claim`;
- non-empty `preconditions`, `stimulus`, `expected_observations`, and
  `postconditions`;
- non-empty `equivalence_cases` and `recovery_cases`;
- one `semantic_owner` and `test_level`;
- exact `coverage` records containing repository-relative Rust source paths
  and test function names;
- `status`, either `covered` or `gap`, with `gap_bead` required for a gap.

A covered link is evidence only when the path exists, the named function
exists exactly, and its immediately preceding attribute is `#[test]`.
Properties may later use pinned semantic minifixtures as an additional test
level, but live reference execution never belongs in this gate.

## Completeness Gate

The native test rejects:

- an inventory other than modules 1 through 1,380 in exact order;
- source-pin drift, missing or duplicate dispositions, and unknown
  dispositions;
- invalid section citations, duplicate property IDs, missing semantic owners,
  or missing required property fields;
- property dispositions whose linked property does not cite that module;
- covered properties without resolvable exact test links;
- gap or deferred records without a linked bead.

The gate proves structural completeness and honest coverage bookkeeping. A
reviewed shard remains a human source audit: the validator cannot determine
whether a paraphrase faithfully describes Knuth's prose.
