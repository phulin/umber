use tex_exec::{EngineBoundary, EngineCheckpoint};

fn main() {
    let _forged = EngineCheckpoint {
        boundary: EngineBoundary::JobStart,
    };
}
