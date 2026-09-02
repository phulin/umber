use tex_state::fork_arena::{ChunkPool, ForkArena};

enum PageLane {}

fn main() {
    let mut pool = ChunkPool::<u32>::default();
    let mut arena = ForkArena::<u32, PageLane>::new();
    let mut builder = arena.begin_builder(&mut pool).unwrap();
    builder.push(41).unwrap();
    let unique = builder.finish_unique();
    let _first = unique.publish();
    let _second = unique.publish();
}
