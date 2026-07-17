use super::support::xfail_deep;

#[derive(Debug, PartialEq)]
enum TranslationValue<'a> {
    Expected {
        actual_expression: &'a str,
        expected_expression: &'a str,
    },
    SemanticEnginePending,
}

/// Executes one translated assertion while the semantic facade is pending.
///
/// The exact upstream call and complete source are retained in the owning
/// module so an assertion can be audited now and replaced in isolation later.
#[track_caller]
fn xfail_upstream(
    assertion: &str,
    actual_expression: &str,
    expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    assert!(
        upstream_source.contains(upstream_call),
        "translated assertion `{assertion}` is absent from its pinned upstream source"
    );
    xfail_deep(
        assertion,
        &TranslationValue::Expected {
            actual_expression,
            expected_expression,
        },
        &TranslationValue::SemanticEnginePending,
    );
}

mod annotations;
mod basic_misc;
mod bcfvalidation;
mod biblatexml;
mod bibtex_aliases;
mod bibtex_output;
mod configfile;
mod crossrefs;
mod datalists;
mod dateformats;
mod dm_constraints;
mod encoding;
mod extradate;
mod extratitle;
mod extratitleyear;
mod full_bbl;
mod full_bibtex;
mod full_dot;
mod labelalpha;
mod labelalphaname;
mod labelname;
mod langtags;
mod maps;
mod names;
mod names_x;
mod options;
mod related_entries;
mod remote_files;
mod sections;
mod sections_complex;
mod set_dynamic;
mod set_legacy;
mod set_static;
mod skips;
mod skipsg;
mod sort_case;
mod sort_complex;
mod sort_names;
mod sort_order;
mod sort_uc;
mod sorting;
mod tool;
mod tool_bltxml;
mod tool_bltxml_inout;
mod tool_config;
mod translit;
mod truncation;
mod uniqueness;
mod uniqueness_nameparts;
mod utils;
mod xdata;
