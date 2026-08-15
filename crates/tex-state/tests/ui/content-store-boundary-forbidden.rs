use tex_state::glue::{GlueSpec, GlueStore};
use tex_state::node::Node;
use tex_state::node_arena::NodeListBuilder;
use tex_state::scaled::Scaled;
use tex_state::token::Token;
use tex_state::token_store::{TokenListBuilder, TokenStore};

fn main() {
    let mut tokens = TokenStore::new();
    let _ = tokens.intern(&[Token::param(1)]);
    let mut token_builder = TokenListBuilder::new();
    let _ = token_builder.finish(&mut tokens);
    let _ = tokens.get(TokenStore::empty_id());

    let mut glue = GlueStore::new();
    let zero = glue.intern(GlueSpec::ZERO);
    let _ = glue.get(zero);

    let mut node_builder = NodeListBuilder::new();
    node_builder.push(Node::MathOn(Scaled::from_raw(0)));
    let _ = node_builder.finish();
}
