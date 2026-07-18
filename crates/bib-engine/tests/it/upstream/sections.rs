//! Native translations of upstream `t/sections.t` at commit 74252e6.

use bib_engine::SectionId;

use super::maps::{entry, list_keys, output_text, section_entry_keys, text_field, try_run_fixture};

const EXPECTED_PREAMBLE: &str = r"\preamble{%\n\v{S}tring for Preamble 1%\nString for Preamble 2%\nString for Preamble 3%\nString for Preamble 4%\n}";
const EXPECTED_HEAD: &str = r"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup

\preamble{%
\v{S}tring for Preamble 1%
String for Preamble 2%
String for Preamble 3%
String for Preamble 4%
}

";

#[test]
#[ignore = "xfail: Biber preamble normalization/output is not implemented by bib-engine"]
fn assertion_001_preamble_for_all_sections() {
    let result = try_run_fixture("sections");
    assert!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| output_text(result).contains(EXPECTED_PREAMBLE))
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_002_section_0_macro_test() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "sect1"))
            .and_then(|entry| text_field(entry, "note")),
        Some("value1")
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_003_section_1_macro_test() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 1, "sect4"))
            .and_then(|entry| text_field(entry, "note")),
        Some("value2")
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_004_section_0_citekeys() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 0, "custom/global//global/global/global"))
            .unwrap_or_default(),
        ["sect1", "sect2", "sect3", "sect8"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_005_section_0_shorthands() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 0, "shorthand/global//global/global/global"))
            .unwrap_or_default(),
        ["sect1", "sect2", "sect8"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_006_section_1_citekeys() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 1, "custom/global//global/global/global"))
            .unwrap_or_default(),
        ["sect4", "sect5"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_007_section_1_shorthands() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 1, "shorthand/global//global/global/global"))
            .unwrap_or_default(),
        ["sect4", "sect5"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_008_section_2_citekeys() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 2, "custom/global//global/global/global"))
            .unwrap_or_default(),
        ["sect1", "sect6", "sect7"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_009_section_2_shorthands() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 2, "shorthand/global//global/global/global"))
            .unwrap_or_default(),
        ["sect1", "sect6", "sect7"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_010_section_3_citekeys() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| section_entry_keys(result, 3))
            .unwrap_or_default(),
        ["sect1", "sect2", "sectall1"]
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_011_checking_output_sections_1() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| result.document().section(SectionId::new(0)))
            .map(|section| section.id().to_string())
            .as_deref(),
        Some("0")
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_012_checking_output_sections_2() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| result.document().section(SectionId::new(1)))
            .map(|section| section.id().to_string())
            .as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native multi-section processing currently fails before producing a document"]
fn assertion_013_checking_output_sections_3() {
    let result = try_run_fixture("sections");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| result.document().section(SectionId::new(2)))
            .map(|section| section.id().to_string())
            .as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: Biber safe-character preamble output is not implemented by bib-engine"]
fn assertion_014_preamble_output_check_with_output_safechars() {
    let result = try_run_fixture("sections");
    let output = result
        .as_ref()
        .ok()
        .map(|result| output_text(result))
        .unwrap_or_default();
    let head = output.split("\refsection").next().unwrap_or(output);
    assert_eq!(head, EXPECTED_HEAD);
}
