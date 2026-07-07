use std::process::Command;

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}
