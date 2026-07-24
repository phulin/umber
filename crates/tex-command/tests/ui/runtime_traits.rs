use std::hash::Hash;

use serde::{Serialize, de::DeserializeOwned};
use tex_command::CommandRuntime;

fn require_clone<T: Clone>() {}
fn require_deserialize<T: DeserializeOwned>() {}
fn require_eq<T: Eq>() {}
fn require_hash<T: Hash>() {}
fn require_serialize<T: Serialize>() {}

fn main() {
    require_clone::<CommandRuntime>();
    require_deserialize::<CommandRuntime>();
    require_eq::<CommandRuntime>();
    require_hash::<CommandRuntime>();
    require_serialize::<CommandRuntime>();
}
