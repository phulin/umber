use serde::de::DeserializeOwned;
use serde::Serialize;
use tex_state::ids::{
    ArenaRef, FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, SnapshotId,
    SurvivorRootId, TokenListId,
};
use tex_state::math::MathField;
use tex_state::node::Node;
use tex_state::token::{FrozenToken, Token};

fn require_deserialize<T: DeserializeOwned>() {}
fn require_serialize<T: Serialize>() {}

fn main() {
    require_deserialize::<TokenListId>();
    require_deserialize::<OriginListId>();
    require_deserialize::<MacroDefinitionId>();
    require_deserialize::<GlueId>();
    require_deserialize::<FontId>();
    require_deserialize::<SnapshotId>();
    require_deserialize::<SurvivorRootId>();
    require_deserialize::<ArenaRef>();
    require_deserialize::<NodeListId>();
    require_deserialize::<Node>();
    require_deserialize::<MathField>();

    require_serialize::<TokenListId>();
    require_serialize::<NodeListId>();
    require_serialize::<Node>();

    let _ = TokenListId::new(1);
    let _ = SurvivorRootId::new(1);
    let _ = Token::frozen_end_template();
    let _ = Token::Frozen(FrozenToken(0));
}
