// Native Rust translation of upstream t/bibtex-aliases.t at commit 74252e6.

use bib_engine::{
    BibCommand, BibCommandOutput, Entry, EntryId, FieldId, FieldValue, FileProvisioner, VfsLimits,
    VirtualPath,
};

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/bibtex-aliases.bcf");
const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/bibtex-aliases.bib");
const WARNINGS_ALIAS2: &[&str] = &[
    "Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Field 'school' invalid in data model - ignoring",
    "Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Invalid entry type 'thing' - defaulting to 'misc'",
    "Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Invalid field 'institution' for entrytype 'misc'",
];
const WARNINGS_ALIAS4: &[&str] = &[
    "Datamodel: customa entry 'alias4' (bibtex-aliases.bib): Invalid field 'author' for entrytype 'customa'",
    "Datamodel: customa entry 'alias4' (bibtex-aliases.bib): Invalid field 'title' for entrytype 'customa'",
];

fn run() -> BibCommandOutput {
    let mut files = FileProvisioner::new(VfsLimits::default()).unwrap();
    files
        .register_user(
            VirtualPath::user("bibtex-aliases.bcf").unwrap(),
            CONTROL.to_vec(),
        )
        .unwrap();
    files
        .register_user(
            VirtualPath::user("bibtex-aliases.bib").unwrap(),
            DATA.to_vec(),
        )
        .unwrap();
    BibCommand::parse(["--noconf", "--nolog", "bibtex-aliases.bcf"])
        .unwrap()
        .execute(&files.snapshot())
}

fn entry(id: &str) -> Entry {
    run()
        .result()
        .and_then(|result| result.document().sections().next())
        .and_then(|section| section.entry(&EntryId::new(id).unwrap()))
        .cloned()
        .unwrap_or_else(|| panic!("missing entry {id}"))
}

fn string_field(entry_id: &str, field: &str) -> Option<String> {
    match entry(entry_id).fields().get(&FieldId::new(field).unwrap()) {
        Some(FieldValue::Literal(value)) => Some(value.as_str().to_owned()),
        Some(FieldValue::Verbatim(value)) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

fn list_field(entry_id: &str, field: &str) -> Option<Vec<String>> {
    match entry(entry_id).fields().get(&FieldId::new(field).unwrap()) {
        Some(FieldValue::LiteralList(values)) => Some(
            values
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
        ),
        _ => None,
    }
}

fn warnings(entry_id: &str) -> Vec<String> {
    let id = EntryId::new(entry_id).unwrap();
    run()
        .result()
        .map(|result| {
            result
                .diagnostics()
                .filter(|diagnostic| diagnostic.entry() == Some(&id))
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

macro_rules! test_eq {
    ($name:ident, $actual:expr, $expected:expr) => {
        #[test]
        #[ignore = "xfail: native alias mapping and validation do not yet reproduce Biber"]
        fn $name() {
            assert_eq!($actual, $expected);
        }
    };
}

test_eq!(
    assertion_001_alias_1,
    entry("alias1").entry_type().as_str(),
    "thesis"
);
test_eq!(
    assertion_002_alias_2,
    string_field("alias1", "type").as_deref(),
    Some("phdthesis")
);
test_eq!(
    assertion_003_alias_3,
    list_field("alias1", "location"),
    Some(vec!["Ivory Towers".to_owned()])
);
test_eq!(
    assertion_004_alias_4,
    string_field("alias1", "address"),
    None
);
test_eq!(
    assertion_005_alias_5,
    entry("alias2").entry_type().as_str(),
    "misc"
);
test_eq!(assertion_006_alias_6, warnings("alias2"), WARNINGS_ALIAS2);
test_eq!(
    assertion_007_alias_7,
    string_field("alias2", "school"),
    None
);
test_eq!(
    assertion_008_alias_8,
    entry("alias3").entry_type().as_str(),
    "customb"
);
test_eq!(
    assertion_009_alias_9,
    entry("alias4").entry_type().as_str(),
    "customa"
);
test_eq!(
    assertion_010_alias_10,
    string_field("alias4", "verba").as_deref(),
    Some("conversation")
);
test_eq!(
    assertion_011_alias_11,
    string_field("alias4", "verbb").as_deref(),
    Some("somevalue")
);
test_eq!(
    assertion_012_alias_12,
    string_field("alias4", "eprint").as_deref(),
    Some("anid")
);
test_eq!(
    assertion_013_alias_13,
    string_field("alias4", "eprinttype").as_deref(),
    Some("pubmedid")
);
test_eq!(
    assertion_014_alias_14,
    string_field("alias4", "userd").as_deref(),
    Some("Some string of things")
);
test_eq!(
    assertion_015_alias_15,
    string_field("alias4", "pubmedid"),
    None
);

#[test]
#[ignore = "xfail: native name alias mapping does not yet expose namea"]
fn assertion_016_alias_16() {
    let entry = entry("alias4");
    let given = match entry.fields().get(&FieldId::new("namea").unwrap()) {
        Some(FieldValue::NameList(names)) => names
            .iter()
            .next()
            .and_then(|name| name.given())
            .map(|part| part.value().as_str()),
        _ => None,
    };
    assert_eq!(given, Some("Sam"));
}

test_eq!(assertion_017_alias_17, warnings("alias4"), WARNINGS_ALIAS4);
test_eq!(
    assertion_018_alias_18,
    string_field("alias5", "abstract"),
    None
);
test_eq!(
    assertion_019_alias_19,
    list_field("alias5", "listb").map(|values| values.join("!")),
    Some("REPlaCEDte!early".to_owned())
);
test_eq!(
    assertion_020_alias_20,
    list_field("alias5", "institution").map(|values| values.join("!")),
    Some("REPlaCEDte!early".to_owned())
);
test_eq!(
    assertion_021_alias_21,
    list_field("alias6", "keywords"),
    Some(vec!["keyw1".to_owned(), "keyw2".to_owned()])
);
test_eq!(
    assertion_022_alias_22,
    list_field("alias7", "lista"),
    Some(vec!["listaval".to_owned()])
);
test_eq!(
    assertion_023_alias_23,
    string_field("alias7", "verbb").as_deref(),
    Some("val2val1")
);
test_eq!(
    assertion_024_alias_24,
    string_field("alias7", "verbc").as_deref(),
    Some("val3val2val1")
);
#[test]
fn assertion_025_alias_25() {
    assert_eq!(string_field("alias8", "verbc"), None);
}
