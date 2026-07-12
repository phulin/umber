use tex_exec::transaction::ExecutionTransaction;

fn main() {
    let _cannot_name_transaction = std::mem::size_of::<ExecutionTransaction<'static>>();
}
