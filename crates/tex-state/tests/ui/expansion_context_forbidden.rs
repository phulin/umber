use tex_state::token::Catcode;
use tex_state::{ExpansionContext, ExpansionState, Universe};

fn lower_crate_expansion_code(ctx: &mut ExpansionContext<'_>) {
    let _ = ctx.world_mut();
    let snapshot = ctx.snapshot();
    ctx.rollback(&snapshot);
    ctx.set_count(0, 1);
    ctx.set_catcode('@', Catcode::Letter);
    let font = ctx.current_font();
    ctx.set_current_font(font);
}

fn main() {
    let mut universe = Universe::new();
    let mut ctx = ExpansionContext::new(&mut universe);
    lower_crate_expansion_code(&mut ctx);
}
