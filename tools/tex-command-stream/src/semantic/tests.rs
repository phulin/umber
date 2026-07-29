//! Unit tests for the minifixture minimality contract: the byte ceiling, the
//! line ceiling, and the no-format-loading rule. Each rule gets an accept and
//! a reject case so a rule that cannot fail is not actually enforced.

use super::{
    Case, MAX_SOURCE_BYTES, MAX_SOURCE_LINES, input_targets, validate_no_format_loading,
    validate_source_dimensions,
};

fn case_with_inputs(inputs: &[(&str, &str)]) -> Case {
    let inputs_json = inputs
        .iter()
        .map(|(name, content)| format!("{name:?}:{content:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r#"{{
            "id": "probe",
            "property_id": "tex82.probe.case",
            "source": "probe.tex",
            "provenance": {{
                "authority": "tex.web",
                "manifest": "tests/tex82-oracle-manifest.txt",
                "sections": [1]
            }},
            "projection": {{ "kind": "predicate-outcomes" }},
            "expected": ["predicate:probe:-:true"],
            "expectation": {{ "kind": "pass" }},
            "inputs": {{ {inputs_json} }}
        }}"#
    );
    serde_json::from_str(&json).expect("probe case JSON is well-formed")
}

fn case() -> Case {
    case_with_inputs(&[])
}

// --- MAX_SOURCE_BYTES -------------------------------------------------

#[test]
fn source_dimensions_accepts_a_source_within_the_byte_ceiling() {
    assert!(validate_source_dimensions("probe", 1, 1).is_ok());
    assert!(validate_source_dimensions("probe", MAX_SOURCE_BYTES as usize, 1).is_ok());
}

#[test]
fn source_dimensions_rejects_a_source_over_the_byte_ceiling() {
    let error = validate_source_dimensions("probe", MAX_SOURCE_BYTES as usize + 1, 1)
        .expect_err("a source over the byte ceiling must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be 1..={MAX_SOURCE_BYTES} bytes")
    );
}

#[test]
fn source_dimensions_rejects_an_empty_source() {
    let error =
        validate_source_dimensions("probe", 0, 0).expect_err("an empty source must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be 1..={MAX_SOURCE_BYTES} bytes")
    );
}

// --- MAX_SOURCE_LINES ---------------------------------------------------

#[test]
fn source_dimensions_accepts_a_source_within_the_line_ceiling() {
    assert!(validate_source_dimensions("probe", 1, MAX_SOURCE_LINES).is_ok());
}

#[test]
fn source_dimensions_rejects_a_source_over_the_line_ceiling() {
    let error = validate_source_dimensions("probe", 1, MAX_SOURCE_LINES + 1)
        .expect_err("a source over the line ceiling must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be at most {MAX_SOURCE_LINES} lines")
    );
}

// --- no format or package loading ---------------------------------------

#[test]
fn format_loading_accepts_a_self_contained_source() {
    assert!(validate_no_format_loading(&case(), "\\count0=1\\end").is_ok());
}

#[test]
fn format_loading_rejects_plain_tex_reference() {
    let error = validate_no_format_loading(&case(), "\\input plain.tex\\end")
        .expect_err("a reference to plain.tex must be rejected");
    assert_eq!(
        error,
        "case probe source references plain.tex, which loads a format or package"
    );
}

#[test]
fn format_loading_rejects_input_plain() {
    let error = validate_no_format_loading(&case(), "\\input plain\\end")
        .expect_err("\\input plain must be rejected");
    assert_eq!(
        error,
        "case probe source uses \\input plain, which loads a format"
    );
}

/// `\dump` writes a format rather than loading one, so it does not bear on
/// minimality and is not forbidden. `main-control/final-cleanup-end-or-dump`
/// exists to exercise tex.web §1335's rejection of it. What actually stops a
/// fixture assembling a format is the undeclared-`\input` rule, which applies
/// to every case without exception.
#[test]
fn format_loading_permits_dump_which_writes_rather_than_loads_a_format() {
    assert!(validate_no_format_loading(&case(), "\\dump").is_ok());
    assert!(validate_no_format_loading(&case(), "\\count0=1\\dump").is_ok());
}

#[test]
fn format_loading_rejects_undeclared_input_target() {
    let error = validate_no_format_loading(&case(), "\\input nested\\end")
        .expect_err("an \\input target absent from the inputs map must be rejected");
    assert_eq!(
        error,
        "case probe uses \\input \"nested.tex\", which is not declared in this case's inputs map"
    );
}

#[test]
fn format_loading_accepts_input_target_declared_in_inputs_map() {
    let declared = case_with_inputs(&[("nested.tex", "N")]);
    assert!(validate_no_format_loading(&declared, "\\input nested\\end").is_ok());

    let declared = case_with_inputs(&[("child.tex", "C")]);
    assert!(validate_no_format_loading(&declared, "\\input child.tex\\end").is_ok());
}

// --- input_targets: TeX file-name scanning ------------------------------

#[test]
fn input_targets_appends_tex_when_the_name_has_no_extension() {
    assert_eq!(input_targets("\\input nested"), ["nested.tex"]);
}

#[test]
fn input_targets_keeps_an_explicit_extension() {
    assert_eq!(input_targets("\\input child.tex"), ["child.tex"]);
}

#[test]
fn input_targets_skips_a_longer_control_word() {
    let empty: Vec<String> = Vec::new();
    assert_eq!(input_targets("\\inputlineno"), empty);
}

#[test]
fn input_targets_finds_every_occurrence() {
    assert_eq!(input_targets("\\input a\\input b.tex"), ["a.tex", "b.tex"]);
}
