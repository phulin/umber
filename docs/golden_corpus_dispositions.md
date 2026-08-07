# Golden Corpus Dispositions

Status: complete. Every legacy execution golden has an active owner or a
documented complementary integration tier.

The command-semantic corpus is the owner for small TeX82 command behavior. A
case in that tier names one `property_id`, exact WEB sections, an explicit
projection, and every applicable oracle channel in its local `manifest.json`.
The generic runner validates those fields and the closed case inventory. A
legacy golden remains only when its observable belongs to a different test
tier or a named prerequisite prevents an oracle-backed migration.

## Retained Areas

| Area        | Tier and owner                                                                                         | Active evidence                                                                                     | Disposition                                                                                                                                                                                                                                                                                                                                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `align`     | Complete-job DVI integration, owned by `umber`                                                         | `run_align_corpus_matches_committed_dvi`                                                            | Retain all ten primitive-only cases. Their byte-level DVI layout is independent evidence, not a command projection.                                                                                                                                                                                                                                                                                             |
| `math`      | Complete-job DVI integration, owned by `umber`                                                         | `run_math_corpus_matches_committed_dvi` and five focused DVI regressions                            | Retain all nineteen cases. They pin font-backed math layout and final DVI bytes.                                                                                                                                                                                                                                                                                                                                |
| `expand`    | CLI integration, owned by `umber expand-dump`                                                          | `expand_dump_prints_stable_token_format_for_corpus`                                                 | Retain all eight cases. They pin user-visible token spelling, source positions, and CLI framing rather than engine-only expansion semantics.                                                                                                                                                                                                                                                                    |
| `etex_exec` | e-TeX CLI integration, owned by `umber run --etex`                                                     | `run_etex_exec_corpus_matches_committed_diagnostics`                                                | Retain `expansion_virtual_input`: its case-local virtual input and complete-job log pin driver mode, file staging, and e-TeX transcript behavior together.                                                                                                                                                                                                                                                      |
| `exec`      | Manual complete-job diagnostic integration, owned by `umber run`                                       | `run_exec_corpus_matches_committed_diagnostics`                                                     | Retain only `math_component_recovery`, which pins complete-job recovery context while canonical completion is tracked by `umber2-alfh.11`. The duplicate `paragraph_line_shape` case migrated to the property-scoped command-semantic tier.                                                                                                                                                                     |
| `typeset`   | Manual complete-job box/list diagnostic integration, owned by `umber run --show-fixtures`              | `run_typeset_corpus_matches_committed_box_dumps`                                                    | Retain the exact thirteen-case census enforced by `execution_minifixtures_are_closed_tracked_directories`. These cases exercise composed alignment, paragraph, math, mark, adjustment, split, and box diagnostic output. The complementary `paragraph_line_shape` complete list dump and driver suppression of the final effect commit are integration observables, not substitutes for the command projection. |
| `tex_exec`  | Crate-internal preserved reference observations, owned by `tex-exec` and the pdfTeX adapter in `umber` | `tex82_reference_observation_fixtures_match_canonical_execution` and twelve named pdfTeX unit tests | Retain twenty-nine exact source/reference cases: seventeen TeX82 cases execute to bounded clean completion and compare ordered terminal/log projections, while twelve pdfTeX cases retain independent adapter observations. The eight property-runner-only pdfTeX cases moved to the pdfTeX extension tier without changing their unique reference bytes.                                                       |

The execution inventory test pins every retained case name, not just a total,
so adding or removing a legacy case requires reviewing this disposition.
Reference regeneration remains exclusively owned by
`scripts/regen-fixtures.sh`. The `tex_exec` branch is intentionally
validation-only: these observations predate a reproducible pinned capture
contract, and both an ambient pdfTeX run and the standalone pinned 1.40.29
INITEX oracle produce different transcript bytes. The command therefore runs
the active consumers without rewriting `expected.ref`; accepting either live
output would silently replace preserved evidence. New oracle-backed executor
coverage belongs in command-semantic or the pdfTeX extension tier rather than
this historical corpus.

The final legacy execution census is 81 cases: `align` 10, `etex_exec` 1,
`exec` 1, `expand` 8, `math` 19, `tex_exec` 29, and `typeset` 13.

## Retained Executor Case Audit

The active TeX82 runner uses these exact ordered normalized lines in both
terminal and log channels. Each row names the complete retained case set it
owns; the test also executes the fixture source under explicit step and fuel
limits and requires clean `\end` completion, so merely retaining the reference
bytes or reaching an expected prefix cannot pass.

| `tex_exec` case             | Ordered projection                                                       |
| --------------------------- | ------------------------------------------------------------------------ |
| `after`                     | `A B`                                                                    |
| `box_brace_aliases`         | `B:7.0pt`                                                                |
| `box_dimensions`            | `B:12.0pt,3.0pt,2.0pt`                                                   |
| `box_movement`              | `M:void,void`                                                            |
| `box_uncopy_badness`        | underfull-box diagnostic; `B:10000 H:kept V:kept`                        |
| `every_box_hooks`           | underfull-box diagnostic; `H:3,10.0pt;V:2`                               |
| `grouping`                  | `G:0,2`                                                                  |
| `hskip_penalty_recovery`    | missing-number diagnostic; illegal-unit diagnostic; `R:recovered`        |
| `illegal_mag`               | illegal-magnification diagnostic; `> 1.0pt.`                             |
| `incompatible_mag`          | incompatible-magnification diagnostic; `> 0.83333pt.`                    |
| `insert_brace_aliases`      | `I:1,3`                                                                  |
| `internal_dimension_params` | `D:11.0pt,7.0pt`                                                         |
| `last_box`                  | vertical-mode diagnostic; math-mode diagnostic; final `L:` state summary |
| `lccode_selector_recovery`  | bad-character-code diagnostic; `L:3:2`                                   |
| `prefixed_macro`            | `P:7`                                                                    |
| `too_many`                  | too-many-right-braces diagnostic                                         |
| `wrong_close`               | wrong-group-close diagnostic; open-group completion report               |

