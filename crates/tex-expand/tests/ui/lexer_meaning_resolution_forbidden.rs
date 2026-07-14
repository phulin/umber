use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;

fn main() {
    let mut stores = Universe::new();
    let symbol = stores.intern("forbidden");
    let mut input = InputStack::new(MemoryInput::new(""));
    let _ = input.resolve_expansion_meaning(&stores, symbol.symbol());
}
