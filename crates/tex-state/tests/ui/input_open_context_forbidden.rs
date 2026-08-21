use tex_state::InputOpenContext;

fn input_hook_code(ctx: &mut InputOpenContext<'_>) {
    let _ = ctx.world_mut();
    let _ = ctx.meaning(ctx.symbol("relax").unwrap());
    ctx.set_count(0, 1);
}

fn main() {}
