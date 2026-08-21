use tex_state::World;

fn main() {
    let mut world = World::memory();
    world.commit_effects(todo!()).unwrap();
    world.record_deferred_write(todo!(), todo!());
    world.rollback_generation_fork(todo!());
}
