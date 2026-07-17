use std::collections::BTreeMap;

use bib_model::{Literal, NameBuilder, NameList};

use super::*;

fn name(family: &str, given: &str) -> bib_model::Name {
    let mut builder = NameBuilder::new();
    builder
        .family(Literal::new(family))
        .given(Literal::new(given));
    builder
        .freeze()
        .expect("test name has family and given parts")
}

#[test]
fn labels_select_first_present_candidate_at_each_boundary() {
    let authors = NameList::new([name("Doe", "Jane")], false);
    let entry = LabelEntry {
        names: BTreeMap::from([("author", &authors)]),
        fields: BTreeMap::from([("date", "2025"), ("title", "Principled labels")]),
    };
    assert_eq!(
        select_labels(
            &entry,
            &["shortauthor", "author"],
            &["eventdate", "date"],
            &["shorttitle", "title"]
        ),
        LabelSelection {
            name_source: Some("author".into()),
            date_source: Some("date".into()),
            title_source: Some("title".into())
        }
    );
}

#[test]
fn labelalpha_respects_name_and_field_widths() {
    let names = NameList::new([name("García", "Ana"), name("Miller", "Bob")], false);
    let entry = LabelEntry {
        names: BTreeMap::new(),
        fields: BTreeMap::from([("year", "2025")]),
    };
    let template = LabelAlphaTemplate(vec![
        LabelAlphaComponent::Names(AlphaNameOptions {
            names: 2,
            name_chars: 1,
            final_name_chars: 2,
            others: "+",
        }),
        LabelAlphaComponent::Field {
            name: "year".into(),
            width: 2,
        },
    ]);
    assert_eq!(template.render(&entry, Some(&names)), "GMi20");
}

#[test]
fn hashes_are_repeatable_and_distinguish_visible_from_full_names() {
    let names = NameList::new([name("Doe", "Jane"), name("Roe", "Richard")], false);
    let first = hash_name_list(&names, 1);
    assert_eq!(first, hash_name_list(&names, 1));
    assert_ne!(first.name_hash, first.full_hash);
    assert_eq!(first.per_name.len(), 2);
}

#[test]
fn extras_follow_list_order_and_reset_between_processing_calls() {
    let fields = [
        ExtraField {
            entry: "b",
            scope: ExtraScope::Date,
            identity: Some("Doe/2025"),
        },
        ExtraField {
            entry: "a",
            scope: ExtraScope::Date,
            identity: Some("Doe/2025"),
        },
        ExtraField {
            entry: "only",
            scope: ExtraScope::Date,
            identity: Some("Roe/2025"),
        },
    ];
    let values = ExtraFieldProcessor::process(&fields);
    assert_eq!(values.get("b", ExtraScope::Date), Some(1));
    assert_eq!(values.get("a", ExtraScope::Date), Some(2));
    assert_eq!(values.get("only", ExtraScope::Date), None);
    assert_eq!(
        ExtraFieldProcessor::process(&fields[..1]).get("b", ExtraScope::Date),
        None
    );
}

#[test]
fn uniqueness_expands_lists_and_records_independent_flags() {
    let entries = [
        UniquenessEntry {
            entry: "a",
            name_hashes: &["doe", "roe"],
            visible_names: 1,
            title: Some("A"),
            work: Some("W1"),
        },
        UniquenessEntry {
            entry: "b",
            name_hashes: &["doe", "smith"],
            visible_names: 1,
            title: Some("B"),
            work: Some("W2"),
        },
    ];
    let state = UniquenessProcessor::process(&entries, UniquenessOptions::default());
    assert_eq!(
        state.names["a"],
        NameDisambiguation {
            visible_names: 2,
            given_name_level: 0
        }
    );
    assert!(state.unique_title.contains("a"));
    assert!(!state.unique_primary_author.contains("a"));
    assert_eq!(
        state,
        UniquenessProcessor::process(&entries, UniquenessOptions::default())
    );
}
