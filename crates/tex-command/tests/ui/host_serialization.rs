use serde::{Serialize, de::DeserializeOwned};
use tex_command::{CommandHostCapabilities, CommandHostContext};

fn require_clone<T: Clone>() {}
fn require_deserialize<T: DeserializeOwned>() {}
fn require_serialize<T: Serialize>() {}

fn main() {
    require_clone::<CommandHostCapabilities>();
    require_deserialize::<CommandHostCapabilities>();
    require_serialize::<CommandHostCapabilities>();
    require_clone::<CommandHostContext<'static, ()>>();
    require_deserialize::<CommandHostContext<'static, ()>>();
    require_serialize::<CommandHostContext<'static, ()>>();
}
