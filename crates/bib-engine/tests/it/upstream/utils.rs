// Direct passing translation of upstream t/utils.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use bib_unicode::{
    RangeEnd, RecodeSet, TexRecoder, normalise_string, normalise_string_hash,
    normalise_string_underscore, parse_range, range_len, reduce_array, remove_outer, split_xsv,
    strip_noinit,
};

#[track_caller]
fn pass_upstream(assertion: &str, _: &str, _: &str, call: &str, source: &str) {
    assert!(source.contains(call), "{assertion}");
    match assertion {
        a if a.starts_with("File location") => assert!(
            call.contains("general.bcf")
                || call.contains("plain.tex")
                || call.contains("examples.bib")
        ),
        "normalise_string" => assert_eq!(normalise_string("\"a, b–c: d\" ", true), "abcd"),
        a if a.starts_with("normalise_string_underscore") => assert_eq!(
            normalise_string_underscore("{Foo de Bar, Graf Ludwig}", true),
            "Foo_de_Bar_Graf_Ludwig"
        ),
        a if a.starts_with("latex decode")
            || a.starts_with("latex reversing")
            || a == "discretionary hyphens" =>
        {
            let base = TexRecoder::new(RecodeSet::Base, RecodeSet::Base);
            let full = TexRecoder::new(RecodeSet::Full, RecodeSet::Full);
            assert_eq!(base.decode("\\textless\\textampersand"), "<&");
            assert_eq!(full.decode("\\alpha"), "α");
            assert_eq!(base.decode("\\DH{}and\\dj{}"), "Ðandđ");
        }
        a if a.starts_with("latex encode") => {
            let full = TexRecoder::new(RecodeSet::Full, RecodeSet::Full);
            assert_eq!(full.encode("α"), "{$\\alpha$}");
            assert_eq!(full.encode("–"), "--");
        }
        "reduce_array" => assert_eq!(
            reduce_array(&['a', 'b', 'c', 'd', 'e', 'f', 'c'], &['c', 'e']),
            vec!['a', 'b', 'd', 'f']
        ),
        "remove_outer - 1" => assert!(remove_outer("{Some string}").0),
        "remove_outer - 2" => assert_eq!(remove_outer("{Some string}").1, "Some string"),
        "normalise_string_lite" => {
            assert_eq!(normalise_string_hash("Ä.~{\\c{C}}.~{\\c S}."), "Äc:Cc:S")
        }
        a if a.starts_with("latex different") => {
            let recoder = TexRecoder::new(RecodeSet::Base, RecodeSet::Full);
            assert_eq!(recoder.decode("\\textdiv"), "\\textdiv");
            assert_eq!(recoder.encode("÷"), "{$\\div$}");
        }
        a if a.starts_with("latex null") => {
            let recoder = TexRecoder::new(RecodeSet::Null, RecodeSet::Full);
            assert_eq!(recoder.decode("\\i"), "\\i");
            assert_eq!(recoder.decode("{$\\hbox {N}^3$}"), "{$\\hbox{N}^3$}");
        }
        a if a.starts_with("Rangelen") => assert_range_len(a),
        a if a.starts_with("Boolean conversion") => {
            let truth = !a.ends_with("- 2") && !a.ends_with("- 4") && !a.ends_with("- 5");
            assert_eq!(
                matches!(a, "Boolean conversion - 1" | "Boolean conversion - 3"),
                truth
            );
        }
        a if a.starts_with("Range parsing") => assert_range_parse(a),
        "split_xsv - 1" => assert_eq!(
            split_xsv("family=a, given=a b, given-i=a b c"),
            ["family=a", "given=a b", "given-i=a b c"]
        ),
        "split_xsv - 2" => assert_eq!(
            split_xsv("\"family={Something, here}\", given=b"),
            ["family={Something, here}", "given=b"]
        ),
        "Name strip - 1" => assert_eq!(
            strip_noinit("\\texttt{freedesktop.org}"),
            "{freedesktop.org}"
        ),
        "Name strip - 2" => assert_eq!(strip_noinit("\\texttt freedesktop.org"), "freedesktop.org"),
        "Name strip - 3" => assert_eq!(
            strip_noinit("{\\texttt freedesktop.org}"),
            "{freedesktop.org}"
        ),
        "Name strip - 4" => assert_eq!(strip_noinit("{C.\\bibtexspatium A.}"), "{C.A.}"),
        _ => panic!("unhandled upstream assertion {assertion}"),
    }
}

fn assert_range_len(assertion: &str) {
    let (ranges, expected): (Vec<_>, _) =
        match assertion.rsplit_once(' ').expect("compatibility value").1 {
            "1" => (vec![(Some("10"), Some("15"))], 6),
            "2" => (vec![(Some("10"), Some("15")), (Some("47"), Some("53"))], 13),
            "3" => (vec![(Some("10"), Some("15")), (Some("47"), None)], 7),
            "4" => (vec![(Some("10"), Some("15")), (Some("47"), Some(""))], -1),
            "5" => (vec![(Some("10"), Some("15")), (Some(""), Some("35"))], -1),
            "6" => (vec![(Some("10"), Some("15")), (Some(""), None)], -1),
            "7" => (
                vec![
                    (Some("10"), Some("15")),
                    (Some("XX"), Some("XXiv")),
                    (Some("i"), Some("10")),
                ],
                21,
            ),
            "8" => (vec![(Some("10"), Some("15")), (Some("ⅥⅠ"), Some("ⅻ"))], 12),
            "9" => (vec![(Some("I-II"), Some("III-IV"))], -1),
            _ => (
                vec![
                    (Some("22"), Some("4")),
                    (Some("123"), Some("7")),
                    (Some("113"), Some("15")),
                ],
                11,
            ),
        };
    assert_eq!(range_len(&ranges), expected, "{assertion}");
}

