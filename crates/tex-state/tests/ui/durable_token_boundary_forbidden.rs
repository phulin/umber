use tex_state::{
    CommandContext, TokenListId,
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

fn cross_generation_id<G, H>(
    context: &CommandContext<'_, H>,
    id: TokenListId<G>,
) {
    let _ = context.token_list(id);
}

fn main() {}
