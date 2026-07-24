use tex_command::{input, processor};

fn main() {
    let _ = std::any::type_name::<input::Missing>();
    let _ = std::any::type_name::<processor::Missing>();
}
