// Native Rust translation of upstream t/langtags.t at commit 74252e6.

use bib_unicode::LanguageTag;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn tag(
    language: &str,
    region: Option<&str>,
    script: Option<&str>,
    variants: &[&str],
) -> LanguageTag {
    LanguageTag {
        language: Some(language.into()),
        region: region.map(Into::into),
        script: script.map(Into::into),
        variants: strings(variants),
        ..LanguageTag::default()
    }
}

macro_rules! parses_as {
    ($name:ident, $input:literal, $expected:expr) => {
        #[test]
        fn $name() {
            assert_eq!(LanguageTag::parse($input), Ok($expected));
        }
    };
}

parses_as!(assertion_001_bcp47_1, "de", tag("de", None, None, &[]));

parses_as!(assertion_002_bcp47_2, "i-enochian", {
    let mut expected = LanguageTag::default();
    expected.grandfathered = Some("i-enochian".into());
    expected
});

parses_as!(
    assertion_003_bcp47_3,
    "zh-Hant",
    tag("zh", None, Some("Hant"), &[])
);

parses_as!(assertion_004_bcp47_4, "zh-cmn-Hans-CN", {
    let mut expected = tag("zh", Some("CN"), Some("Hans"), &[]);
    expected.extlang = strings(&["cmn"]);
    expected
});

parses_as!(
    assertion_005_bcp47_5,
    "cmn-Hans-CN",
    tag("cmn", Some("CN"), Some("Hans"), &[])
);

parses_as!(
    assertion_006_bcp47_6,
    "yue-HK",
    tag("yue", Some("HK"), None, &[])
);

parses_as!(
    assertion_007_bcp47_7,
    "sl-rozaj",
    tag("sl", None, None, &["rozaj"])
);

parses_as!(
    assertion_008_bcp47_8,
    "sl-rozaj-biske",
    tag("sl", None, None, &["rozaj", "biske"])
);

parses_as!(
    assertion_009_bcp47_9,
    "de-CH-1901",
    tag("de", Some("CH"), None, &["1901"])
);

parses_as!(
    assertion_010_bcp47_10,
    "hy-Latn-IT-arevela",
    tag("hy", Some("IT"), Some("Latn"), &["arevela"])
);

parses_as!(
    assertion_011_bcp47_11,
    "de-DE",
    tag("de", Some("DE"), None, &[])
);

parses_as!(
    assertion_012_bcp47_12,
    "es-419",
    tag("es", Some("419"), None, &[])
);

parses_as!(assertion_013_bcp47_13, "de-CH-x-phonebk", {
    let mut expected = tag("de", Some("CH"), None, &[]);
    expected.private_use = strings(&["phonebk"]);
    expected
});

parses_as!(assertion_014_bcp47_14, "az-Arab-x-AZE-derbend", {
    let mut expected = tag("az", None, Some("Arab"), &[]);
    expected.private_use = strings(&["AZE", "derbend"]);
    expected
});

parses_as!(assertion_015_bcp47_15, "en-US-u-islamcal", {
    let mut expected = tag("en", Some("US"), None, &[]);
    expected.extensions = strings(&["islamcal"]);
    expected
});

parses_as!(assertion_016_bcp47_16, "en-a-myext-b-another", {
    let mut expected = tag("en", None, None, &[]);
    expected.extensions = strings(&["myext", "another"]);
    expected
});

parses_as!(assertion_017_bcp47_17, "zh-CN-a-myext-x-private", {
    let mut expected = tag("zh", Some("CN"), None, &[]);
    expected.extensions = strings(&["myext"]);
    expected.private_use = strings(&["private"]);
    expected
});

#[test]
fn assertion_018_bcp47_18() {
    assert!(LanguageTag::parse("de-419-DE").is_err());
}

#[test]
fn assertion_019_bcp47_19() {
    assert!(LanguageTag::parse("a-DE").is_err());
}
