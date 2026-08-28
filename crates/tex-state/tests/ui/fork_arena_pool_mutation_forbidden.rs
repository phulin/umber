use tex_state::fork_arena::{ChunkPool, ForkArena};

enum PageLane {}

fn main() {
    let mut pool = ChunkPool::<u32>::default();
    let mut arena = ForkArena::<u32, PageLane>::new();
    let list = {
        let mut builder = arena.begin_builder(&mut pool).unwrap();
        builder.push(41).unwrap();
        builder.seal().unwrap()
    };
    let value = arena.list(&pool, list).unwrap().get(0).unwrap();
    let _builder = arena.begin_builder(&mut pool).unwrap();
    println!("{value}");
}
