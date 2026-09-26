use tex_state::fork_arena::{ArenaListId, ChunkPool, ForkArena, RegionValue};

enum PageLane {}

struct Value(u32);

impl RegionValue<PageLane> for Value {
    fn visit_region_lists(&self, _visit: &mut dyn FnMut(ArenaListId<PageLane>)) {}

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}

fn main() {
    let mut pool = ChunkPool::<Value>::default();
    let mut arena = ForkArena::<Value, PageLane>::new();
    let list = {
        let mut builder = arena.begin_builder(&mut pool).unwrap();
        builder.push(Value(41)).unwrap();
        builder.finish()
    };
    let value = arena.list(&pool, list).unwrap().get(0).unwrap();
    let _builder = arena.begin_builder(&mut pool).unwrap();
    println!("{}", value.0);
}
