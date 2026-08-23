use tex_command::CommandAttempt;

fn main() {
    let mut attempt = CommandAttempt::<()>::default();
    let _escaped = attempt
        .with_scope(|scope| scope.allocate_token_list([]))
        .expect("scope opens")
        .expect("list allocates");
}
