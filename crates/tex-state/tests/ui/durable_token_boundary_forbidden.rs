use tex_state::{
    CommandContext, TokenListId, TokenListView,
    token::{Token, TokenWord},
};

fn cross_generation_builder<G, H>(
    source: &mut CommandContext<'_, G>,
    destination: &mut CommandContext<'_, H>,
) {
    let builder = source.begin_token_list_builder().unwrap();
    destination
        .append_token_list_word(&builder, TokenWord::pack(Token::frozen_relax()))
        .unwrap();
}

fn view_outlives_admission<'a, G>(
    context: &'a CommandContext<'a, G>,
    id: TokenListId<G>,
) -> TokenListView<'static, G> {
    context.token_list(id)
}

fn main() {}
