//! Source-free box and horizontal-mode runtime surface.
//!
//! Main control consumes only these typed mutation, material,
//! packing, and migration operations.

pub(crate) mod hmode;
mod leaders;
mod material;
mod packaging;
mod vsplit;

pub(crate) use packaging::{
    first_box_node, hpack_owned_with_overfull_rule, hpack_with_overfull_rule, take_last_box,
};
pub(crate) use vsplit::split_vbox_register;

pub(crate) use hmode::{
    append_character_with_fuel, append_control_space_with_fuel, append_italic_correction_with_fuel,
    append_space_with_fuel, append_whatsit, commit_current_list, control_space_glue_spec,
    fixed_infinite_glue, flush_pending_hchars, flush_pending_hchars_with_fuel,
    flush_pending_hchars_without_right_boundary,
};
pub(crate) use leaders::{
    append_leader_contribution, leader_glue_kind, payload_from_node, take_register_payload,
};
pub(crate) use material::{
    append_box_node_to_current_list, apply_box_shift_delta, execute_delete_last,
    execute_scanned_saved_vertical_discards, execute_scanned_unbox_with_error_context,
    split_hpack_migrations,
};

pub(crate) use hmode::{indent_in_hmode, norm_min};

pub(crate) fn append_node_to_current_list<G>(
    nest: &mut crate::ModeNest,
    stores: &mut tex_state::Universe<G>,
    node: tex_state::node::Node,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), crate::ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    crate::vertical::append_node_to_current_list(nest, stores, node)
}
