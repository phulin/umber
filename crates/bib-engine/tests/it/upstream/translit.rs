// Native Rust translation of upstream t/translit.t at commit 74252e6.

use bib_engine::{BibCommand, FileProvisioner, VfsLimits, VirtualPath};
use bib_unicode::normalise_nfc;

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/translit.bcf");
const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/translit.bib");

fn sorted_keys() -> Vec<String> {
    let mut files = FileProvisioner::new(VfsLimits::default()).unwrap();
    files
        .register_user(VirtualPath::user("translit.bcf").unwrap(), CONTROL.to_vec())
        .unwrap();
    files
        .register_user(VirtualPath::user("translit.bib").unwrap(), DATA.to_vec())
        .unwrap();
    let output = BibCommand::parse(["--noconf", "--nolog", "translit.bcf"])
        .unwrap()
        .execute(&files.snapshot());
    output
        .result()
        .and_then(|result| result.document().sections().next())
        .and_then(|section| section.lists().next())
        .map(|list| {
            list.entries()
                .map(|entry| normalise_nfc(entry.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "xfail: native transliteration sorting does not yet reproduce Biber ordering"]
fn assertion_001_translit_sorting_1() {
    assert_eq!(
        sorted_keys(),
        [
            "aachen",
            "aix-en-provence",
            "arnhem",
            "augsburg",
            "avignon",
            "berlin",
            "utrecht",
            "zeven",
            "kumāra",
            "kha",
            "jīvita",
            "jvara",
            "tyāga",
            "tridaśa",
            "tvid",
            "kṣetra",
            "jñāna",
        ]
    );
}
