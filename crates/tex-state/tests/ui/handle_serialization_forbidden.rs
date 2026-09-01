use serde::de::DeserializeOwned;
use serde::Serialize;
use tex_state::node_arena::PageListId;
use tex_state::{DefinitionRef, GlueId, ProvenanceId, TokenListId};

enum Generation {}

fn require_deserialize<T: DeserializeOwned>() {}
fn require_serialize<T: Serialize>() {}

fn main() {
    require_deserialize::<DefinitionRef<Generation>>();
    require_deserialize::<TokenListId<Generation>>();
    require_deserialize::<GlueId<Generation>>();
    require_deserialize::<ProvenanceId<Generation>>();
    require_deserialize::<PageListId>();

    require_serialize::<DefinitionRef<Generation>>();
    require_serialize::<TokenListId<Generation>>();
    require_serialize::<GlueId<Generation>>();
    require_serialize::<ProvenanceId<Generation>>();
    require_serialize::<PageListId>();
}
