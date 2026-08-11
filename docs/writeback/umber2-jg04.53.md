# umber2-jg04.53: Plain Format Hash-Occupancy Publication

The committed schema-11 Plain image was not a nondeterministic rebuild or an
over-retained semantic closure. Its first differing byte, byte 33, is the
little-endian file-length field at header offset 32. The decisive earlier
header difference is the container ABI fingerprint: the committed
`bcb48ed0c45bd6db55ea24132172cb9eb0cbb901320105098eb090f8a2ef03fb`
image carries container-v3 fingerprint `0x37e2ca8d9c892616`, while current
generation carries container-v4 fingerprint `0x145e2fd8f13d4e8a`.

Commit `40fb016b0` deliberately changed that fingerprint when the formerly
reserved byte in every frozen name record became TeX hash-occupancy state.
TeX82 §§256 and 259 make a multiletter control sequence's occupied hash slot a
permanent allocator coordinate, and §1334 reports that coordinate after a
format load. A current decoder must therefore reject the old image rather than
infer the missing bits. The existing schema-11 ABI contract already requires
exact-fingerprint compatibility, so no loader, comparison, normalization, or
guard change is justified.

The repository-owned Plain publisher verified every input against
`crates/umber-wasm/assets/plain-source.lock`, generated two byte-identical
images, and proved source-initialized and format-loaded DVI equality before
publishing. The renewed image is 128792 bytes with SHA-256
`f61b2a4d979b558dd434f85659e8e9b24283c77bb146b5517af358b77791d0fc`;
an independent `--check` regeneration reproduced it exactly.

The maintained INITEX matrix passed from lock-matching closures. LaTeX retained
SHA-256 `b6b0882044b526393d4c353da01b6eea798383adda3d1fac2e01d192c03d6d6d`
at 1983586 bytes, and pdfLaTeX retained SHA-256
`8fb12d8b9b63f380255437ad5a31446d80ee1e560c05760ca46053b5b3fa3068`
at 2025975 bytes. Exact compatibility TRIP, official two-phase e-TRIP, and
canonical Gentle passed under their unchanged comparators. The exhaustive
command tracer reported zero ordered semantic divergences and zero advisory
geometry differences. Focused hash-occupancy round-trip, format-container,
matrix-routing, official-artifact perturbation, and TRIP caller-boundary
controls passed. Finally, `cargo test -q --tests` passed and `scripts/check.sh`
reported `all 4 gates passed`.
