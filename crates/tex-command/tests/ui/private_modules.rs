use tex_command::{
    conditionals, input, macro_call, primitives, processor, scan_toks, scanners,
};

fn main() {
    let _ = std::any::type_name::<conditionals::Missing>();
    let _ = std::any::type_name::<input::Missing>();
    let _ = std::any::type_name::<macro_call::Missing>();
    let _ = std::any::type_name::<primitives::Missing>();
    let _ = std::any::type_name::<processor::Missing>();
    let _ = std::any::type_name::<scan_toks::Missing>();
    let _ = std::any::type_name::<scanners::Missing>();
}
