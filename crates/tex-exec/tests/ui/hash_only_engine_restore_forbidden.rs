use tex_exec::{Executor, HashOnlyObservation};
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;

fn restore_hash_only(
    executor: &mut Executor,
    input: &mut InputStack<MemoryInput>,
    stores: &mut Universe,
    observation: &HashOnlyObservation,
) {
    let _ = executor.restore_checkpoint(input, stores, observation, |_, _, _| {
        Ok::<_, ()>(MemoryInput::new(""))
    });
}

fn main() {}