The twelve actively consumed pdfTeX cases are `pdf_compatibility_controls`,
`pdf_font_codes`, `pdf_font_config`, `pdf_form_diagnostics`, `pdf_form_state`,
`pdf_form_traversal_diagnostics`, `pdf_ignored_dimen_effects`,
`pdf_image_config`, `pdf_metadata_config`, `pdf_microtype_effects`,
`pdf_move_chars_warning`, and `pdf_output_policy`. Their named unit tests in
`crates/umber/src/pdftex.rs` compare pdfTeX-adapter behavior, while the
catalogue gate independently checks each projection against the corresponding
preserved `expected.ref` bytes.

The eight cases activated by `.29` are `pdf_navigation_dest_lifecycle`,
`pdf_navigation_dest_scan`, `pdf_navigation_outline_scan`,
`pdf_navigation_outline_tree`, `pdf_navigation_thread_graph`,
`pdf_navigation_thread_lifecycle`, `pdf_navigation_thread_scan`, and
`pdf_ximage_enquiries`. Their passing and xfail observations are owned by
`tests/pdftex-properties/catalogue.json`; their closed sources and unchanged
unique references now live under `tests/pdftex-properties/fixtures`, and none
is inventory-only evidence. `retained_executor_cases_have_exact_active_ownership`
requires the two cohorts remaining in `tex_exec` to partition that retained
directory exactly.

## Retired Legacy Replacements

The following legacy sources are retired because their active replacements
carry authenticated property ownership and executable evidence. The
replacement fixtures and catalogues, rather than this document, are the
machine-validated semantic authority.

| Retired legacy cases                                                                                                                                                                                                                                           | Active owners                                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `exec/after_tokens`, `case_ignorespaces`, `edef_noexpand_the`, `everypar_state`, `grouping_global`, `long_message`, and `scanner_boundaries`                                                                                                                   | `main-control/afterassignment-aftergroup-order`, `main-control/case-shift-uppercase-lowercase`, `input-expansion/edef-noexpand-the-interaction`, `math/everypar-hook-paragraph-entry`, `main-control/globaldefs-scope-override`, `main-control/message-line-wrap`, and `scanners-internal-quantities/integer-sign-chain-and-units` |
| `exec/diagnostics`                                                                                                                                                                                                                                             | `main-control/messages`, `main-control/show-meaning`, `main-control/show-value`, and `input-expansion/mode-activities`                                                                                                                                                                                                             |
| `exec/paragraph_line_shape`                                                                                                                                                                                                                                    | `line-breaking/paragraph-line-shape`                                                                                                                                                                                                                                                                                               |
| `exec/hbox_text`, `hmode_accent`, `hmode_ligkern`, `hmode_material_primitives`, `hmode_space_factor`, `showbox_limits`, `showbox_simple`, `vbox_baseline_spacing`, and `vbox_nointerlineskip`                                                                  | `math/hbox-text-kern-word`, `math/hbox-accent-kerns`, `math/ligature-kern-word`, `math/hbox-material-primitives`, `math/hbox-space-factor-reset`, `main-control/showbox-breadth-depth-limits`, `math/showbox-basic-box-kinds`, `math/vbox-baselineskip-append`, and `math/vbox-prevdepth-nointerlineskip`                          |
| `typeset/accent_kerns`, `box_dimensions`, `box_group_assignments`, `ligkern_words`, `material_primitives`, `packing_hbox_text`, `showbox_limits`, `space_factor`, and `vbox_baseline_spacing`                                                                  | The same focused math/showbox fixtures above, plus `main-control/auxiliary-state` and `main-control/box-group-save-level`; alignment lifecycle remains independently owned by the retained alignment integration tier and the `alignments` command-semantic domain.                                                                |
| `tex_exec_io/frozen_endwrite` and `leader_payload_effects`                                                                                                                                                                                                     | `page-output/endwrite-stopper-sentinel` and `page-output/leaders-suppress-numbered-stream-effects`                                                                                                                                                                                                                                 |
| `tex_exec_io/ordinary_open_close` and `special_payload`                                                                                                                                                                                                        | `page-output/open-close-effect-observation`, `page-output/count-write-and-text`, and `page-output/special-in-shipped-hbox` already cover the same mechanisms.                                                                                                                                                                      |
| `tex_exec/pdf_navigation_dest_lifecycle`, `pdf_navigation_dest_scan`, `pdf_navigation_outline_scan`, `pdf_navigation_outline_tree`, `pdf_navigation_thread_graph`, `pdf_navigation_thread_lifecycle`, `pdf_navigation_thread_scan`, and `pdf_ximage_enquiries` | The pdfTeX extension tier preserves each source and unique `expected.ref` under `tests/pdftex-properties/fixtures`; the catalogue authenticates source ownership and the active runner compares status, terminal, and log projections with strict xfail fingerprints.                                                              |
| `tex_exec_io/closeout_stream_selectors`, `open_close_without_write`, and `top_open_close`                                                                                                                                                                      | `page-output/closeout-stream-selectors`, `page-output/open-close-without-write`, and `page-output/top-open-write-close` provide oracle-backed all-channel ownership, including generated-artifact bytes.                                                                                                                           |

The obsolete `hello` fixture and its dedicated live-reference generator are
also removed. `main-control/messages` pins the same `\message` mechanism with
an owned TeX82 property and applicable oracle channels, while ordinary CLI
success is covered directly by the CLI integration suite.
