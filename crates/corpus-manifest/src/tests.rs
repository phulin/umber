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
    assert_eq!(manifest.doc[1].url, "http://example.com/gentle.tex");
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
fn rejects_duplicate_field() {
    let error = parse_manifest(&format!(
        r#"
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
    .expect_err("duplicate field should fail");

    assert!(error.to_string().contains("duplicate manifest field: url"));
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
