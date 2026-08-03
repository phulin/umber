//! Source-free box and horizontal-mode runtime surface.
//!
//! Canonical command control consumes only these typed mutation, material,
//! packing, and migration operations. Expansion-driven box/hmode scanners
//! remain private to the legacy assignment front.

mod vsplit;

pub(crate) use vsplit::split_vbox_register;

pub(crate) use crate::assignments::{
    append_box_node_to_current_list, append_canonical_character_with_fuel,
    append_canonical_control_space_with_fuel, append_canonical_space_with_fuel,
    append_italic_correction_with_fuel, append_whatsit, apply_box_shift_delta, commit_current_list,
    control_space_glue_spec, execute_delete_last, execute_scanned_saved_vertical_discards,
    execute_scanned_unbox, first_box_node, fixed_infinite_glue, flush_pending_hchars,
    flush_pending_hchars_with_fuel, flush_pending_hchars_without_right_boundary,
    hpack_with_overfull_rule, indent_in_hmode, norm_min, split_hpack_migrations, take_last_box,
};
