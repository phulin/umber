use super::*;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn parses_multiple_documents() {
    let manifest = parse_manifest(&format!(
        r#"
# corpus

support plain.tex
url https://example.com/plain.tex
sha256 {HASH}
license Knuth-CTAN
redistributable true
notes exact upstream support file

doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license Knuth-CTAN
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture notes may contain spaces

doc gentle.tex
url http://example.com/gentle.tex
sha256 {HASH}
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes another fixture
"#
    ))
    .expect("manifest should parse");

    assert_eq!(manifest.support.len(), 1);
    assert_eq!(manifest.doc.len(), 2);
    assert_eq!(manifest.doc[0].name, "story.tex");
    assert_eq!(manifest.doc[0].notes, "fixture notes may contain spaces");
    assert_eq!(manifest.doc[1].urls, ["http://example.com/gentle.tex"]);
}

#[test]
fn parses_committed_manifest() {
    let manifest = parse_manifest(include_str!("../../../tests/corpus-manifest.txt"))
        .expect("committed manifest should parse");

    assert!(!manifest.doc.is_empty());
}

#[test]
fn rejects_unknown_field() {
    let error = parse_manifest(&format!(
        r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
bogus value
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect_err("unknown field should fail");

    assert!(error.to_string().contains("unknown manifest field: bogus"));
}

#[test]
fn accepts_ordered_locator_fallbacks() {
    let error = parse_manifest(&format!(
        r#"
support plain.tex
url https://example.com/plain.tex
sha256 {HASH}
license MIT
redistributable true
notes support

doc story.tex
url https://example.com/story.tex
url https://example.com/other.tex
sha256 {HASH}
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect("multiple URL locators should parse");
    assert_eq!(
        error.doc[0].urls,
        [
            "https://example.com/story.tex",
            "https://example.com/other.tex"
        ]
    );
}

#[test]
fn rejects_duplicate_or_unsafe_locators_exactly() {
    for (urls, expected) in [
        (
            "url https://example.com/story.tex\nurl https://example.com/story.tex",
            "line 1: story.tex has duplicate URL: https://example.com/story.tex",
        ),
        (
            "url file:///tmp/story.tex",
            "line 1: story.tex has unsupported URL scheme: file:///tmp/story.tex",
        ),
    ] {
        let text = format!(
            "doc story.tex\n{urls}\nsha256 {HASH}\nlicense MIT\nredistributable true\nformat_source plain.tex\nexpected_ref_dvi_sha256 {HASH}\nnotes fixture\n"
        );
        assert_eq!(
            parse_manifest(&text)
                .expect_err("invalid locator should fail")
                .to_string(),
            expected
        );
    }
}

#[test]
fn rejects_missing_field() {
    let error = parse_manifest(&format!(
        r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect_err("missing field should fail");

    assert!(
        error
            .to_string()
            .contains("missing required field: redistributable")
    );
}

#[test]
fn rejects_bad_hash() {
    let error = parse_manifest(&format!(
        r#"
doc story.tex
url https://example.com/story.tex
sha256 nope
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect_err("bad hash should fail");

    assert!(error.to_string().contains("has invalid sha256"));
}

#[test]
fn rejects_path_traversal_document_name() {
    let error = parse_manifest(&format!(
        r#"
doc ../story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect_err("unsafe file name should fail");

    assert!(
        error
            .to_string()
            .contains("invalid corpus file name: ../story.tex")
    );
}

#[test]
fn rejects_bad_bool() {
    let error = parse_manifest(&format!(
        r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
redistributable yes
format_source plain.tex
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
    ))
    .expect_err("bad bool should fail");

    assert!(
        error
            .to_string()
            .contains("redistributable must be true or false")
    );
}
