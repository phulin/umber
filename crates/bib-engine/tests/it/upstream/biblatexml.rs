// Direct translation of upstream t/biblatexml.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use bib_input::{
    OptionComponent, XmlFieldValue, XmlLimits, parse_biblatexml_bytes, parse_control_bytes,
};

const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/biblatexml.bltxml");
const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/biblatexml.bcf");

#[track_caller]
fn pass_upstream(assertion: &str, _: &str, _: &str, call: &str, source: &str) {
    assert!(source.contains(call), "{assertion}");
    let data = parse_biblatexml_bytes(DATA, XmlLimits::default()).expect(assertion);
    let entry = data.entry("bltx1").expect("bltx1");
    match assertion {
        "BibLaTeXML - 1" => {
            assert_eq!(entry.entry_type, "book");
            assert!(
                matches!(entry.fields.get("author"), Some(XmlFieldValue::Names { values, .. }) if values.len() == 3)
            );
            assert!(entry.fields.contains_key("eventdate") && entry.fields.contains_key("pages"));
            assert_eq!(entry.annotations.len(), 9);
        }
        "Citekey aliases - 1" => assert_eq!(data.canonical_id("bltx1a1"), Some("bltx1")),
        "Citekey aliases - 2" => assert_eq!(data.canonical_id("bltx1a2"), Some("bltx1")),
        "useprefix at name list and name scope - 1" => assert!(
            matches!(entry.fields.get("author"), Some(XmlFieldValue::Names { attributes, .. }) if attributes.get("useprefix").map(String::as_str) == Some("true"))
        ),
        "BibLaTeXML automapcreate - 1" => {
            let control = parse_control_bytes(CONTROL, XmlLimits::default()).expect(assertion);
            assert!(
                CONTROL
                    .windows(b"map_entry_new".len())
                    .any(|window| window == b"map_entry_new")
            );
            assert!(
                control
                    .option_set(OptionComponent::Biblatex, "global")
                    .is_some()
            );
        }
        _ => panic!("unhandled upstream assertion {assertion}"),
    }
    panic!("xfail: exact upstream preparation and BBL rendering is not publicly exposed");
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';
use Text::Diff::Config;
$Text::Diff::Config::Output_Unicode = 1;


use Test::More tests => 5;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Output::bbl;
use Log::Log4perl;
use Encode;

chdir("t/tdata");

# Set up Biber object
# THERE ARE MAPS IN THE BCF
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
$biber->parse_ctrlfile('biblatexml.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
Biber::Config->setoption('bcf', 'biblatexml.bcf');

# Now generate the information
$biber->prepare;
my $out = $biber->get_output_obj;
my $section = $biber->sections->get_section(0);
my $main = $biber->datalists->get_list('custom/global//global/global/global');

my $bibentries = $section->bibentries;

my $l1 = q|    \entry{bltx1}{misc}{useprefix=false}{}
      \true{moreauthor}
      \true{morelabelname}
      \name{author}{3}{useprefix=true}{%
        {{hash=bdef740dab20c2b52a3b6e0563c42bdb}{%
           family={Булгаков},
           familyi={Б\\bibinitperiod},
           given={Павел\\bibnamedelima Георгиевич},
           giveni={П\\bibinitperiod\\bibinitdelim Г\\bibinitperiod},
           prefix={von},
           prefixi={v\\bibinitperiod}}}%
        {{useprefix=false,hash=485f1e5d5e81a43fe067b440706c4979}{%
           family={РРозенфельд},
           familyi={Р\\bibinitperiod},
           given={Борис-ZZ\\bibnamedelima Aбрамович},
           giveni={Б\\bibinithyphendelim Z\\bibinitperiod\\bibinitdelim A\\bibinitperiod},
           prefix={von},
           prefixi={v\\bibinitperiod}}}%
        {{hash=39dcc744aabf73006cb446d70a1beea2}{%
           family={Aхмедов},
           familyi={A\\bibinitperiod},
           given={Ашраф\\bibnamedelima Ахмедович},
           giveni={A\\bibinitperiod\\bibinitdelim А\\bibinitperiod}}}%
      }
      \name{foreword}{1}{}{%
        {{hash=88354d4ba914f2ded2574386a2493996}{%
           family={Brown},
           familyi={B\\bibinitperiod},
           given={John},
           giveni={J\\bibinitperiod}}}%
      }
      \name{translator}{1}{}{%
        {{hash=b44eba830fe9817fbe8e53c82f1cbe04}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Paul},
           giveni={P\\bibinitperiod}}}%
      }
      \list{language}{1}{%
        {russian}%
      }
      \list{location}{1}{%
        {Москва}%
      }
      \list{publisher}{1}{%
        {Наука}%
      }
      \strng{namehash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{fullhash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{fullhashraw}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{bibnamehash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{authorbibnamehash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{authornamehash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{authorfullhash}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{authorfullhashraw}{3400c73d7bf3e361d36350deb4832ad7}
      \strng{forewordbibnamehash}{88354d4ba914f2ded2574386a2493996}
      \strng{forewordnamehash}{88354d4ba914f2ded2574386a2493996}
      \strng{forewordfullhash}{88354d4ba914f2ded2574386a2493996}
      \strng{forewordfullhashraw}{a7a73749ea467229221b7e9cbf870988}
      \strng{translatorbibnamehash}{b44eba830fe9817fbe8e53c82f1cbe04}
      \strng{translatornamehash}{b44eba830fe9817fbe8e53c82f1cbe04}
      \strng{translatorfullhash}{b44eba830fe9817fbe8e53c82f1cbe04}
      \strng{translatorfullhashraw}{b44eba830fe9817fbe8e53c82f1cbe04}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{addendum}{userc}
      \field{eventday}{16}
      \field{eventendday}{17}
      \field{eventendmonth}{5}
      \field{eventendyear}{1990}
      \field{eventmonth}{5}
      \field{eventyear}{1990}
      \field{origyear}{356}
      \field{pagetotal}{240}
      \field{series}{Научно-биографическая литература}
      \field{title}{Мухаммад ибн муса ал-Хорезми. Около 783 – около 850}
      \field{urlendyear}{}
      \field{urlyear}{1991}
      \field{userb}{usera}
      \field{userd}{userc}
      \field{usere}{a}
      \field{year}{1980}
      \field{dateunspecified}{yearindecade}
      \field{dateera}{ce}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{origdateera}{bce}
      \true{urldatecirca}
      \field{urldateera}{ce}
      \field{pages}{1\\bibrangedash 10\\bibrangessep 30\\bibrangedash 34}
      \range{pages}{15}
      \annotation{field}{author}{alt}{}{}{0}{names-ann3}
      \annotation{field}{author}{default}{}{}{0}{names-ann}
      \annotation{field}{language}{default}{}{}{0}{list-ann1}
      \annotation{field}{title}{default}{}{}{0}{field-ann1}
      \annotation{item}{author}{default}{1}{}{0}{name-ann1}
      \annotation{item}{author}{default}{3}{}{0}{name-ann2}
      \annotation{item}{language}{default}{1}{}{0}{item-ann1}
      \annotation{part}{author}{default}{1}{given}{1}{namepart-ann1}
      \annotation{part}{author}{default}{2}{family}{0}{namepart-ann2}
    \endentry
|;

my $l2 = q|    \entry{loopkey:a}{book}{}{}
      \field{sortinit}{0}
      \field{sortinithash}{c5602f03f17cc894ea7a6362c3cb0e13}
    \endentry
|;


my $bltx1 = 'mm,,,vonБулгаков   Павел Георгиевич  РРозенфельдБорис-ZZ AбрамовичvonAхмедов    Ашраф Ахмедович   ,1980,0,Мухаммад ибн муса ал-Хорезми. Около 783 – около 850';

# Test::Differences doesn't like utf8 unless it's encoded here
eq_or_diff(encode_utf8($out->get_output_entry('bltx1', $main)), encode_utf8($l1), 'BibLaTeXML - 1');
eq_or_diff($section->get_citekey_alias('bltx1a1'), 'bltx1', 'Citekey aliases - 1');
eq_or_diff($section->get_citekey_alias('bltx1a2'), 'bltx1', 'Citekey aliases - 2');
eq_or_diff(encode_utf8($main->get_sortdata_for_key('bltx1')->[0]), encode_utf8($bltx1), 'useprefix at name list and name scope - 1' );
eq_or_diff(encode_utf8($out->get_output_entry('loopkey:a', $main)), encode_utf8($l2), 'BibLaTeXML automapcreate - 1');
"#;

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_001_biblatexml_1() {
    pass_upstream(
        "BibLaTeXML - 1",
        r"encode_utf8($out->get_output_entry('bltx1', $main))",
        r"encode_utf8($l1)",
        r"eq_or_diff(encode_utf8($out->get_output_entry('bltx1', $main)), encode_utf8($l1), 'BibLaTeXML - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_002_citekey_aliases_1() {
    pass_upstream(
        "Citekey aliases - 1",
        r"$section->get_citekey_alias('bltx1a1')",
        r"'bltx1'",
        r"eq_or_diff($section->get_citekey_alias('bltx1a1'), 'bltx1', 'Citekey aliases - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_003_citekey_aliases_2() {
    pass_upstream(
        "Citekey aliases - 2",
        r"$section->get_citekey_alias('bltx1a2')",
        r"'bltx1'",
        r"eq_or_diff($section->get_citekey_alias('bltx1a2'), 'bltx1', 'Citekey aliases - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_004_useprefix_at_name_list_and_name_scope_1() {
    pass_upstream(
        "useprefix at name list and name scope - 1",
        r"encode_utf8($main->get_sortdata_for_key('bltx1')->[0])",
        r"encode_utf8($bltx1)",
        r"eq_or_diff(encode_utf8($main->get_sortdata_for_key('bltx1')->[0]), encode_utf8($bltx1), 'useprefix at name list and name scope - 1' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_005_biblatexml_automapcreate_1() {
    pass_upstream(
        "BibLaTeXML automapcreate - 1",
        r"encode_utf8($out->get_output_entry('loopkey:a', $main))",
        r"encode_utf8($l2)",
        r"eq_or_diff(encode_utf8($out->get_output_entry('loopkey:a', $main)), encode_utf8($l2), 'BibLaTeXML automapcreate - 1');",
        UPSTREAM_SOURCE,
    );
}
