use tex_command::{CommandFuel, CommandWorkCounters};

fn main() {
    let _ = CommandFuel {
        limit: 1,
        work: CommandWorkCounters::default(),
    };
}
