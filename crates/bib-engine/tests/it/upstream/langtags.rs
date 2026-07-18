// Direct translation of upstream t/langtags.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use bib_unicode::LanguageTag;

#[track_caller]
fn pass_upstream(
    assertion: &str,
    actual_expression: &str,
    _expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    assert!(upstream_source.contains(upstream_call), "{assertion}");
    let marker = "parse('";
    let start = actual_expression.find(marker).expect("parse expression") + marker.len();
    let end = actual_expression[start..]
        .find("')")
        .expect("tag terminator")
        + start;
    let actual = LanguageTag::parse(&actual_expression[start..end]);
    if matches!(assertion, "BCP47 - 18" | "BCP47 - 19") {
        assert!(actual.is_err(), "{assertion}");
        return;
    }
    let mut expected = LanguageTag::default();
    match assertion {
        "BCP47 - 1" => expected.language = Some("de".into()),
        "BCP47 - 2" => expected.grandfathered = Some("i-enochian".into()),
        "BCP47 - 3" => set(&mut expected, "zh", None, Some("Hant"), None),
        "BCP47 - 4" => {
            set(&mut expected, "zh", Some("CN"), Some("Hans"), None);
            expected.extlang = strings(&["cmn"]);
        }
        "BCP47 - 5" => set(&mut expected, "cmn", Some("CN"), Some("Hans"), None),
        "BCP47 - 6" => set(&mut expected, "yue", Some("HK"), None, None),
        "BCP47 - 7" => set(&mut expected, "sl", None, None, Some(&["rozaj"])),
        "BCP47 - 8" => set(&mut expected, "sl", None, None, Some(&["rozaj", "biske"])),
        "BCP47 - 9" => set(&mut expected, "de", Some("CH"), None, Some(&["1901"])),
        "BCP47 - 10" => set(
            &mut expected,
            "hy",
            Some("IT"),
            Some("Latn"),
            Some(&["arevela"]),
        ),
        "BCP47 - 11" => set(&mut expected, "de", Some("DE"), None, None),
        "BCP47 - 12" => set(&mut expected, "es", Some("419"), None, None),
        "BCP47 - 13" => {
            set(&mut expected, "de", Some("CH"), None, None);
            expected.private_use = strings(&["phonebk"]);
        }
        "BCP47 - 14" => {
            set(&mut expected, "az", None, Some("Arab"), None);
            expected.private_use = strings(&["AZE", "derbend"]);
        }
        "BCP47 - 15" => {
            set(&mut expected, "en", Some("US"), None, None);
            expected.extensions = strings(&["islamcal"]);
        }
        "BCP47 - 16" => {
            set(&mut expected, "en", None, None, None);
            expected.extensions = strings(&["myext", "another"]);
        }
        "BCP47 - 17" => {
            set(&mut expected, "zh", Some("CN"), None, None);
            expected.extensions = strings(&["myext"]);
            expected.private_use = strings(&["private"]);
        }
        _ => panic!("unhandled upstream assertion {assertion}"),
    }
    assert_eq!(actual.expect(assertion), expected, "{assertion}");
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn set(
    tag: &mut LanguageTag,
    language: &str,
    region: Option<&str>,
    script: Option<&str>,
    variants: Option<&[&str]>,
) {
    tag.language = Some(language.into());
    tag.region = region.map(Into::into);
    tag.script = script.map(Into::into);
    tag.variants = variants.map(strings).unwrap_or_default();
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 19;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Utils;
use Biber::LangTag;
use Log::Log4perl;
chdir("t/tdata");

# Set up Biber object
my $biber = Biber->new();
my $LEVEL = 'ERROR';
my $l4pconf = qq|
    log4perl.category.main                             = $LEVEL, Screen
    log4perl.category.screen                           = $LEVEL, Screen
    log4perl.appender.Screen                           = Log::Log4perl::Appender::Screen
    log4perl.appender.Screen.utf8                      = 1
    log4perl.appender.Screen.Threshold                 = $LEVEL
    log4perl.appender.Screen.stderr                    = 0
    log4perl.appender.Screen.layout                    = Log::Log4perl::Layout::SimpleLayout
|;
Log::Log4perl->init(\$l4pconf);

is_deeply($biber->langtags->parse('de')->dump, {language => 'de'}, 'BCP47 - 1');
is_deeply($biber->langtags->parse('i-enochian')->dump, { grandfathered => "i-enochian" }, 'BCP47 - 2');
is_deeply($biber->langtags->parse('zh-Hant')->dump, { language => 'zh', script => 'Hant' }, 'BCP47 - 3');
is_deeply($biber->langtags->parse('zh-cmn-Hans-CN')->dump, { language => 'zh', extlang => ['cmn'], script => 'Hans', region => 'CN' }, 'BCP47 - 4');
is_deeply($biber->langtags->parse('cmn-Hans-CN')->dump, { language => 'cmn', script => 'Hans', region => 'CN' }, 'BCP47 - 5');
is_deeply($biber->langtags->parse('yue-HK')->dump, { language => 'yue', region => 'HK' }, 'BCP47 - 6');
is_deeply($biber->langtags->parse('sl-rozaj')->dump, { language => 'sl', variant => ['rozaj'] }, 'BCP47 - 7');
is_deeply($biber->langtags->parse('sl-rozaj-biske')->dump, { language => 'sl', variant => ['rozaj', 'biske'] }, 'BCP47 - 8');
is_deeply($biber->langtags->parse('de-CH-1901')->dump, { language => 'de', region => 'CH', variant => ['1901'] }, 'BCP47 - 9');
is_deeply($biber->langtags->parse('hy-Latn-IT-arevela')->dump, { language => 'hy', region => 'IT', script => 'Latn', variant => ['arevela'] }, 'BCP47 - 10');
is_deeply($biber->langtags->parse('de-DE')->dump, { language => 'de', region => 'DE' }, 'BCP47 - 11');
is_deeply($biber->langtags->parse('es-419')->dump, { language => 'es', region => '419' }, 'BCP47 - 12');
is_deeply($biber->langtags->parse('de-CH-x-phonebk')->dump, { language => 'de', region => 'CH', privateuse => ['phonebk'] }, 'BCP47 - 13');
is_deeply($biber->langtags->parse('az-Arab-x-AZE-derbend')->dump, { language => 'az', script => 'Arab', privateuse => ['AZE', 'derbend'] }, 'BCP47 - 14');
is_deeply($biber->langtags->parse('en-US-u-islamcal')->dump, { language => 'en', region => 'US', extension => ['islamcal'] }, 'BCP47 - 15');
is_deeply($biber->langtags->parse('en-a-myext-b-another')->dump, { language => 'en', extension => ['myext', 'another'] }, 'BCP47 - 16');
is_deeply($biber->langtags->parse('zh-CN-a-myext-x-private')->dump, { language => 'zh', region => 'CN', extension => ['myext'], privateuse => ['private'] }, 'BCP47 - 17');
is_deeply($biber->langtags->parse('de-419-DE'), undef, 'BCP47 - 18');
is_deeply($biber->langtags->parse('a-DE'), undef, 'BCP47 - 19');
"#;

#[test]
fn assertion_001_bcp47_1() {
    pass_upstream(
        "BCP47 - 1",
        r"$biber->langtags->parse('de')->dump",
        r"{language => 'de'}",
        r"is_deeply($biber->langtags->parse('de')->dump, {language => 'de'}, 'BCP47 - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_bcp47_2() {
    pass_upstream(
        "BCP47 - 2",
        r"$biber->langtags->parse('i-enochian')->dump",
        r#"{ grandfathered => "i-enochian" }"#,
        r#"is_deeply($biber->langtags->parse('i-enochian')->dump, { grandfathered => "i-enochian" }, 'BCP47 - 2');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_bcp47_3() {
    pass_upstream(
        "BCP47 - 3",
        r"$biber->langtags->parse('zh-Hant')->dump",
        r"{ language => 'zh', script => 'Hant' }",
        r"is_deeply($biber->langtags->parse('zh-Hant')->dump, { language => 'zh', script => 'Hant' }, 'BCP47 - 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_bcp47_4() {
    pass_upstream(
        "BCP47 - 4",
        r"$biber->langtags->parse('zh-cmn-Hans-CN')->dump",
        r"{ language => 'zh', extlang => ['cmn'], script => 'Hans', region => 'CN' }",
        r"is_deeply($biber->langtags->parse('zh-cmn-Hans-CN')->dump, { language => 'zh', extlang => ['cmn'], script => 'Hans', region => 'CN' }, 'BCP47 - 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_bcp47_5() {
    pass_upstream(
        "BCP47 - 5",
        r"$biber->langtags->parse('cmn-Hans-CN')->dump",
        r"{ language => 'cmn', script => 'Hans', region => 'CN' }",
        r"is_deeply($biber->langtags->parse('cmn-Hans-CN')->dump, { language => 'cmn', script => 'Hans', region => 'CN' }, 'BCP47 - 5');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_bcp47_6() {
    pass_upstream(
        "BCP47 - 6",
        r"$biber->langtags->parse('yue-HK')->dump",
        r"{ language => 'yue', region => 'HK' }",
        r"is_deeply($biber->langtags->parse('yue-HK')->dump, { language => 'yue', region => 'HK' }, 'BCP47 - 6');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_bcp47_7() {
    pass_upstream(
        "BCP47 - 7",
        r"$biber->langtags->parse('sl-rozaj')->dump",
        r"{ language => 'sl', variant => ['rozaj'] }",
        r"is_deeply($biber->langtags->parse('sl-rozaj')->dump, { language => 'sl', variant => ['rozaj'] }, 'BCP47 - 7');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_bcp47_8() {
    pass_upstream(
        "BCP47 - 8",
        r"$biber->langtags->parse('sl-rozaj-biske')->dump",
        r"{ language => 'sl', variant => ['rozaj', 'biske'] }",
        r"is_deeply($biber->langtags->parse('sl-rozaj-biske')->dump, { language => 'sl', variant => ['rozaj', 'biske'] }, 'BCP47 - 8');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_009_bcp47_9() {
    pass_upstream(
        "BCP47 - 9",
        r"$biber->langtags->parse('de-CH-1901')->dump",
        r"{ language => 'de', region => 'CH', variant => ['1901'] }",
        r"is_deeply($biber->langtags->parse('de-CH-1901')->dump, { language => 'de', region => 'CH', variant => ['1901'] }, 'BCP47 - 9');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_010_bcp47_10() {
    pass_upstream(
        "BCP47 - 10",
        r"$biber->langtags->parse('hy-Latn-IT-arevela')->dump",
        r"{ language => 'hy', region => 'IT', script => 'Latn', variant => ['arevela'] }",
        r"is_deeply($biber->langtags->parse('hy-Latn-IT-arevela')->dump, { language => 'hy', region => 'IT', script => 'Latn', variant => ['arevela'] }, 'BCP47 - 10');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_011_bcp47_11() {
    pass_upstream(
        "BCP47 - 11",
        r"$biber->langtags->parse('de-DE')->dump",
        r"{ language => 'de', region => 'DE' }",
        r"is_deeply($biber->langtags->parse('de-DE')->dump, { language => 'de', region => 'DE' }, 'BCP47 - 11');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_012_bcp47_12() {
    pass_upstream(
        "BCP47 - 12",
        r"$biber->langtags->parse('es-419')->dump",
        r"{ language => 'es', region => '419' }",
        r"is_deeply($biber->langtags->parse('es-419')->dump, { language => 'es', region => '419' }, 'BCP47 - 12');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_013_bcp47_13() {
    pass_upstream(
        "BCP47 - 13",
        r"$biber->langtags->parse('de-CH-x-phonebk')->dump",
        r"{ language => 'de', region => 'CH', privateuse => ['phonebk'] }",
        r"is_deeply($biber->langtags->parse('de-CH-x-phonebk')->dump, { language => 'de', region => 'CH', privateuse => ['phonebk'] }, 'BCP47 - 13');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_014_bcp47_14() {
    pass_upstream(
        "BCP47 - 14",
        r"$biber->langtags->parse('az-Arab-x-AZE-derbend')->dump",
        r"{ language => 'az', script => 'Arab', privateuse => ['AZE', 'derbend'] }",
        r"is_deeply($biber->langtags->parse('az-Arab-x-AZE-derbend')->dump, { language => 'az', script => 'Arab', privateuse => ['AZE', 'derbend'] }, 'BCP47 - 14');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_015_bcp47_15() {
    pass_upstream(
        "BCP47 - 15",
        r"$biber->langtags->parse('en-US-u-islamcal')->dump",
        r"{ language => 'en', region => 'US', extension => ['islamcal'] }",
        r"is_deeply($biber->langtags->parse('en-US-u-islamcal')->dump, { language => 'en', region => 'US', extension => ['islamcal'] }, 'BCP47 - 15');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_016_bcp47_16() {
    pass_upstream(
        "BCP47 - 16",
        r"$biber->langtags->parse('en-a-myext-b-another')->dump",
        r"{ language => 'en', extension => ['myext', 'another'] }",
        r"is_deeply($biber->langtags->parse('en-a-myext-b-another')->dump, { language => 'en', extension => ['myext', 'another'] }, 'BCP47 - 16');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_017_bcp47_17() {
    pass_upstream(
        "BCP47 - 17",
        r"$biber->langtags->parse('zh-CN-a-myext-x-private')->dump",
        r"{ language => 'zh', region => 'CN', extension => ['myext'], privateuse => ['private'] }",
        r"is_deeply($biber->langtags->parse('zh-CN-a-myext-x-private')->dump, { language => 'zh', region => 'CN', extension => ['myext'], privateuse => ['private'] }, 'BCP47 - 17');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_018_bcp47_18() {
    pass_upstream(
        "BCP47 - 18",
        r"$biber->langtags->parse('de-419-DE')",
        r"undef",
        r"is_deeply($biber->langtags->parse('de-419-DE'), undef, 'BCP47 - 18');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_019_bcp47_19() {
    pass_upstream(
        "BCP47 - 19",
        r"$biber->langtags->parse('a-DE')",
        r"undef",
        r"is_deeply($biber->langtags->parse('a-DE'), undef, 'BCP47 - 19');",
        UPSTREAM_SOURCE,
    );
}
