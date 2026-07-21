# Lightweight PDF test architecture

Status: implemented and dependency-audited architecture for the lightweight PDF
test stack.

This document fixes the replacement oracle mix for the `tex-out` serializer,
the `umber` PDF importer and output driver, shared PDF fixture normalization,
and PDF corpus parity. The target is the same overall assurance, independence,
and coverage as the current suite. Individual assertions may be consolidated,
reshaped, or retired when a stronger oracle covers the same failure.

## Boundaries and oracle independence

No replacement is a second general-purpose PDF implementation.

- **G — typed graph validation:** assert facts on `tex-out`'s detached model or
  the engine ledger before serialization. This is strongest for allocation,
  ownership, references, dictionaries, and policy, but is not independent of
  Umber's producer.
- **F — raw fixture generation:** a dependency-free test helper writes only
  explicit indirect objects, dictionaries, raw or filtered streams, a classic
  xref table, and a trailer. It checks its calculated offsets and `/Length`
  values. Object streams and other complex syntax remain committed,
  externally generated fixtures. This helper is input construction, never an
  output validator.
- **H — Hayro probe:** one bounded host-test abstraction over `hayro-syntax`
  parses independently emitted bytes, retains indirect identities while
  traversing, decodes streams, and projects content operations. It owns depth,
  object, value, and decoded-stream budgets.
- **B — exact bytes:** deterministic Umber bytes, focused byte fragments, and
  stream payloads catch spelling, ordering, compression, and framing drift.
- **N — normalized parity:** the Hayro probe resolves only semantic
  references and compares catalog, ordered pages, resources, selected
  extensions, and decoded operations with committed pdfTeX output. It removes
  object allocation and byte-layout differences only.
- **E — external validation:** a versioned explicit gate runs `qpdf --check`
  (or a reviewed equivalent) over the representative output matrix. Poppler
  raster and text-extraction attestations remain a separate consumer oracle.
  External tools never run in `cargo test --tests`.

The producer (`pdf_writer`), hermetic parser (`hayro-syntax`), structural
validator (`qpdf`), and rendering/extraction consumer (Poppler) are independent
implementations. G and B provide precise local diagnostics; H and N provide a
fast portable semantic gate; E catches shared assumptions and file-level
conformance. No single oracle is treated as sufficient.

The F helper is `test_support::pdf_fixture`. Callers provide explicit object
numbers and raw PDF value syntax through insertion-ordered dictionaries. The
writer owns `/Length`, `/Size`, classic generation-zero xref entries, and
`startxref`, and verifies the resulting offsets and payload spans before
returning bytes. Filtered-stream input is already encoded by the caller; the
helper only declares its filter. Raw object bodies and `nested_array` preserve
focused malformed, cycle, and depth-limit cases without adding a general PDF
value model or encoder.

## Required Hayro boundary

Pinned `hayro-syntax` 0.7.2 already supplies all stable identity and content
access needed except arbitrary trailer entries:

- `XRef::root_id()` identifies `/Root`, and `XRef::get` resolves it.
- `Dict::obj_id()` retains the identity of an indirect dictionary, so
  `Page::raw().obj_id()` is the ordered page object's identity. `Stream::obj_id`
  does the same for streams.
- `Dict::get_raw` and `MaybeRef::as_obj_ref` preserve reference identity;
  `XRef::get` resolves classic xref and type-2 object-stream entries alike.
- `Page::page_stream`, `Page::operations`, and `Stream::decoded` expose decoded
  page, form, and ordinary stream data.

The smallest upstream-compatible addition is:

```rust
impl XRef {
    pub fn trailer(&self) -> Option<Dict<'_>>;
}
```

Hayro should retain the selected trailer dictionary's byte range inside its
owned `PdfData` and parse a `Dict` on demand with the xref reader context. This
avoids a self-referential `XRef`, exposes no mutable state, works for classic
trailers and xref-stream dictionaries, and returns `None` only for Hayro's
root-only repair fallback. The local probe then reads `/Root`, `/Info`, `/ID`,
and selected raw extensions through the ordinary `Dict` API. No public page or
object API change is required.

This boundary is implemented by `test_support::pdf_probe`. The workspace pins
the immutable `phulin/hayro` revision
`abf6c167f6b877a18a077b9ff76dad36573e271d`, based directly on the 0.7.2
release commit; its sole compatibility addition retains the selected trailer
byte range and exposes the accessor above. Once an
equivalent accessor is released upstream, replace the git pin with that release;
the probe itself uses no other fork-specific API.

