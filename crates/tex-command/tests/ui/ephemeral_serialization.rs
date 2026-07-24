use serde::{Serialize, de::DeserializeOwned};
use tex_command::{CommandProcessor, CurrentCommand};

fn assert_owned_snapshot<T: Serialize + DeserializeOwned>() {}

fn main() {
    assert_owned_snapshot::<CurrentCommand>();
    assert_owned_snapshot::<CommandProcessor<'static>>();
}
