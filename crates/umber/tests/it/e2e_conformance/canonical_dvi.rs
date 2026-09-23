//! Focused canonical DVI comparisons over raw TeX82 jobs.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_ligature_group_boundaries_match_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("canonical-dvi", "ligature_group_boundaries");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected =
        test_support::read_binary_fixture("canonical-dvi", "ligature_group_boundaries", "dvi");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_space_factor_reset_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("canonical-dvi", "rule_space_factor_reset");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected =
        test_support::read_binary_fixture("canonical-dvi", "rule_space_factor_reset", "dvi");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_alignment_leading_tabskip_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "alignment_leading_tabskip");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "alignment_leading_tabskip",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_rule_follows_pending_characters_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "rule_character_order");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "rule_character_order",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_relax_breaks_ligatures_in_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "relax_ligature_boundary");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "relax_ligature_boundary",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_display_equation_number_preserves_formula_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "display_eqnos");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path("math", "display_eqnos", "dvi"))
        .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-side committed fixture loading.
fn canonical_math_group_singleton_ord_matches_reference_dvi() {
    let setup = test_support::dvi::DviCaseSetup::new("math", "mathopen_boxed_delimiter");
    let actual = run_file_in_process_canonical(setup.source_path()).expect("canonical DVI");
    let expected = fs::read(test_support::fixture_path(
        "math",
        "mathopen_boxed_delimiter",
        "dvi",
    ))
    .expect("reference DVI");
    assert_eq!(
        normalized_dvi_for_comparison(&actual).expect("normalize actual"),
        normalized_dvi_for_comparison(&expected).expect("normalize reference")
    );
}