Each public projection starts a fresh `ProbeLimits` accounting scope. Depth
counts nested arrays, dictionaries, streams, and resolved references; object
counting covers indirect resolutions; value counting covers projected values
and content instructions; and stream bytes count all raw and decoded bytes
materialized by the query. References carry their indirect ID alongside the
resolved target, active cycles become stable back-reference markers, and missing
xref targets remain explicit unresolved-reference markers. Ordered pages also
project inherited boxes, rotation, and resource layers from ancestor to child.

The probe stores both raw and decoded stream bytes, decoded SHA-256, and lenient
untyped operations. A malformed or unsupported filtered stream projects empty
decoded bytes when Hayro cannot decode it; retaining the raw bytes and complete decoded digest
makes that recovery observable without turning the probe into a second strict
validator. Strict syntax acceptance remains the external validator's role.

Hayro's operation iterator is intentionally lenient and does not report a
terminal parse error. The local projection therefore records both canonical
operations and the byte length plus SHA-256 of the complete decoded stream.
The digest makes an unconsumed or malformed suffix observable even if operation
iteration stops early; the external validator remains responsible for syntax
acceptance. This is a local abstraction, not a Hayro fork extension.

## Proof obligations

| Concern              | Required observation                                                                                                                                                                                                                                                                                                  |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Trailer entries      | H reads `/Root`, `/Info`, `/ID`, and selected raw entries from `XRef::trailer`; G checks the typed `PdfTrailer`; B checks deterministic order and omission; E checks final framing.                                                                                                                                   |
| Root identity        | H compares `XRef::root_id()` with the raw trailer `/Root` reference, resolves that exact ID, and requires `/Type /Catalog`; G checks `PdfDocument::catalog()`.                                                                                                                                                        |
| Page object identity | H records `Page::raw().obj_id()` before normalizing the page to its one-based order. Identity-sensitive action, annotation, destination, and bead tests compare raw refs with those IDs; N substitutes `page N` only after that check.                                                                                |
| Indirect cycles      | F emits self and two-object cycles. H follows `MaybeRef` values with an active `ObjectIdentifier` stack, emits a stable back-reference marker on a repeated active ID, and enforces depth/object/value budgets. It neither recursively resolves through `Dict::get` nor drops the edge.                               |
| Object streams       | B requires `/Type /ObjStm` and `/Type /XRef`; H resolves known compressed dictionary IDs through `XRef::get`, records their identities and values, and separately decodes ordinary streams; E validates representative type-2 xrefs. Complex object-stream input is a committed external fixture, not generated by F. |
| Decoded operations   | H uses the untyped iterator to retain exact operands and operators for pages and forms, while also recording decoded length and digest. Focused tests inspect `Tj`, `TJ`, `Do`, graphics, and inline-image operations; N compares the complete projection and E/Poppler independently consume it.                     |

## Completed migration inventory

The following tables record the former call sites and the replacement oracle
used by the completed migration. Imports and manifest entries have been
removed from the workspace and fixture-generator lockfiles.

### `crates/tex-out/src/pdf/serialize/tests.rs`

| Current test                                                                                                   | Replacement                                                                                                                           |
| -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `compact_serialization_is_deterministic_and_independently_parseable`                                           | Keep B determinism/header/EOF; H checks version, page count, root ID, object 4, and content bytes; E covers framing.                  |
| `document_info_is_registered_in_the_pdf_writer_trailer`                                                        | G checks typed trailer and object 6; H checks trailer `/Info` identity and dictionary values.                                         |
| `raw_page_entries_are_hashed_validated_and_serialized_verbatim`                                                | Keep G semantic-hash and B fragment checks; H checks page ID 3 and `/Rotate`.                                                         |
| `deterministic_flate_streams_are_declared_and_decode_exactly`                                                  | Keep B determinism; H checks `/FlateDecode` and decoded bytes.                                                                        |
| `raw_objects_and_trailer_extensions_keep_pdf_writer_framing`                                                   | Keep B object/trailer ordering; G checks the typed trailer; H checks `/Custom` and `/ID`; E replaces parser-acceptance-only coverage. |
| `encoded_streams_preserve_their_filter_and_bytes_under_automatic_compression`                                  | G checks encoded-stream policy; H checks object 6, `/DCTDecode`, and raw encoded payload.                                             |
| `adapter_emits_real_object_streams_for_levels_one_through_three`                                               | Keep B markers/determinism; H resolves compressed object 2 and decodes ordinary stream 4; E checks type-2 xrefs.                      |
| `pdf_writer_object_streams_parse_deterministically_at_levels_one_through_three` (including nested `serialize`) | Keep B determinism; H resolves compressed objects 2 and 3 and ordinary stream 4; E covers the low-level writer fixture.               |

### `crates/test-support/src/pdf.rs`

