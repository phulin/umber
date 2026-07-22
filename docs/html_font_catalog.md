# HTML MVP font catalog inventory

Status: implemented inventory for contract version 1 in
[cross_output_fonts.md](cross_output_fonts.md).

The hosted HTML MVP supports exactly one legacy TFM mapping and two explicit
OpenType selections. This is a publication allow-list, not a limit on local or
client-managed DVI/PDF font resolution.

| Selection                  | Exact input identity                                                                                             | Decoded program identity                                           | License                                                                                    |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| Legacy `cmr10` text        | `cmr10.tfm`, 1,296 bytes, SHA-256 `87f2d8981927644cbecaf3d639e96e348ea4e7be49d8804468bd8ba9ff3f5244`             | Selects `cmu-serif-roman` below through mapping schema 1           | Inherits the selected CMU record                                                           |
| `opentype:cmu-serif-roman` | CMU Serif Roman WOFF2, 222,840 bytes, SHA-256 `1b875e541dc5c517cd11d244710d8639addbe91a0bb1ba55e7c4593225c7a970` | `7f8f29c0b55f41195c211242ce71837776e485828423324e74bbc7f425ad78a4` | OFL-1.1, object SHA-256 `73273dffdefe2e5f1e138084d4a4b65b1c50df2ab0179f78484f31beefe30d84` |
| `opentype:stix-two-math`   | STIX Two Math WOFF2, 558,764 bytes, SHA-256 `cb1149b7c8b7b194eff7f42e20cf9e7a9706d342ffc2b14765624577d8be38e3`   | `b48adabb239892c9c246da05fb77e6b9525ca18b3ed613116cbca45bb35213c5` | OFL-1.1, object SHA-256 `0c8825913b60d858aacdb33c4ca6660a7d64b0d6464702efbb19313f5765861a` |

The canonical machine inventory is
`tools/texlive-wasm-publish/catalog/html-mvp-v1.json`. It contains no runtime
files, exactly two font records, one legacy mapping, two unique license
objects, and two unique WOFF2 objects. The `cmr10` mapping contains exactly 256
slots: codes 0–127 have reviewed OT1 text projections, including multi-scalar
ligatures, and codes 128–255 are intentionally unmapped. A used unmapped code
therefore follows the typed missing-mapping path; the catalog does not guess a
Unicode scalar or substitute another family.

The CMU object is referenced by both the legacy mapping and explicit text
selection, so HTML publication retains and emits one content-addressed WOFF2
object. The STIX object is available only by explicit OpenType selection and
has a validated MATH table. Both records affirm embedding and redistribution,
pin their complete OFL text, source version, conversion receipt, object digest,
decoded program identity, feature-policy version, and request key.

## Reproduction and audit

From the repository root, regenerate the canonical catalog with:

```bash
cargo run -q --manifest-path tools/texlive-wasm-publish/Cargo.toml -- \
  --write-html-mvp-catalog umber-html-mvp-v1 \
  tools/texlive-wasm-publish/catalog/html-mvp-v1.json \
  crates/tex-fonts/tests/fixtures/cm/cmr10.tfm \
  crates/umber-wasm/assets/cmu-serif-500-roman.woff2 \
  crates/umber-wasm/assets/CMU-OFL.txt \
  crates/tex-fonts/tests/fixtures/stix-two-math.woff2 \
  crates/tex-fonts/tests/fixtures/stix-two-math.LICENSE.txt
```

The generator rejects any byte or length drift in those five inputs and emits
canonical shard JSON. Its test requires byte-for-byte equality with the
committed inventory. The `umber` catalog audit independently decodes the WOFF2
programs and verifies program identities, cmap totality for every mapped slot,
MATH availability, object reuse, license digests, and affirmative permissions.

Additional Computer Modern faces or sizes, other legacy encodings, virtual
fonts, Type 1/PK conversion, OS-font discovery, and automatic SFNT conversion
are unsupported by this MVP catalog. They require additive records or a later
versioned policy; they must not change the meaning of this inventory.

Compatibility tests exercise those additive seams with synthetic records only:
another exact TFM/encoding mapping, an advanced instance, and another explicit
family all preserve the MVP request, record, object, program, and license
identities while sharing the existing synthetic transport object. The
committed production inventory above remains exactly three selections. HTML VF
requests remain the typed `UnsupportedHtmlVirtualFont` outcome; the future
closure and positioned-leaf insertion point is tracked by `umber2-nobk.12` and
specified in section 8 of the cross-output contract.
