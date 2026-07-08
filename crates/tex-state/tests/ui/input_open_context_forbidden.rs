use std::path::Path;

use tex_state::{InputOpenContext, InputReadState, Universe};

fn input_hook_code(ctx: &mut InputOpenContext<'_>) {
    let _ = ctx.read_input_file(Path::new("main.tex"));
    let _ = ctx.world_mut();
    let _ = ctx.meaning(ctx.symbol("relax").unwrap());
    ctx.set_count(0, 1);
}

fn main() {
    let mut universe = Universe::new();
    let mut ctx = InputOpenContext::new(&mut universe);
    input_hook_code(&mut ctx);
}
