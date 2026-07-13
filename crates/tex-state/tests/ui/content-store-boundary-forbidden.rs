use tex_state::glue::{GlueSpec, GlueStore};
use tex_state::node::Node;
use tex_state::node_arena::{NodeArena, NodeListBuilder};
use tex_state::scaled::Scaled;
use tex_state::survivor::SurvivorArena;
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

    let mut nodes = NodeArena::new();
    let survivors = SurvivorArena::new();
    let mut node_builder = NodeListBuilder::new();
    node_builder.push(Node::MathOn(Scaled::from_raw(0)));
    let id = node_builder.finish(&mut nodes);
    let _ = nodes.get(id, &survivors);
}