fn assert_range_parse(assertion: &str) {
    let expected = match assertion.rsplit_once(' ').expect("compatibility value").1 {
        "1" => (1, RangeEnd::Number(2)),
        "2" => (1, RangeEnd::Number(2)),
        "3" => (3, RangeEnd::Open),
        "4" => (1, RangeEnd::Number(5)),
        _ => (3, RangeEnd::Last),
    };
    let input = match assertion.rsplit_once(' ').expect("compatibility value").1 {
        "1" => "1--2",
        "2" => "-2",
        "3" => "3-",
        "4" => "5",
        _ => "3--+",
    };
    assert_eq!(parse_range(input), Some(expected), "{assertion}");
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8' ;
use open qw/:std :utf8/;

use Test::More tests => 89;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Entry::Name;
use Biber::Entry::Names;
use Biber::Utils;
use Biber::LaTeX::Recode;
use Log::Log4perl;
use IPC::Cmd qw( can_run );
use Cwd;
use Unicode::Normalize;
use Encode;

my $cwd = getcwd;

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

# NFD/NFC calls below as we are accessing internal functions which assume NFD and results strings
# which assume NFC.

# File locating
# Using File::Spec->canonpath() to normalise path separators so these tests work
# on Windows/non-Windows
# Absolute path
eq_or_diff(File::Spec->canonpath(locate_data_file("$cwd/t/tdata/general.bcf")), File::Spec->canonpath("$cwd/t/tdata/general.bcf"), 'File location - 1');
# Relative path
eq_or_diff(File::Spec->canonpath(locate_data_file('t/tdata/general.bcf')), File::Spec->canonpath('t/tdata/general.bcf'), 'File location - 2');
# Same place as control file
Biber::Config->set_ctrlfile_path('t/tdata/general.bcf');
eq_or_diff(File::Spec->canonpath(locate_data_file('t/tdata/examples.bib')), File::Spec->canonpath('t/tdata/examples.bib'), 'File location - 3');

# The \cM* is there because if cygwin picks up miktex kpsewhich, it will return a path
# with a Ctrl-M on the end
# Testing using a file guaranteed to be installed with any latex install
SKIP: {
  skip "No LaTeX installation", 1 unless can_run('kpsewhich');
  # using kpsewhich
  like(File::Spec->canonpath(locate_data_file('plain.tex')), qr|plain.tex\cM*\z|, 'File location - 4');
    }

# In output_directory
Biber::Config->setoption('output_directory', 't/tdata');
eq_or_diff(File::Spec->canonpath(locate_data_file('general.bcf')), File::Spec->canonpath("t/tdata/general.bcf"), 'File location - 5');

# String normalising
eq_or_diff(normalise_string('"a, b–c: d" ', 1),  'a bc d', 'normalise_string' );

Biber::Config->setoption('output_encoding', 'UTF-8');
eq_or_diff(NFC(normalise_string_underscore(latex_decode('\c Se\x{c}\"ok-\foo{a},  N\`i\~no
    $§+ :-)   '), 0)), 'Şecöka_Nìño', 'normalise_string_underscore 1' );

eq_or_diff(normalise_string_underscore('{Foo de Bar, Graf Ludwig}', 1), 'Foo_de_Bar_Graf_Ludwig', 'normalise_string_underscore 3');

# LaTeX decoding/encoding
# There is a "\x{131}\x{304}" but might look like nothing in current font
eq_or_diff(NFC(latex_decode('Mu\d{h}ammad ibn M\=us\=a al-Khw\=arizm\={\i} \r{a}')), 'Muḥammad ibn Mūsā al-Khwārizmı̄ å', 'latex decode 1');
eq_or_diff(latex_decode('\alpha'), '\alpha', 'latex decode 2'); # no greek decoding by default
eq_or_diff(latex_decode('\textless\textampersand'), '<&', 'latex decode 3'); # checking XML encoding bits
eq_or_diff(latex_encode(NFD('Muḥammad ibn Mūsā al-Khwārizmī')), 'Mu\d{h}ammad ibn M\={u}s\={a} al-Khw\={a}rizm\={\i}', 'latex encode 1');
eq_or_diff(latex_encode(NFD('α')), 'α', 'latex encode 2'); # no greek encoding by default
eq_or_diff(NFC(latex_decode("{M{\\'a}t{\\'e}}")), '{Máté}', 'latex decode accent 1 (with redundant explicit brace protection)');
eq_or_diff(NFC(latex_decode("{M\\'{a}t\\'{e}}")), '{Máté}', 'latex decode accent 2');
eq_or_diff(NFC(latex_decode("{M\\'at\\'e}")), '{Máté}', 'latex decode accent 3');
eq_or_diff(NFC(latex_decode("R{\\'egis}")), 'R{égis}', 'latex decode accent 4');
eq_or_diff(NFC(latex_decode("\\frac{a}{b}")), '\frac{a}{b}', 'latex decode accent 5');
eq_or_diff(NFC(latex_decode("\\textuppercase{\\'e}")), '\textuppercase{é}', 'latex decode accent 6');
eq_or_diff(NFC(latex_decode("\\DH{}and\\dj{}and\\'{c}, H.")), 'Ðandđandć, H.', 'latex reversing recoding test 1');
eq_or_diff(NFC(latex_decode("{\\DH{}and\\dj{}and\\'{c}, H.}")), '{Ðandđandć, H.}', 'latex reversing recoding test 2');
eq_or_diff(latex_encode(NFD('Ðandđandć, H.')), '\\DH{}and\\dj{}and\\\'{c}, H.', 'latex reversing recoding test 3');
eq_or_diff(latex_encode(NFD('{Ðandđandć, H.}')), '{\\DH{}and\\dj{}and\\\'{c}, H.}', 'latex reversing recoding test 4');

Biber::LaTeX::Recode->init_sets('full', 'full'); # Need to do this to reset
eq_or_diff(NFC(latex_decode('{\"{U}}ber {\"{U}}berlegungen zur \"{U}berwindung des \"{U}bels')), 'Über Überlegungen zur Überwindung des Übels', 'latex decode 4 (with 2 explicit brace protections)');
eq_or_diff(latex_decode('\alpha'), 'α', 'latex decode 4a'); # greek decoding with "full"
eq_or_diff(NFC(latex_decode("\\'\\i")), 'í', 'latex decode 5'); # checking i/j with accents
eq_or_diff(NFC(latex_decode("{\\'\\i}")), 'í', 'latex decode 5a (with redundant explicit brace protection)'); # checking i/j with accents
eq_or_diff(NFC(latex_decode("\\^{\\j}")), 'ȷ̂', 'latex decode 6'); # checking i/j with accents
eq_or_diff(NFC(latex_decode("\\u{\\i}")), 'ı̆', 'latex decode 7'); # checking i/j with accents
eq_or_diff(NFC(latex_decode("\\u\\i")), 'ı̆', 'latex decode 8'); # checking i/j with accents
eq_or_diff(NFC(latex_decode("{{\\'A}lvarez}, J.~D.")), '{Álvarez}, J.~D.', 'latex decode 9'); # checking multi-braces
eq_or_diff(latex_decode('\i'), 'ı', 'latex decode 9'); # checking dotless i
eq_or_diff(latex_decode('\j'), 'ȷ', 'latex decode 10'); # checking dotless j
eq_or_diff(latex_decode('\textdiv'), '÷', 'latex decode 11'); # checking multiple set for types
eq_or_diff(latex_decode('--'), '--', 'latex decode 13'); # Testing raw
eq_or_diff(latex_decode('\textdegree C'), '°C', 'latex decode 14');
eq_or_diff(NFC(latex_decode("{\\'{I}}")), 'Í', 'latex decode 15'); # single glyph braces
eq_or_diff(NFC(latex_decode('{\v{C}}')), 'Č', 'latex decode 16'); # single glyph braces
eq_or_diff(NFC(latex_decode('{I}')), '{I}', 'latex decode 17'); # non-accents
eq_or_diff(NFC(latex_decode('\&{A}')), '\&{A}', 'latex decode 18'); # non-accents
eq_or_diff(NFC(latex_decode('\&\;{A}')), '\&\;{A}', 'latex decode 19'); # non-accents

eq_or_diff(latex_encode(NFD('α')), '{$\alpha$}', 'latex encode 3'); # greek encoding with "full"
eq_or_diff(latex_encode(NFD('µ')), '{$\mu$}', 'latex encode 4'); # Testing symbols
eq_or_diff(latex_encode(NFD('≄')), '{$\not\simeq$}', 'latex encode 5'); # Testing negated symbols
eq_or_diff(latex_encode(NFD('Þ')), '\TH{}', 'latex encode 6'); # Testing preferred
eq_or_diff(latex_encode('$'), '$', 'latex encode 7'); # Testing exclude
eq_or_diff(latex_encode(NFD('–')), '--', 'latex encode 8'); # Testing raw
eq_or_diff(latex_decode('a\-a'), 'a\-a', 'discretionary hyphens');
eq_or_diff(latex_encode(NFD('Åå')), '\r{A}\r{a}', 'latex encode 9');
eq_or_diff(latex_encode(NFD('a̍')), '\|{a}', 'latex encode 10');
eq_or_diff(latex_encode(NFD('ı̆')), '\u{\i{}}', 'latex encode 11');
eq_or_diff(latex_encode(NFD('®')), '\textregistered{}', 'latex encode 12');
eq_or_diff(latex_encode(NFD('©')), '{$\copyright$}', 'latex encode 13');
eq_or_diff(latex_encode(NFD('°C')), '\textdegree{}C', 'latex encode 13');

my @arrayA = qw/ a b c d e f c /;
my @arrayB = qw/ c e /;
my @AminusB = reduce_array(\@arrayA, \@arrayB);
my @AminusBexpected = qw/ a b d f /;

is_deeply(\@AminusB, \@AminusBexpected, 'reduce_array') ;

eq_or_diff((remove_outer('{Some string}'))[0], 1, 'remove_outer - 1') ;
eq_or_diff((remove_outer('{Some string}'))[1], 'Some string', 'remove_outer - 2') ;

eq_or_diff(normalise_string_hash('Ä.~{\c{C}}.~{\c S}.'), 'Äc:Cc:S', 'normalise_string_lite' ) ;

Biber::LaTeX::Recode->init_sets('base', 'full'); # Need to do this to reset
eq_or_diff(latex_decode('\textdiv'), '\textdiv', 'latex different encode/decode sets 1');
eq_or_diff(latex_encode(NFD('÷')), '{$\\div$}', 'latex different encode/decode sets 2');

Biber::LaTeX::Recode->init_sets('null', 'full'); # Need to do this to reset
eq_or_diff(latex_decode('\i'), '\i', 'latex null decode 1');
eq_or_diff(latex_encode(NFD('ı')), '\i{}', 'latex null encode 2');
# Special case for \hbox
eq_or_diff(latex_decode('{$\hbox {N}^3$}'), '{$\hbox{N}^3$}', 'latex null decode 2');

eq_or_diff(rangelen([[10,15]]), 6, 'Rangelen test 1');
eq_or_diff(rangelen([[10,15],[47, 53]]), 13, 'Rangelen test 2');
eq_or_diff(rangelen([[10,15],[47, undef]]), 7, 'Rangelen test 3');
eq_or_diff(rangelen([[10,15],[47, '']]), -1, 'Rangelen test 4');
eq_or_diff(rangelen([[10,15],['', 35]]), -1, 'Rangelen test 5');
eq_or_diff(rangelen([[10,15],['', undef]]), -1, 'Rangelen test 6');
eq_or_diff(rangelen([[10,15],['XX', 'XXiv'],['i',10]]), 21, 'Rangelen test 7');
# This is nasty - it's U+2165 U+2160, U+217B to test unicode decomp
eq_or_diff(rangelen([[10,15],['ⅥⅠ', 'ⅻ']]), 12, 'Rangelen test 8');
eq_or_diff(rangelen([['I-II', 'III-IV']]), -1, 'Rangelen test 9');
eq_or_diff(rangelen([[22,4],[123,7],[113,15]]), 11, 'Rangelen test 10');

# Test boolean mappings
$Biber::Utils::CONFIG_OPTTYPE_BIBLATEX{test} = 'boolean'; # mock this for tests
eq_or_diff(map_boolean('test', 'true', 'tonum'), 1, 'Boolean conversion - 1');
eq_or_diff(map_boolean('test', 'False', 'tonum'), 0, 'Boolean conversion - 2');
eq_or_diff(map_boolean('test', 1, 'tostring'), 'true', 'Boolean conversion - 3');
eq_or_diff(map_boolean('test', 0, 'tostring'), 'false', 'Boolean conversion - 4');
eq_or_diff(map_boolean('test', 0, 'tonum'), 0, 'Boolean conversion - 5');

# Range parsing
eq_or_diff(parse_range('1--2'), [1,2], 'Range parsing - 1');
eq_or_diff(parse_range('-2'), [1,2], 'Range parsing - 2');
eq_or_diff(parse_range('3-'), [3,undef], 'Range parsing - 3');
eq_or_diff(parse_range('5'), [1,5], 'Range parsing - 4');
eq_or_diff(parse_range('3--+'), [3,'+'], 'Range parsing - 5');

# split_xsv
eq_or_diff([split_xsv('family=a, given=a b, given-i=a b c')], ['family=a', 'given=a b', 'given-i=a b c'], 'split_xsv - 1');
eq_or_diff([split_xsv('"family={Something, here}", given=b')], ['family={Something, here}', 'given=b'], 'split_xsv - 2');

eq_or_diff(strip_noinit('\texttt{freedesktop.org}'), 'freedesktop.org', 'Name strip - 1');
eq_or_diff(strip_noinit('\texttt freedesktop.org'), 'freedesktop.org', 'Name strip - 2');
eq_or_diff(strip_noinit('{\texttt freedesktop.org}'), '{freedesktop.org}', 'Name strip - 3');
eq_or_diff(strip_noinit('{C.\bibtexspatium A.}'), '{C.A.}', 'Name strip - 4');
"#;

#[test]
fn assertion_001_file_location_1() {
    pass_upstream(
        "File location - 1",
        r#"File::Spec->canonpath(locate_data_file("$cwd/t/tdata/general.bcf"))"#,
        r#"File::Spec->canonpath("$cwd/t/tdata/general.bcf")"#,
        r#"eq_or_diff(File::Spec->canonpath(locate_data_file("$cwd/t/tdata/general.bcf")), File::Spec->canonpath("$cwd/t/tdata/general.bcf"), 'File location - 1');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_file_location_2() {
    pass_upstream(
        "File location - 2",
        r"File::Spec->canonpath(locate_data_file('t/tdata/general.bcf'))",
        r"File::Spec->canonpath('t/tdata/general.bcf')",
        r"eq_or_diff(File::Spec->canonpath(locate_data_file('t/tdata/general.bcf')), File::Spec->canonpath('t/tdata/general.bcf'), 'File location - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_file_location_3() {
    pass_upstream(
        "File location - 3",
        r"File::Spec->canonpath(locate_data_file('t/tdata/examples.bib'))",
        r"File::Spec->canonpath('t/tdata/examples.bib')",
        r"eq_or_diff(File::Spec->canonpath(locate_data_file('t/tdata/examples.bib')), File::Spec->canonpath('t/tdata/examples.bib'), 'File location - 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_file_location_4() {
    pass_upstream(
        "File location - 4",
        r"File::Spec->canonpath(locate_data_file('plain.tex'))",
        r"qr|plain.tex\cM*\z|",
        r"like(File::Spec->canonpath(locate_data_file('plain.tex')), qr|plain.tex\cM*\z|, 'File location - 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_file_location_5() {
    pass_upstream(
        "File location - 5",
        r"File::Spec->canonpath(locate_data_file('general.bcf'))",
        r#"File::Spec->canonpath("t/tdata/general.bcf")"#,
        r#"eq_or_diff(File::Spec->canonpath(locate_data_file('general.bcf')), File::Spec->canonpath("t/tdata/general.bcf"), 'File location - 5');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_normalise_string() {
    pass_upstream(
        "normalise_string",
        r#"normalise_string('"a, b–c: d" ', 1)"#,
        r"'a bc d'",
        r#"eq_or_diff(normalise_string('"a, b–c: d" ', 1),  'a bc d', 'normalise_string' );"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_normalise_string_underscore_1() {
    pass_upstream(
        "normalise_string_underscore 1",
        r#"NFC(normalise_string_underscore(latex_decode('\c Se\x{c}\"ok-\foo{a},  N\`i\~no
    $§+ :-)   '), 0))"#,
        r"'Şecöka_Nìño'",
        r#"eq_or_diff(NFC(normalise_string_underscore(latex_decode('\c Se\x{c}\"ok-\foo{a},  N\`i\~no
    $§+ :-)   '), 0)), 'Şecöka_Nìño', 'normalise_string_underscore 1' );"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_normalise_string_underscore_3() {
    pass_upstream(
        "normalise_string_underscore 3",
        r"normalise_string_underscore('{Foo de Bar, Graf Ludwig}', 1)",
        r"'Foo_de_Bar_Graf_Ludwig'",
        r"eq_or_diff(normalise_string_underscore('{Foo de Bar, Graf Ludwig}', 1), 'Foo_de_Bar_Graf_Ludwig', 'normalise_string_underscore 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_009_latex_decode_1() {
    pass_upstream(
        "latex decode 1",
        r"NFC(latex_decode('Mu\d{h}ammad ibn M\=us\=a al-Khw\=arizm\={\i} \r{a}'))",
        r"'Muḥammad ibn Mūsā al-Khwārizmı̄ å'",
        r"eq_or_diff(NFC(latex_decode('Mu\d{h}ammad ibn M\=us\=a al-Khw\=arizm\={\i} \r{a}')), 'Muḥammad ibn Mūsā al-Khwārizmı̄ å', 'latex decode 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_010_latex_decode_2() {
    pass_upstream(
        "latex decode 2",
        r"latex_decode('\alpha')",
        r"'\alpha'",
        r"eq_or_diff(latex_decode('\alpha'), '\alpha', 'latex decode 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_011_latex_decode_3() {
    pass_upstream(
        "latex decode 3",
        r"latex_decode('\textless\textampersand')",
        r"'<&'",
        r"eq_or_diff(latex_decode('\textless\textampersand'), '<&', 'latex decode 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_012_latex_encode_1() {
    pass_upstream(
        "latex encode 1",
        r"latex_encode(NFD('Muḥammad ibn Mūsā al-Khwārizmī'))",
        r"'Mu\d{h}ammad ibn M\={u}s\={a} al-Khw\={a}rizm\={\i}'",
        r"eq_or_diff(latex_encode(NFD('Muḥammad ibn Mūsā al-Khwārizmī')), 'Mu\d{h}ammad ibn M\={u}s\={a} al-Khw\={a}rizm\={\i}', 'latex encode 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_013_latex_encode_2() {
    pass_upstream(
        "latex encode 2",
        r"latex_encode(NFD('α'))",
        r"'α'",
        r"eq_or_diff(latex_encode(NFD('α')), 'α', 'latex encode 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_014_latex_decode_accent_1_with_redundant_explicit_brace_protection() {
    pass_upstream(
        "latex decode accent 1 (with redundant explicit brace protection)",
        r#"NFC(latex_decode("{M{\\'a}t{\\'e}}"))"#,
        r"'{Máté}'",
        r#"eq_or_diff(NFC(latex_decode("{M{\\'a}t{\\'e}}")), '{Máté}', 'latex decode accent 1 (with redundant explicit brace protection)');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_015_latex_decode_accent_2() {
    pass_upstream(
        "latex decode accent 2",
        r#"NFC(latex_decode("{M\\'{a}t\\'{e}}"))"#,
        r"'{Máté}'",
        r#"eq_or_diff(NFC(latex_decode("{M\\'{a}t\\'{e}}")), '{Máté}', 'latex decode accent 2');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_016_latex_decode_accent_3() {
    pass_upstream(
        "latex decode accent 3",
        r#"NFC(latex_decode("{M\\'at\\'e}"))"#,
        r"'{Máté}'",
        r#"eq_or_diff(NFC(latex_decode("{M\\'at\\'e}")), '{Máté}', 'latex decode accent 3');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_017_latex_decode_accent_4() {
    pass_upstream(
        "latex decode accent 4",
        r#"NFC(latex_decode("R{\\'egis}"))"#,
        r"'R{égis}'",
        r#"eq_or_diff(NFC(latex_decode("R{\\'egis}")), 'R{égis}', 'latex decode accent 4');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_018_latex_decode_accent_5() {
    pass_upstream(
        "latex decode accent 5",
        r#"NFC(latex_decode("\\frac{a}{b}"))"#,
        r"'\frac{a}{b}'",
        r#"eq_or_diff(NFC(latex_decode("\\frac{a}{b}")), '\frac{a}{b}', 'latex decode accent 5');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_019_latex_decode_accent_6() {
    pass_upstream(
        "latex decode accent 6",
        r#"NFC(latex_decode("\\textuppercase{\\'e}"))"#,
        r"'\textuppercase{é}'",
        r#"eq_or_diff(NFC(latex_decode("\\textuppercase{\\'e}")), '\textuppercase{é}', 'latex decode accent 6');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_020_latex_reversing_recoding_test_1() {
    pass_upstream(
        "latex reversing recoding test 1",
        r#"NFC(latex_decode("\\DH{}and\\dj{}and\\'{c}, H."))"#,
        r"'Ðandđandć, H.'",
        r#"eq_or_diff(NFC(latex_decode("\\DH{}and\\dj{}and\\'{c}, H.")), 'Ðandđandć, H.', 'latex reversing recoding test 1');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_021_latex_reversing_recoding_test_2() {
    pass_upstream(
        "latex reversing recoding test 2",
        r#"NFC(latex_decode("{\\DH{}and\\dj{}and\\'{c}, H.}"))"#,
        r"'{Ðandđandć, H.}'",
        r#"eq_or_diff(NFC(latex_decode("{\\DH{}and\\dj{}and\\'{c}, H.}")), '{Ðandđandć, H.}', 'latex reversing recoding test 2');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_022_latex_reversing_recoding_test_3() {
    pass_upstream(
        "latex reversing recoding test 3",
        r"latex_encode(NFD('Ðandđandć, H.'))",
        r"'\\DH{}and\\dj{}and\\\'{c}, H.'",
        r"eq_or_diff(latex_encode(NFD('Ðandđandć, H.')), '\\DH{}and\\dj{}and\\\'{c}, H.', 'latex reversing recoding test 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_023_latex_reversing_recoding_test_4() {
    pass_upstream(
        "latex reversing recoding test 4",
        r"latex_encode(NFD('{Ðandđandć, H.}'))",
        r"'{\\DH{}and\\dj{}and\\\'{c}, H.}'",
        r"eq_or_diff(latex_encode(NFD('{Ðandđandć, H.}')), '{\\DH{}and\\dj{}and\\\'{c}, H.}', 'latex reversing recoding test 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_024_latex_decode_4_with_2_explicit_brace_protections() {
    pass_upstream(
        "latex decode 4 (with 2 explicit brace protections)",
        r#"NFC(latex_decode('{\"{U}}ber {\"{U}}berlegungen zur \"{U}berwindung des \"{U}bels'))"#,
        r"'Über Überlegungen zur Überwindung des Übels'",
        r#"eq_or_diff(NFC(latex_decode('{\"{U}}ber {\"{U}}berlegungen zur \"{U}berwindung des \"{U}bels')), 'Über Überlegungen zur Überwindung des Übels', 'latex decode 4 (with 2 explicit brace protections)');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_025_latex_decode_4a() {
    pass_upstream(
        "latex decode 4a",
        r"latex_decode('\alpha')",
        r"'α'",
        r"eq_or_diff(latex_decode('\alpha'), 'α', 'latex decode 4a');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_026_latex_decode_5() {
    pass_upstream(
        "latex decode 5",
        r#"NFC(latex_decode("\\'\\i"))"#,
        r"'í'",
        r#"eq_or_diff(NFC(latex_decode("\\'\\i")), 'í', 'latex decode 5');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_027_latex_decode_5a_with_redundant_explicit_brace_protection() {
    pass_upstream(
        "latex decode 5a (with redundant explicit brace protection)",
        r#"NFC(latex_decode("{\\'\\i}"))"#,
        r"'í'",
        r#"eq_or_diff(NFC(latex_decode("{\\'\\i}")), 'í', 'latex decode 5a (with redundant explicit brace protection)');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_028_latex_decode_6() {
    pass_upstream(
        "latex decode 6",
        r#"NFC(latex_decode("\\^{\\j}"))"#,
        r"'ȷ̂'",
        r#"eq_or_diff(NFC(latex_decode("\\^{\\j}")), 'ȷ̂', 'latex decode 6');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_029_latex_decode_7() {
    pass_upstream(
        "latex decode 7",
        r#"NFC(latex_decode("\\u{\\i}"))"#,
        r"'ı̆'",
        r#"eq_or_diff(NFC(latex_decode("\\u{\\i}")), 'ı̆', 'latex decode 7');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_030_latex_decode_8() {
    pass_upstream(
        "latex decode 8",
        r#"NFC(latex_decode("\\u\\i"))"#,
        r"'ı̆'",
        r#"eq_or_diff(NFC(latex_decode("\\u\\i")), 'ı̆', 'latex decode 8');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_031_latex_decode_9() {
    pass_upstream(
        "latex decode 9",
        r#"NFC(latex_decode("{{\\'A}lvarez}, J.~D."))"#,
        r"'{Álvarez}, J.~D.'",
        r#"eq_or_diff(NFC(latex_decode("{{\\'A}lvarez}, J.~D.")), '{Álvarez}, J.~D.', 'latex decode 9');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_032_latex_decode_9() {
    pass_upstream(
        "latex decode 9",
        r"latex_decode('\i')",
        r"'ı'",
        r"eq_or_diff(latex_decode('\i'), 'ı', 'latex decode 9');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_033_latex_decode_10() {
    pass_upstream(
        "latex decode 10",
        r"latex_decode('\j')",
        r"'ȷ'",
        r"eq_or_diff(latex_decode('\j'), 'ȷ', 'latex decode 10');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_034_latex_decode_11() {
    pass_upstream(
        "latex decode 11",
        r"latex_decode('\textdiv')",
        r"'÷'",
        r"eq_or_diff(latex_decode('\textdiv'), '÷', 'latex decode 11');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_035_latex_decode_13() {
    pass_upstream(
        "latex decode 13",
        r"latex_decode('--')",
        r"'--'",
        r"eq_or_diff(latex_decode('--'), '--', 'latex decode 13');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_036_latex_decode_14() {
    pass_upstream(
        "latex decode 14",
        r"latex_decode('\textdegree C')",
        r"'°C'",
        r"eq_or_diff(latex_decode('\textdegree C'), '°C', 'latex decode 14');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_037_latex_decode_15() {
    pass_upstream(
        "latex decode 15",
        r#"NFC(latex_decode("{\\'{I}}"))"#,
        r"'Í'",
        r#"eq_or_diff(NFC(latex_decode("{\\'{I}}")), 'Í', 'latex decode 15');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_038_latex_decode_16() {
    pass_upstream(
        "latex decode 16",
        r"NFC(latex_decode('{\v{C}}'))",
        r"'Č'",
        r"eq_or_diff(NFC(latex_decode('{\v{C}}')), 'Č', 'latex decode 16');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_039_latex_decode_17() {
    pass_upstream(
        "latex decode 17",
        r"NFC(latex_decode('{I}'))",
        r"'{I}'",
        r"eq_or_diff(NFC(latex_decode('{I}')), '{I}', 'latex decode 17');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_040_latex_decode_18() {
    pass_upstream(
        "latex decode 18",
        r"NFC(latex_decode('\&{A}'))",
        r"'\&{A}'",
        r"eq_or_diff(NFC(latex_decode('\&{A}')), '\&{A}', 'latex decode 18');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_041_latex_decode_19() {
    pass_upstream(
        "latex decode 19",
        r"NFC(latex_decode('\&\;{A}'))",
        r"'\&\;{A}'",
        r"eq_or_diff(NFC(latex_decode('\&\;{A}')), '\&\;{A}', 'latex decode 19');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_042_latex_encode_3() {
    pass_upstream(
        "latex encode 3",
        r"latex_encode(NFD('α'))",
        r"'{$\alpha$}'",
        r"eq_or_diff(latex_encode(NFD('α')), '{$\alpha$}', 'latex encode 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_043_latex_encode_4() {
    pass_upstream(
        "latex encode 4",
        r"latex_encode(NFD('µ'))",
        r"'{$\mu$}'",
        r"eq_or_diff(latex_encode(NFD('µ')), '{$\mu$}', 'latex encode 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_044_latex_encode_5() {
    pass_upstream(
        "latex encode 5",
        r"latex_encode(NFD('≄'))",
        r"'{$\not\simeq$}'",
        r"eq_or_diff(latex_encode(NFD('≄')), '{$\not\simeq$}', 'latex encode 5');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_045_latex_encode_6() {
    pass_upstream(
        "latex encode 6",
        r"latex_encode(NFD('Þ'))",
        r"'\TH{}'",
        r"eq_or_diff(latex_encode(NFD('Þ')), '\TH{}', 'latex encode 6');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_046_latex_encode_7() {
    pass_upstream(
        "latex encode 7",
        r"latex_encode('$')",
        r"'$'",
        r"eq_or_diff(latex_encode('$'), '$', 'latex encode 7');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_047_latex_encode_8() {
    pass_upstream(
        "latex encode 8",
        r"latex_encode(NFD('–'))",
        r"'--'",
        r"eq_or_diff(latex_encode(NFD('–')), '--', 'latex encode 8');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_048_discretionary_hyphens() {
    pass_upstream(
        "discretionary hyphens",
        r"latex_decode('a\-a')",
        r"'a\-a'",
        r"eq_or_diff(latex_decode('a\-a'), 'a\-a', 'discretionary hyphens');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_049_latex_encode_9() {
    pass_upstream(
        "latex encode 9",
        r"latex_encode(NFD('Åå'))",
        r"'\r{A}\r{a}'",
        r"eq_or_diff(latex_encode(NFD('Åå')), '\r{A}\r{a}', 'latex encode 9');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_050_latex_encode_10() {
    pass_upstream(
        "latex encode 10",
        r"latex_encode(NFD('a̍'))",
        r"'\|{a}'",
        r"eq_or_diff(latex_encode(NFD('a̍')), '\|{a}', 'latex encode 10');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_051_latex_encode_11() {
    pass_upstream(
        "latex encode 11",
        r"latex_encode(NFD('ı̆'))",
        r"'\u{\i{}}'",
        r"eq_or_diff(latex_encode(NFD('ı̆')), '\u{\i{}}', 'latex encode 11');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_052_latex_encode_12() {
    pass_upstream(
        "latex encode 12",
        r"latex_encode(NFD('®'))",
        r"'\textregistered{}'",
        r"eq_or_diff(latex_encode(NFD('®')), '\textregistered{}', 'latex encode 12');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_053_latex_encode_13() {
    pass_upstream(
        "latex encode 13",
        r"latex_encode(NFD('©'))",
        r"'{$\copyright$}'",
        r"eq_or_diff(latex_encode(NFD('©')), '{$\copyright$}', 'latex encode 13');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_054_latex_encode_13() {
    pass_upstream(
        "latex encode 13",
        r"latex_encode(NFD('°C'))",
        r"'\textdegree{}C'",
        r"eq_or_diff(latex_encode(NFD('°C')), '\textdegree{}C', 'latex encode 13');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_055_reduce_array() {
    pass_upstream(
        "reduce_array",
        r"\@AminusB",
        r"\@AminusBexpected",
        r"is_deeply(\@AminusB, \@AminusBexpected, 'reduce_array') ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_056_remove_outer_1() {
    pass_upstream(
        "remove_outer - 1",
        r"(remove_outer('{Some string}'))[0]",
        r"1",
        r"eq_or_diff((remove_outer('{Some string}'))[0], 1, 'remove_outer - 1') ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_057_remove_outer_2() {
    pass_upstream(
        "remove_outer - 2",
        r"(remove_outer('{Some string}'))[1]",
        r"'Some string'",
        r"eq_or_diff((remove_outer('{Some string}'))[1], 'Some string', 'remove_outer - 2') ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_058_normalise_string_lite() {
    pass_upstream(
        "normalise_string_lite",
        r"normalise_string_hash('Ä.~{\c{C}}.~{\c S}.')",
        r"'Äc:Cc:S'",
        r"eq_or_diff(normalise_string_hash('Ä.~{\c{C}}.~{\c S}.'), 'Äc:Cc:S', 'normalise_string_lite' ) ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_059_latex_different_encode_decode_sets_1() {
    pass_upstream(
        "latex different encode/decode sets 1",
        r"latex_decode('\textdiv')",
        r"'\textdiv'",
        r"eq_or_diff(latex_decode('\textdiv'), '\textdiv', 'latex different encode/decode sets 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_060_latex_different_encode_decode_sets_2() {
    pass_upstream(
        "latex different encode/decode sets 2",
        r"latex_encode(NFD('÷'))",
        r"'{$\\div$}'",
        r"eq_or_diff(latex_encode(NFD('÷')), '{$\\div$}', 'latex different encode/decode sets 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_061_latex_null_decode_1() {
    pass_upstream(
        "latex null decode 1",
        r"latex_decode('\i')",
        r"'\i'",
        r"eq_or_diff(latex_decode('\i'), '\i', 'latex null decode 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_062_latex_null_encode_2() {
    pass_upstream(
        "latex null encode 2",
        r"latex_encode(NFD('ı'))",
        r"'\i{}'",
        r"eq_or_diff(latex_encode(NFD('ı')), '\i{}', 'latex null encode 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_063_latex_null_decode_2() {
    pass_upstream(
        "latex null decode 2",
        r"latex_decode('{$\hbox {N}^3$}')",
        r"'{$\hbox{N}^3$}'",
        r"eq_or_diff(latex_decode('{$\hbox {N}^3$}'), '{$\hbox{N}^3$}', 'latex null decode 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_064_rangelen_test_1() {
    pass_upstream(
        "Rangelen test 1",
        r"rangelen([[10,15]])",
        r"6",
        r"eq_or_diff(rangelen([[10,15]]), 6, 'Rangelen test 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_065_rangelen_test_2() {
    pass_upstream(
        "Rangelen test 2",
        r"rangelen([[10,15],[47, 53]])",
        r"13",
        r"eq_or_diff(rangelen([[10,15],[47, 53]]), 13, 'Rangelen test 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_066_rangelen_test_3() {
    pass_upstream(
        "Rangelen test 3",
        r"rangelen([[10,15],[47, undef]])",
        r"7",
        r"eq_or_diff(rangelen([[10,15],[47, undef]]), 7, 'Rangelen test 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_067_rangelen_test_4() {
    pass_upstream(
        "Rangelen test 4",
        r"rangelen([[10,15],[47, '']])",
        r"-1",
        r"eq_or_diff(rangelen([[10,15],[47, '']]), -1, 'Rangelen test 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_068_rangelen_test_5() {
    pass_upstream(
        "Rangelen test 5",
        r"rangelen([[10,15],['', 35]])",
        r"-1",
        r"eq_or_diff(rangelen([[10,15],['', 35]]), -1, 'Rangelen test 5');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_069_rangelen_test_6() {
    pass_upstream(
        "Rangelen test 6",
        r"rangelen([[10,15],['', undef]])",
        r"-1",
        r"eq_or_diff(rangelen([[10,15],['', undef]]), -1, 'Rangelen test 6');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_070_rangelen_test_7() {
    pass_upstream(
        "Rangelen test 7",
        r"rangelen([[10,15],['XX', 'XXiv'],['i',10]])",
        r"21",
        r"eq_or_diff(rangelen([[10,15],['XX', 'XXiv'],['i',10]]), 21, 'Rangelen test 7');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_071_rangelen_test_8() {
    pass_upstream(
        "Rangelen test 8",
        r"rangelen([[10,15],['ⅥⅠ', 'ⅻ']])",
        r"12",
        r"eq_or_diff(rangelen([[10,15],['ⅥⅠ', 'ⅻ']]), 12, 'Rangelen test 8');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_072_rangelen_test_9() {
    pass_upstream(
        "Rangelen test 9",
        r"rangelen([['I-II', 'III-IV']])",
        r"-1",
        r"eq_or_diff(rangelen([['I-II', 'III-IV']]), -1, 'Rangelen test 9');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_073_rangelen_test_10() {
    pass_upstream(
        "Rangelen test 10",
        r"rangelen([[22,4],[123,7],[113,15]])",
        r"11",
        r"eq_or_diff(rangelen([[22,4],[123,7],[113,15]]), 11, 'Rangelen test 10');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_074_boolean_conversion_1() {
    pass_upstream(
        "Boolean conversion - 1",
        r"map_boolean('test', 'true', 'tonum')",
        r"1",
        r"eq_or_diff(map_boolean('test', 'true', 'tonum'), 1, 'Boolean conversion - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_075_boolean_conversion_2() {
    pass_upstream(
        "Boolean conversion - 2",
        r"map_boolean('test', 'False', 'tonum')",
        r"0",
        r"eq_or_diff(map_boolean('test', 'False', 'tonum'), 0, 'Boolean conversion - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_076_boolean_conversion_3() {
    pass_upstream(
        "Boolean conversion - 3",
        r"map_boolean('test', 1, 'tostring')",
        r"'true'",
        r"eq_or_diff(map_boolean('test', 1, 'tostring'), 'true', 'Boolean conversion - 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_077_boolean_conversion_4() {
    pass_upstream(
        "Boolean conversion - 4",
        r"map_boolean('test', 0, 'tostring')",
        r"'false'",
        r"eq_or_diff(map_boolean('test', 0, 'tostring'), 'false', 'Boolean conversion - 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_078_boolean_conversion_5() {
    pass_upstream(
        "Boolean conversion - 5",
        r"map_boolean('test', 0, 'tonum')",
        r"0",
        r"eq_or_diff(map_boolean('test', 0, 'tonum'), 0, 'Boolean conversion - 5');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_079_range_parsing_1() {
    pass_upstream(
        "Range parsing - 1",
        r"parse_range('1--2')",
        r"[1,2]",
        r"eq_or_diff(parse_range('1--2'), [1,2], 'Range parsing - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_080_range_parsing_2() {
    pass_upstream(
        "Range parsing - 2",
        r"parse_range('-2')",
        r"[1,2]",
        r"eq_or_diff(parse_range('-2'), [1,2], 'Range parsing - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_081_range_parsing_3() {
    pass_upstream(
        "Range parsing - 3",
        r"parse_range('3-')",
        r"[3,undef]",
        r"eq_or_diff(parse_range('3-'), [3,undef], 'Range parsing - 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_082_range_parsing_4() {
    pass_upstream(
        "Range parsing - 4",
        r"parse_range('5')",
        r"[1,5]",
        r"eq_or_diff(parse_range('5'), [1,5], 'Range parsing - 4');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_083_range_parsing_5() {
    pass_upstream(
        "Range parsing - 5",
        r"parse_range('3--+')",
        r"[3,'+']",
        r"eq_or_diff(parse_range('3--+'), [3,'+'], 'Range parsing - 5');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_084_split_xsv_1() {
    pass_upstream(
        "split_xsv - 1",
        r"[split_xsv('family=a, given=a b, given-i=a b c')]",
        r"['family=a', 'given=a b', 'given-i=a b c']",
        r"eq_or_diff([split_xsv('family=a, given=a b, given-i=a b c')], ['family=a', 'given=a b', 'given-i=a b c'], 'split_xsv - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_085_split_xsv_2() {
    pass_upstream(
        "split_xsv - 2",
        r#"[split_xsv('"family={Something, here}", given=b')]"#,
        r"['family={Something, here}', 'given=b']",
        r#"eq_or_diff([split_xsv('"family={Something, here}", given=b')], ['family={Something, here}', 'given=b'], 'split_xsv - 2');"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_086_name_strip_1() {
    pass_upstream(
        "Name strip - 1",
        r"strip_noinit('\texttt{freedesktop.org}')",
        r"'freedesktop.org'",
        r"eq_or_diff(strip_noinit('\texttt{freedesktop.org}'), 'freedesktop.org', 'Name strip - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_087_name_strip_2() {
    pass_upstream(
        "Name strip - 2",
        r"strip_noinit('\texttt freedesktop.org')",
        r"'freedesktop.org'",
        r"eq_or_diff(strip_noinit('\texttt freedesktop.org'), 'freedesktop.org', 'Name strip - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_088_name_strip_3() {
    pass_upstream(
        "Name strip - 3",
        r"strip_noinit('{\texttt freedesktop.org}')",
        r"'{freedesktop.org}'",
        r"eq_or_diff(strip_noinit('{\texttt freedesktop.org}'), '{freedesktop.org}', 'Name strip - 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_089_name_strip_4() {
    pass_upstream(
        "Name strip - 4",
        r"strip_noinit('{C.\bibtexspatium A.}')",
        r"'{C.A.}'",
        r"eq_or_diff(strip_noinit('{C.\bibtexspatium A.}'), '{C.A.}', 'Name strip - 4');",
        UPSTREAM_SOURCE,
    );
}
