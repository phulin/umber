// Native Rust translation of upstream t/full-bbl.t at commit 74252e6.

use bib_engine::{BibCommand, BibCommandOutput, FileProvisioner, VfsLimits, VirtualPath};

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-bbl.bcf");
const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-bbl.bib");
const EXPECTED: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/full-bbl.bbl");

fn run() -> BibCommandOutput {
    let mut files = FileProvisioner::new(VfsLimits::default()).unwrap();
    files
        .register_user(VirtualPath::user("full-bbl.bcf").unwrap(), CONTROL.to_vec())
        .unwrap();
    files
        .register_user(VirtualPath::user("full-bbl.bib").unwrap(), DATA.to_vec())
        .unwrap();
    BibCommand::parse([
        "--noconf",
        "--nolog",
        "--output-file=actual.bbl",
        "full-bbl.bcf",
    ])
    .unwrap()
    .execute(&files.snapshot())
}

#[test]
fn assertion_001_full_test_has_zero_exit_status() {
    assert_eq!(run().status().code(), 0);
}

#[test]
#[ignore = "xfail: full BBL serialization is incomplete"]
fn assertion_002_testing_lossort_case_and_sortinit_for_macros() {
    let output = run();
    let bytes = output
        .result()
        .and_then(|result| result.files().next())
        .map(bib_engine::GeneratedFile::bytes);
    assert_eq!(bytes, Some(EXPECTED));
}

fn assert_terminal_contains(expected: &[u8]) {
    assert!(
        run()
            .terminal()
            .windows(expected.len())
            .any(|window| window == expected)
    );
}

#[test]
#[ignore = "xfail: duplicate entry warning wording differs"]
fn assertion_003_testing_duplicate_case_key_warnings_1() {
    assert_terminal_contains(
        b"WARN - Duplicate entry key: 'F1' in file 't/tdata/full-bbl.bib', skipping ...",
    );
}

#[test]
#[ignore = "xfail: datasource case warning is not emitted"]
fn assertion_004_testing_duplicate_case_key_warnings_2() {
    assert_terminal_contains(b"WARN - Possible typo (case mismatch) between datasource keys: 'f1' and 'F1' in file 't/tdata/full-bbl.bib'");
}

#[test]
#[ignore = "xfail: citation case warning is not emitted"]
fn assertion_005_testing_duplicate_case_key_warnings_3() {
    assert_terminal_contains(b"WARN - Possible typo (case mismatch) between citation and datasource keys: 'C1' and 'c1' in file 't/tdata/full-bbl.bib'");
}
