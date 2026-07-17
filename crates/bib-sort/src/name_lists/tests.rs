use bib_model::{Literal, NameBuilder};

use super::*;

#[test]
fn keeps_short_lists_and_truncates_long_lists_to_the_minimum() {
    let names = list(&["A", "B", "C", "D"], false);
    let limits = NameListLimits::new(3, 1).expect("valid limits");
    let visible = NameListVisibility::resolve(&names, limits);

    assert_eq!(visible.visible_count(), 1);
    assert!(visible.more_names());
    assert!(visible.is_truncated());
    assert_eq!(families(visible), ["A"]);

    let short = list(&["A", "B", "C"], false);
    let visible = NameListVisibility::resolve(&short, limits);
    assert_eq!(visible.visible_count(), 3);
    assert!(!visible.more_names());
    assert!(!visible.is_truncated());
}

#[test]
fn explicit_others_preserves_the_marker_and_caps_visibility_at_concrete_names() {
    let limits = NameListLimits::new(99, 3).expect("valid limits");
    let two = list(&["Author", "Secondauthor"], true);
    let one = list(&["Author"], true);

    let two_visible = NameListVisibility::resolve(&two, limits);
    assert_eq!(two_visible.visible_count(), 2);
    assert!(two_visible.more_names());
    assert!(!two_visible.is_truncated());
    assert!(two_visible.to_name_list(true).has_others());
    assert!(!two_visible.to_name_list(false).has_others());

    let one_visible = NameListVisibility::resolve(&one, limits);
    assert_eq!(one_visible.visible_count(), 1);
    assert!(one_visible.more_names());
}

#[test]
fn resolves_cite_bibliography_and_alpha_limits_independently() {
    let names = list(&["A", "B", "C", "D", "E"], false);
    let visibility = NameVisibility::resolve(
        &names,
        NameVisibilityOptions {
            cite: NameListLimits::new(3, 1).expect("cite limits"),
            bibliography: NameListLimits::new(2, 2).expect("bibliography limits"),
            alpha: NameListLimits::new(3, 2).expect("alpha limits"),
        },
    );

    assert_eq!(visibility.cite().visible_count(), 1);
    assert_eq!(visibility.bibliography().visible_count(), 2);
    assert_eq!(visibility.alpha().visible_count(), 2);
}

#[test]
fn rejects_invalid_limits_with_stable_diagnostics() {
    let cases = [
        ((0, 1), "maximum visible names must be at least one"),
        ((1, 0), "minimum visible names must be at least one"),
        ((2, 3), "minimum visible names (3) exceeds maximum (2)"),
    ];
    for ((maximum, minimum), expected) in cases {
        let error = NameListLimits::new(maximum, minimum).expect_err("invalid limits");
        assert_eq!(error.to_string(), expected);
    }
}

fn list(families: &[&str], has_others: bool) -> NameList {
    NameList::new(
        families.iter().map(|family| {
            let mut builder = NameBuilder::new();
            builder.family(Literal::new(*family));
            builder.freeze().expect("family name")
        }),
        has_others,
    )
}

fn families(visibility: NameListVisibility<'_>) -> Vec<&str> {
    visibility
        .iter()
        .map(|name| name.family().expect("family").value().as_str())
        .collect()
}