`normalize_structure` and its helpers become the bounded H/N probe. It retains
the current version, catalog type, ordered pages, media boxes, inherited
resources, beads, selected catalog/trailer/info extensions, stable cycle
markers, user streams, form dictionaries, and decoded operations. It adds the
identity checks and decoded length/digest described above. The normalized
fixture schema must be versioned if those additions change committed text.

### `crates/umber/src/pdf_import/tests.rs`

| Current test/helper                          | Replacement                                                                                                                                  |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `deeply_nested_resource_values_are_rejected` | F writes the deliberately deep classic-xref page. The importer remains the assertion subject; H is not used to pre-validate malformed input. |

`pinned_arxiv_dct_resources_import_as_encoded_streams_when_available` already
uses Hayro and typed imported values; it has no `lopdf` assertion.

### `crates/umber/src/pdf_output.rs` test module

| Current test/helper family                                                                                                                                                                                                 | Replacement                                                                                                                                                                  |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `test_pdf_page`, `test_pdf_page_source`, `test_pdf_page_with_icc_jpeg_source`                                                                                                                                              | F writes the page/group and ICCBased-DCT input PDFs. These are fixture constructors, not validation oracles.                                                                 |
| `raster_png_ximage_is_reused_and_emitted_through_typed_xobjects`, `rgba_png_ximage_uses_a_typed_soft_mask`, `png_gamma_controls_match_the_pinned_pdftex_sample_oracle`, `indexed_png_expands_palette_and_transparency`     | G checks lowering and reuse; H checks image/soft-mask dictionaries and decoded sample bytes.                                                                                 |
| `jpeg_bytes_are_preserved_behind_a_typed_dct_filter`                                                                                                                                                                       | G checks encoded-stream lowering; H checks `/DCTDecode` and exact encoded bytes.                                                                                             |
| `pdf_page_ximage_is_a_reused_typed_form_with_shared_page_group`, `pdf_page_ximage_preserves_icc_based_jpeg_resources`, `pdf_page_group_collision_warning_obeys_signed_suppression`                                         | F supplies inputs; G checks allocation/policy; H checks form/group/resource references, decoded form operations, and preserved JPEG bytes.                                   |
| `annotations_are_page_owned_typed_indirect_objects`                                                                                                                                                                        | G checks allocation and ownership; H records page IDs and requires unique indirect annotation IDs, `/Subtype /Text`, and page-local arrays.                                  |
| `shown_text_operands`, used by `tagged_spacing_uses_explicit_space_and_reanchors_after_disabled_glue` and `fallback_space_font_is_lazy_shared_and_keeps_first_selection_across_pages`                                      | H untyped operations collect `Tj`/`TJ` operands from decoded page streams; G retains font-allocation and state assertions.                                                   |
| `text_page_emits_font_resources_and_pdf_writer_text_operators`                                                                                                                                                             | G checks the typed font graph; H checks font dictionaries, embedded stream identities/data, and decoded text operations.                                                     |
| `subset_map_entry_embeds_only_used_and_included_type1_glyphs`, `committed_woff2_embeds_as_valid_truetype_fontfile2`, `subset_truetype_uses_named_glyph_closure_and_simple_pdf_encoding`                                    | G checks subset selection; H checks descriptors, filters, embedded decoded program lengths/bytes, and encoding dictionaries.                                                 |
| `explicit_glyph_mappings_emit_to_unicode_and_extract_exact_text`                                                                                                                                                           | G checks ToUnicode construction; H checks the CMap stream; the committed Poppler extraction attestation is the independent text oracle.                                      |
| `default_info_dictionary_uses_the_pinned_job_clock`, `info_omission_date_suppression_and_ptex_key_policy_match_pdftex`                                                                                                     | G checks policy and typed values; H uses trailer access for `/Info` identity/absence and checks selected dictionary values; B covers exact deterministic spelling.           |
| `procset_policy_is_captured_at_each_shipout`, `page_parameters_are_consumed_at_pdftex_scopes`, `raw_media_box_overrides_automatic_box_and_pk_mode_freezes`, `zero_page_dimensions_fall_back_to_box_plus_twice_the_origins` | G checks captured policy; H checks ordered page dictionaries, media boxes, resources, and decoded page operations.                                                           |
| `fixed_policy_drives_version_compression_and_decimal_output`, `object_compression_levels_one_through_three_emit_type_two_xrefs`                                                                                            | Keep B version/markers/determinism; H checks pages, compressed object resolution, and ordinary decoded streams; E checks xref/object streams.                                |
| `raw_objects_and_document_fragments_lower_exclusively_through_pdf_writer`                                                                                                                                                  | G checks reserved IDs/fragments; H checks root/page/object identities, catalog/info/names/trailer entries, raw and stream objects, and `/ID`; B checks writer-owned framing. |
| `catalog_openaction_uses_canonical_object_ids_and_pdf_writer_catalog_reference`, `catalog_openaction_serializes_user_and_remote_action_forms`                                                                              | G checks canonical allocation; H compares action and destination refs with root/page IDs and checks action dictionaries.                                                     |
| `referenced_form_uses_typed_pdf_writer_xobject_and_page_resource`                                                                                                                                                          | G checks the typed form graph; H checks resource-to-form identity and decoded form operations.                                                                               |
| `pdf_destinations_emit_typed_arrays_dictionaries_and_six_way_name_tree`, `pdf_outlines_emit_typed_hierarchy_actions_and_indirect_titles`, `running_threads_add_vbox_beads_and_missing_actions_get_fixed_beads`             | G checks typed navigation ownership; H checks identity-preserving destination, outline, thread, and bead edges. N covers the committed composed navigation fixture.          |

