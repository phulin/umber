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
mod configfile;
mod crossrefs;
mod dateformats;
mod dm_constraints;
mod encoding;
mod langtags;
mod maps;
mod options;
mod related_entries;
mod remote_files;
mod sections;
mod sections_complex;
mod set_dynamic;
mod set_legacy;
mod set_static;
mod translit;
mod utils;
mod xdata;
