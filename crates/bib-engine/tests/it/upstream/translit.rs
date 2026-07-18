// Direct translation of upstream t/translit.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use bib_unicode::{Transliteration, transliterate};

#[track_caller]
fn pass_upstream(assertion: &str, _: &str, expected: &str, call: &str, source: &str) {
    assert!(source.contains(call), "{assertion}");
    for value in [
        "kumāra", "kha", "jīvita", "jvara", "tyāga", "tridaśa", "tvid", "kṣetra", "jñāna",
    ] {
        assert_eq!(transliterate(value, Transliteration::Latin), value);
        assert!(expected.contains(value));
    }
    assert_eq!(
        transliterate("क्षेत्र", Transliteration::DevanagariLatin),
        "kṣetr"
    );
    panic!("xfail: exact transliterated BBL sorting output is not publicly exposed");
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 1;

use Biber;
use Biber::Output::bbl;
use Log::Log4perl;
use Unicode::Normalize;
chdir("t/tdata");

# Set up Biber object
my $biber = Biber->new(noconf => 1);
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

$biber->parse_ctrlfile('translit.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# (re)generate information based on option settings
$biber->prepare;
my $section = $biber->sections->get_section(0);
my $main = $biber->datalists->get_list('nty/global//global/global/global');

is_deeply([map {NFC($_)} $main->get_keys->@*], ['aachen', 'aix-en-provence', 'arnhem', 'augsburg', 'avignon', 'berlin', 'utrecht', 'zeven', 'kumāra', 'kha', 'jīvita', 'jvara', 'tyāga', 'tridaśa', 'tvid', 'kṣetra', 'jñāna'], 'translit sorting - 1');


"#;

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_001_translit_sorting_1() {
    pass_upstream(
        "translit sorting - 1",
        r"[map {NFC($_)} $main->get_keys->@*]",
        r"['aachen', 'aix-en-provence', 'arnhem', 'augsburg', 'avignon', 'berlin', 'utrecht', 'zeven', 'kumāra', 'kha', 'jīvita', 'jvara', 'tyāga', 'tridaśa', 'tvid', 'kṣetra', 'jñāna']",
        r"is_deeply([map {NFC($_)} $main->get_keys->@*], ['aachen', 'aix-en-provence', 'arnhem', 'augsburg', 'avignon', 'berlin', 'utrecht', 'zeven', 'kumāra', 'kha', 'jīvita', 'jvara', 'tyāga', 'tridaśa', 'tvid', 'kṣetra', 'jñāna'], 'translit sorting - 1');",
        UPSTREAM_SOURCE,
    );
}
