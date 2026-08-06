# Typesetting Assertion Compaction Ledger

Status: closed for `umber2-vgjr.10.3`

Scope: migration-era aggregate tests in `tex-typeset` that overlap active,
case-level packing, vertical-break, and Appendix G tests. A row authorizes
deletion only when every assertion has an active owner below. Test names are
stable evidence identifiers; source line numbers are deliberately not used.

## Packing

### `tex82_hpack_node_measurement_orders_and_adjustment_migration`

- Exact target width `20`: owned by
  `packing::tests::hpack_sets_finite_stretch_order_and_ratio`, which asserts an
  exact target and additionally pins the ratio.
- Positive finite-order glue selects `Stretching`: owned by
  `packing::tests::hpack_sets_finite_stretch_order_and_ratio`.
- The highest nonzero order is `Fil`: owned by
  `packing::tests::hpack_sets_finite_stretch_order_and_ratio` and the broader
  all-order case `packing::tests::spread_target_and_highest_glue_order`.
- Disposition: delete the aggregate case. Its three assertions are strict
  subsets of the named active owners.

### `tex82_vpack_depth_and_append_to_vlist_baseline_matrix`

- Shifted vertical child contributes `width + shift`: owned by the vertical
  half of `packing::tests::leader_glue_participates_in_packing_like_ordinary_glue`
  for perpendicular width and by `packing::tests::hpack_measures_shifted_child_boxes`
  for signed child shifts.
- Exact vertical target remains the requested height: owned by
  `packing::tests::vpack_records_overfull_badness_when_normal_shrink_is_insufficient`
  and `packing::tests::vtop_preserves_total_size_when_first_box_exceeds_target`.
- Final depth clamps to `box_max_depth`, transferring the excess to height:
  owned with exact operands by `packing::tests::vpack_clamps_depth_to_box_max_depth`.
- Disposition: delete the aggregate case. The active owners separate the
  perpendicular, target, and depth policies and assert strictly more state.

## Math

### `tex82_clean_box_delimiter_and_mu_helper_matrix`

- Empty-field width, height, and depth are zero: owned by
  `math::tests::clean_empty_field_uses_tex82_null_box_without_hpack`, which also
  asserts empty material and absence of a pack observation.
- A clean character uses its font width: owned by
  `math::tests::clean_math_character_observes_both_tex82_hpack_completions`,
  whose observation tuple pins all three measured dimensions twice.
- A source sub-box preserves dimensions: owned by
  `math::tests::direct_sub_box_nucleus_does_not_republish_its_source_pack`.
- A source sub-box preserves display/list kind and glue setting: owned by
  `math::tests::fraction_reuses_single_explicit_numerator_box`, which also pins
  glue sign and order.
- Small-chain delimiter selection chooses `[` at target 25: owned with the
  same delimiter and target by
  `math::tests::var_delimiter_searches_small_chain_before_large_and_builds_extensible`.
- The target-35 extensible delimiter is vertical and has five pieces: owned
  with the same delimiter and target by
  `math::tests::var_delimiter_searches_small_chain_before_large_and_builds_extensible`,
  which additionally pins height and depth.
- Mu glue width/stretch conversion and mu-kern conversion: owned over positive,
  negative, finite-order, and infinite-order boundaries by
  `math::tests::mu_glue_kern_signed_rounding_and_rebox_boundaries`.
- Overline material produces a vertical box: owned structurally by
  `math::tests::nested_under_overline_retains_inner_vertical_box` and
  `math::tests::under_and_overline_rules_retain_running_width_after_packing`.
- Disposition: delete the aggregate case. Every assertion is a subset of a
  named active case; no fixture, diagnostic, event, or byte assertion moves.

## Retained Unique Evidence

`vertical_break::tests::planned::tex82_vert_break_cost_depth_and_tie_matrix`
remains active. Its first-break depth assertion overlaps focused cases, but its
equal-cost artificial-end tie policy has no assertion-complete active owner.
Deleting the whole matrix would weaken evidence, so this issue claims no
vertical-break test reduction.

## Deletion Accounting

The compaction removes two packing aggregate cases and one math aggregate case:
165 authored Rust lines at the issue base after current formatting. It removes
no production scaffolding and claims no deletion for moved code. The ledger is
durable proof for those deletions only; it does not authorize future compaction
without another assertion-by-assertion review.
