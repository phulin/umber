use serde::{Serialize, de::DeserializeOwned};
use tex_command::{CommandProcessor, CurrentCommand};

fn assert_owned_snapshot<T: Serialize + DeserializeOwned>() {}

fn assert_current_command_is_not_serializable<G>() {
    assert_owned_snapshot::<CurrentCommand<G>>();
}

fn assert_processor_is_not_serializable<G: 'static>() {
    assert_owned_snapshot::<CommandProcessor<'static, G>>();
}

fn main() {}
