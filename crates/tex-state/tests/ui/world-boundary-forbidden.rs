use tex_state::{StreamSlot, Universe};

fn main() {
    let mut universe = Universe::new();
    let effect_pos = universe.world().effect_pos();
    universe.world_mut().commit_effects(effect_pos).unwrap();
    let _ = universe.world_mut().store_artifact(b"page").unwrap();
    let tokens = universe.intern_token_list(&[]);
    universe
        .world_mut()
        .record_deferred_write(StreamSlot::new(0), tokens);
}
