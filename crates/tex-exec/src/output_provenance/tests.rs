use super::provenance_recipe_for_origins;

#[test]
fn detached_recipe_budget_rejects_the_complete_optional_recipe() {
    let zero_budget = tex_state::Universe::new().with_provenance_config(
        tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
        tex_state::ProvenanceBudgets {
            detached_artifact_recipe_bytes: 0,
            ..tex_state::ProvenanceBudgets::default()
        },
    );
    assert!(
        provenance_recipe_for_origins(&zero_budget, [tex_state::token::OriginId::UNKNOWN])
            .is_none()
    );

    let one_slot = tex_state::Universe::new().with_provenance_config(
        tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
        tex_state::ProvenanceBudgets {
            detached_artifact_recipe_bytes: std::mem::size_of::<u32>(),
            ..tex_state::ProvenanceBudgets::default()
        },
    );
    let recipe = provenance_recipe_for_origins(&one_slot, [tex_state::token::OriginId::UNKNOWN])
        .expect("one unknown slot fits its exact charge");
    assert_eq!(recipe.origin_slots.as_ref(), &[u32::MAX]);
}