Parser-acceptance-only calls in these tests are consolidated into H plus E;
they do not need one parser call per test once the representative matrix covers
the same byte policies.

### `crates/umber/tests/it/pdf_parity.rs`

| Current helper/assertion                                                               | Replacement                                                                                                                                                                                                  |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `annotation_projection` / `annotation_fixture_matches_page_ownership_and_rectangles`   | H records ordered page IDs, annotation IDs, subtype, rectangles, and action subtype; N compares the projection and still rejects cross-page annotation reuse.                                                |
| `check_embedded_font_case`'s `extract_text` assertion                                  | Retire the duplicate in-process extraction. The committed, digest-bound Poppler `pdftotext` output is the independent extraction oracle; H separately checks text operations, font resources, and ToUnicode. |
| `committed_pdftex_fixtures_match_structure_and_bytes` and the remaining parity helpers | N moves from the old normalizer to the Hayro probe; B, raster, extraction, and attestation checks remain unchanged.                                                                                          |

## External validation matrix

The explicit validator gate must include at least:

- classic xref and trailer: `minimal_rule`, `object_dictionaries`;
- xref stream and object stream: a deterministic serializer/object-compression
  artifact at each supported compression policy;
- imported image/PDF: `external_pdf_page` plus raster, alpha, and DCT-focused
  generated outputs;
- fonts/tagging: Type 1, TrueType, PK, subset omission, and
  `embedded_tagged_spacing`;
- graph edges: `annotations_running`, `form_xobjects`, and
  `navigation_structures`.

The implemented command is `scripts/check-pdf-external.sh`. It pins qpdf
12.3.2 and Poppler 25.08.0, fails on any validator warning in `--ci` mode, and
may explicitly skip for a missing tool only in nongating `--local` mode. Its
focused unit-test invocations export temporary xref/object-stream and
raster/alpha/DCT artifacts for qpdf without adding an external process to the
Rust tests. Poppler rendering and extraction remain separate comparisons
against the committed attestations rather than being folded into qpdf
validation. CI and release invocation is
`scripts/check-pdf-external.sh --ci`; local invocation is the same command with
`--local`.

## Delivery order

1. Add F and migrate synthetic inputs.
2. Obtain the one Hayro trailer accessor, build H, and migrate semantic and
   normalized assertions.
3. Migrate remaining output tests to G/B/H and remove `lopdf` manifests and
   lockfile entries.
4. Add the E matrix and document its local and CI invocation.

At every step `cargo test --tests` remains hermetic. The final dependency audit
must show no `lopdf` package or source reference and no change to the normal
`umber-wasm` production dependency graph.

## Final dependency and artifact audit

The completed migration leaves no `lopdf` entry in a workspace manifest,
Rust source file, Cargo metadata, or `Cargo.lock`. Mentions in this document
and `docs/AGENTS.md` are retained only as the historical migration inventory.

The normal `umber-wasm` WebAssembly dependency topology is unchanged from the
pre-migration graph. Its existing `hayro-syntax` 0.7.2 edge now resolves to the
documented compatibility revision above; the revision adds trailer access but
does not add a package edge. The final audit compares
`cargo tree -p umber-wasm --target wasm32-unknown-unknown --edges normal`
before and after the migration with workspace paths and source provenance
normalized.

The dependency-removal audit does not rewrite committed PDF bytes, normalized
structures, raster expectations, or extraction attestations. The only PDF
corpus correction made during the epic is the separately reviewed
`object_dictionaries` fixture repair required by the strict external validator.
The final gates are the focused `tex-out`, `test-support`, and `umber` native
tests, the default native test suite, `scripts/check.sh`,
`scripts/check-wasm.sh`, and `scripts/check-pdf-external.sh --ci` where the
pinned qpdf and Poppler tools are available.
