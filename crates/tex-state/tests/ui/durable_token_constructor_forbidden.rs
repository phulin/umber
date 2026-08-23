use tex_state::TokenListBuilder;

fn forge_builder<G>() -> TokenListBuilder<G> {
    TokenListBuilder {
        slot: 0,
        serial: 1,
        _brand: core::marker::PhantomData,
    }
}

fn main() {}
