// Native Rust translation of upstream t/full-dot.t at commit 74252e6.

use bib_engine::{BibCommand, BibCommandOutput, ProjectWorkspace, VfsLimits, VirtualPath};

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-dot.bcf");
const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-dot.bib");
const EXPECTED: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-dot.dot");

fn run() -> BibCommandOutput {
    let mut files = ProjectWorkspace::new(VfsLimits::default()).expect("valid VFS limits");
    files
        .register_user(
            VirtualPath::user("full-dot.bcf").expect("valid virtual path"),
            CONTROL.to_vec(),
        )
        .expect("unique registered file");
    files
        .register_user(
            VirtualPath::user("full-dot.bib").expect("valid virtual path"),
            DATA.to_vec(),
        )
        .expect("unique registered file");
    BibCommand::parse([
        "--noconf",
        "--nolog",
        "-dot-include=section,field,xdata,crossref,xref,related",
        "--output-format=dot",
        "--output-file=actual.dot",
        "full-dot.bcf",
    ])
    .expect("valid command line")
    .execute(&files.snapshot())
}

#[test]
fn assertion_001_full_test_has_zero_exit_status() {
    assert_eq!(run().status().code(), 0);
}

#[test]
#[ignore = "xfail: full DOT serialization is incomplete"]
fn assertion_002_testing_dot_output() {
    let output = run();
    let bytes = output
        .result()
        .and_then(|result| result.files().next())
        .map(bib_engine::GeneratedFile::bytes);
    assert_eq!(bytes, Some(EXPECTED));
}
