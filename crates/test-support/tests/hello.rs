use refexec::{RefTex, RunOpts};
use test_support::{assert_matches_fixture, corpus_root, normalize};

#[test]
fn hello_reference_log_matches_fixture() {
    let tex_file = corpus_root().join("hello/hello.tex");
    let output = RefTex::locate()
        .expect("reference TeX should be available")
        .run(&tex_file, &RunOpts::default())
        .expect("reference TeX should run hello fixture");

    assert!(output.success, "reference TeX failed:\n{}", output.stdout);
    assert!(
        output.stdout.contains("hello umber"),
        "reference stdout did not contain hello message:\n{}",
        output.stdout
    );

    assert_matches_fixture("hello", "hello", "log", &normalize::tex_log(&output.log));
}
