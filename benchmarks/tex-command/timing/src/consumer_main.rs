#[path = "../../src/command_consumer_core.rs"]
mod command_consumer_core;

fn main() {
    command_consumer_core::run(command_consumer_core::ProfileHooks::release());
}
