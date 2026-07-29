# TeX82 Conditional Semantic Minifixtures

This domain manifest owns tiny, hand-authored inputs for TeX82 conditional
properties. The generic `tex-command-stream` integration test discovers the
manifest at run time, validates its property ownership and provenance, drives
each source through `tex_exec::CanonicalMainControl` in the TeX82 INITEX exact
8-bit profile, and compares the declared short semantic observation projection.
It does not invoke TeX, load a format, read the long-document trace registry, or
copy expected bytes from a reference run.

Canonical provenance is the `tex.web` identity pinned by
`tests/tex82-oracle-manifest.txt`. Each case in `manifest.json` names its owning
property, tiny source, exact numbered sections, projection kind, expected
observations, and expectation. The passing cases preserve the established
classification, stack, skipping, branch, predicate, scalar, and token coverage,
including e-TeX's format-loaded `\unless` frame identity and bounded
`\ifdefined` and `\ifcsname` predicate outcomes. The `\ifcsname` case also
proves the non-creating lookup by immediately testing the absent spelling with
`\ifdefined`.

The §505 selector-recovery case is a strict expected failure linked to
`umber2-johp.246`. Its canonical observation list remains the expectation, and
its expectation record pins the first differing index, mismatch kind, expected
value, and actual value. The runner rejects an XPASS and any different failure;
it uses no ignored or panic-expected test.

`tests/corpus/command-semantic/manifest.schema.json` is the version-1 data
schema. New property domains add their own `<domain>/manifest.json` and tiny
sources without editing a Rust case registry or adding another Cargo integration
binary. The runtime validator also rejects unknown manifest fields, duplicate
case identities or source ownership, unsafe or oversized sources, unowned
catalogue properties, malformed provenance, and malformed xfail records.
