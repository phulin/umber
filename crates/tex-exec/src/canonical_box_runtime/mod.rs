//! Source-free box and horizontal-mode runtime surface.
//!
//! Canonical command control consumes only these typed mutation, material,
//! packing, and migration operations. Expansion-driven box/hmode scanners
//! remain private to the legacy assignment front.

pub(crate) mod hmode;
mod material;
mod packaging;
mod vsplit;

#[cfg(test)]
pub(crate) use packaging::project_short_diagnostic_discs;
pub(crate) use packaging::{
    first_box_node, hpack_owned_with_overfull_rule, hpack_with_overfull_rule,
};
pub(crate) use vsplit::split_vbox_register;

pub(crate) use hmode::{
    append_canonical_character_with_fuel, append_canonical_control_space_with_fuel,
    append_canonical_space_with_fuel, append_italic_correction_with_fuel, append_whatsit,
    commit_current_list, control_space_glue_spec, fixed_infinite_glue, flush_pending_hchars,
    flush_pending_hchars_with_fuel, flush_pending_hchars_without_right_boundary,
};
pub(crate) use material::{
    append_box_node_to_current_list, append_box_register, apply_box_shift_delta,
    execute_delete_last, execute_scanned_saved_vertical_discards, execute_scanned_unbox,
    split_hpack_migrations,
};

pub(crate) use crate::assignments::{indent_in_hmode, norm_min, take_last_box};
