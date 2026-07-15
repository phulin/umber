use std::path::Path;

use proptest::prelude::*;

use super::{VirtualPath, VirtualPathError};

fn assert_path(actual: Result<VirtualPath, VirtualPathError>, expected: &str) {
    let actual = actual.expect("valid virtual path");
    assert_eq!(actual.as_str(), expected);
    assert_eq!(actual.as_path(), Path::new(expected));
    assert_eq!(actual.to_string(), expected);
    assert_eq!(format!("{actual:?}"), format!("VirtualPath({expected:?})"));
}

fn assert_error(actual: Result<VirtualPath, VirtualPathError>, expected: &'static str) {
    let error = actual.expect_err("invalid virtual path");
    assert_eq!(error.message(), expected);
    assert_eq!(error.to_string(), expected);
    assert_eq!(error, VirtualPathError::new(expected));
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn user_paths_preserve_the_existing_canonical_forms() {
    for (input, expected) in [
        ("main.tex", "/job/main.tex"),
        ("./parts//chapter.tex", "/job/parts/chapter.tex"),
        ("/job/main.tex", "/job/main.tex"),
        ("//job///./parts/./chapter.tex/", "/job/parts/chapter.tex"),
        ("job/main.tex", "/job/job/main.tex"),
        (".hidden", "/job/.hidden"),
        ("résumé/日本語.tex", "/job/résumé/日本語.tex"),
        ("e\u{301}.tex", "/job/e\u{301}.tex"),
        ("🦀.tex", "/job/🦀.tex"),
        ("a b/%23.tex", "/job/a b/%23.tex"),
    ] {
        assert_path(VirtualPath::user(input), expected);
    }
}

#[test]
fn distribution_paths_preserve_the_existing_canonical_forms() {
    for (input, expected) in [
        ("/texlive/plain.tex", "/texlive/plain.tex"),
        (
            "//texlive///tex/./plain//base/plain.tex/",
            "/texlive/tex/plain/base/plain.tex",
        ),
        ("/texlive/.hidden", "/texlive/.hidden"),
        ("/texlive/文書/é.tex", "/texlive/文書/é.tex"),
        ("/texlive/a b/%23.tex", "/texlive/a b/%23.tex"),
    ] {
        assert_path(VirtualPath::distribution(input), expected);
    }
}

#[test]
fn user_path_failures_retain_exact_error_categories_and_messages() {
    for (input, expected) in [
        ("", "path is empty"),
        ("/", "path does not name a file"),
        (".", "path does not name a file"),
        ("././", "path does not name a file"),
        ("/job", "path does not name a file"),
        ("/job///./", "path does not name a file"),
        ("..", "parent traversal is not allowed"),
        ("../secret.tex", "parent traversal is not allowed"),
        ("a/../secret.tex", "parent traversal is not allowed"),
        ("/job/../secret.tex", "parent traversal is not allowed"),
        (
            "/other/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/Job/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/texlive/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "https://example.test/a.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "C:/file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "dir\\file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "bad\0name.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
    ] {
        assert_error(VirtualPath::user(input), expected);
    }
}

#[test]
fn distribution_path_failures_retain_exact_error_categories_and_messages() {
    for (input, expected) in [
        ("", "distribution paths must be absolute under /texlive"),
        (
            "plain.tex",
            "distribution paths must be absolute under /texlive",
        ),
        (
            "texlive/plain.tex",
            "distribution paths must be absolute under /texlive",
        ),
        ("/", "path does not name a file"),
        ("/texlive", "path does not name a file"),
        ("/texlive/./", "path does not name a file"),
        ("/../texlive/a.tex", "parent traversal is not allowed"),
        ("/texlive/a/../b.tex", "parent traversal is not allowed"),
        (
            "/job/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/Texlive/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/texlive/https://example.test/a.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "/texlive/dir\\file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "/texlive/bad\0name.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
    ] {
        assert_error(VirtualPath::distribution(input), expected);
    }
}

#[test]
fn ordering_and_cloning_use_canonical_bytes() {
    let canonical = VirtualPath::user("a//b.tex").expect("path");
    let equivalent = VirtualPath::user("./a/b.tex").expect("path");
    let later = VirtualPath::user("a/c.tex").expect("path");
    assert_eq!(canonical, equivalent);
    assert_eq!(canonical.clone(), canonical);
    assert!(canonical < later);
}

proptest! {
    #[test]
    fn every_accepted_user_path_is_canonical_and_confined(input in any::<String>()) {
        if let Ok(path) = VirtualPath::user(&input) {
            prop_assert!(path.as_str().starts_with("/job/"));
            prop_assert!(!path.as_str().contains("//"));
            prop_assert!(!path.as_str().split('/').any(|part| part == "." || part == ".."));
            prop_assert!(!path.as_str().contains(['\0', '\\', ':']));
            prop_assert_eq!(VirtualPath::user(path.as_str()), Ok(path));
        }
    }

    #[test]
    fn every_accepted_distribution_path_is_canonical_and_confined(input in any::<String>()) {
        if let Ok(path) = VirtualPath::distribution(&input) {
            prop_assert!(path.as_str().starts_with("/texlive/"));
            prop_assert!(!path.as_str().contains("//"));
            prop_assert!(!path.as_str().split('/').any(|part| part == "." || part == ".."));
            prop_assert!(!path.as_str().contains(['\0', '\\', ':']));
            prop_assert_eq!(VirtualPath::distribution(path.as_str()), Ok(path));
        }
    }
}
