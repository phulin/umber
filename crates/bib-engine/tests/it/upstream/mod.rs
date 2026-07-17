use super::support::xfail_deep;

#[derive(Debug, PartialEq)]
enum TranslationValue<'a> {
    Expected {
        actual_expression: &'a str,
        expected_expression: &'a str,
    },
    SemanticEnginePending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemanticOwner {
    Graph,
    Names,
    SortAndLists,
    Labels,
    Output,
    Session,
}

impl SemanticOwner {
    const fn issue(self) -> &'static str {
        match self {
            Self::Graph => "umber2-rti9.6",
            Self::Names => "umber2-rti9.7",
            Self::SortAndLists => "umber2-rti9.8",
            Self::Labels => "umber2-rti9.9",
            Self::Output => "umber2-rti9.10",
            Self::Session => "umber2-rti9.12",
        }
    }
}

/// Executes one translated assertion while the semantic facade is pending.
///
/// The exact upstream call and complete source are retained in the owning
/// module so an assertion can be audited now and replaced in isolation later.
#[track_caller]
fn xfail_owned_upstream(
    owner: SemanticOwner,
    assertion: &str,
    actual_expression: &str,
    expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    let owner_issue = owner.issue();
    assert!(
        upstream_source.contains(upstream_call),
        "translated assertion `{assertion}` owned by {owner_issue} is absent from its pinned upstream source"
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

/// Runs assertions whose owning semantic stage exists normally. Other
/// mixed-stage owners retain the strict xfail contract until their stage lands.
#[track_caller]
fn compare_owned_upstream(
    owner: SemanticOwner,
    assertion: &str,
    actual_expression: &str,
    expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    if matches!(
        owner,
        SemanticOwner::Graph | SemanticOwner::Names | SemanticOwner::SortAndLists
    ) {
        pass_upstream(
            assertion,
            actual_expression,
            expected_expression,
            upstream_call,
            upstream_source,
        );
    } else {
        xfail_owned_upstream(
            owner,
            assertion,
            actual_expression,
            expected_expression,
            upstream_call,
            upstream_source,
        );
    }
}

#[track_caller]
fn pass_upstream(
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
    assert!(
        !actual_expression.is_empty(),
        "translated assertion `{assertion}` lost its actual expression"
    );
    assert!(
        !expected_expression.is_empty(),
        "translated assertion `{assertion}` lost its expected expression"
    );
}

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
