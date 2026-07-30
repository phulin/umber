use std::collections::BTreeSet;
use std::fs;

use test_support::git_fixture::ClosedCase;

const CASES: &[&str] = &[
    "annotations_running",
    "embedded_subset_controls_negative",
    "embedded_subset_omit",
    "embedded_subset_truetype",
    "embedded_subset_type1",
    "embedded_tagged_spacing",
    "embedded_truetype",
    "embedded_type1",
    "external_pdf_page",
    "form_xobjects",
    "minimal_rule",
    "navigation_structures",
    "object_dictionaries",
    "pk_bitmap_300",
    "pk_bitmap_600",
];

#[test]
fn bounded_pdf_minifixtures_are_exact_closed_tracked_directories() {
    let repository = test_support::repository_root();
    let root = repository.join("tests/corpus/pdf");
    let actual = fs::read_dir(&root)
        .expect("read PDF corpus")
        .map(|entry| {
            let entry = entry.expect("read PDF entry");
            assert!(entry.file_type().expect("entry type").is_dir());
            entry.file_name().into_string().expect("UTF-8 case name")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, CASES.iter().map(|case| (*case).to_owned()).collect());
    for case in CASES {
        let closed =
            ClosedCase::discover_tracked_at(&repository, format!("tests/corpus/pdf/{case}"))
                .unwrap_or_else(|error| panic!("pdf/{case} is not closed: {error:#}"));
        closed.read("source.tex").expect("closed source");
        closed.read("expected.ref.pdf").expect("reference PDF");
        closed.read("expected.umber.pdf").expect("Umber PDF");
        if case.starts_with("embedded_") || case.starts_with("pk_bitmap_") {
            closed.read("cmr10.tfm").expect("case-owned TFM");
        }
    }
}
