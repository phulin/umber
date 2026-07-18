// Direct mixed-stage translation of upstream t/bibtex-aliases.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::{SemanticOwner, compare_owned_upstream};

// Alias processing belongs to the graph stage; the structured-name assertion
// additionally needs the name stage and is therefore owned by that later issue.
fn compare_upstream(
    assertion: &str,
    actual_expression: &str,
    expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    let owner = match assertion {
        "Alias - 16" => SemanticOwner::Names,
        "Alias - 1" | "Alias - 2" | "Alias - 3" | "Alias - 4" | "Alias - 5" | "Alias - 6"
        | "Alias - 7" | "Alias - 8" | "Alias - 9" | "Alias - 10" | "Alias - 11" | "Alias - 12"
        | "Alias - 13" | "Alias - 14" | "Alias - 15" | "Alias - 17" | "Alias - 18"
        | "Alias - 19" | "Alias - 20" | "Alias - 21" | "Alias - 22" | "Alias - 23"
        | "Alias - 24" | "Alias - 25" => SemanticOwner::Graph,
        _ => panic!("mixed-stage assertion `{assertion}` has no semantic owner"),
    };
    compare_owned_upstream(
        owner,
        assertion,
        actual_expression,
        expected_expression,
        upstream_call,
        upstream_source,
    );
    panic!("xfail: alias-stage compatibility behavior is not exposed by bib-engine");
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 25;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Utils;
use Biber::Output::bbl;
use Log::Log4perl;
chdir("t/tdata") ;

# Set up Biber object
my $biber = Biber->new( noconf => 1);

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

$biber->parse_ctrlfile('bibtex-aliases.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
Biber::Config->setoption('validate_datamodel', 1);

# THERE IS A MAPPING SECTION IN THE .bcf BEING USED TO TEST USER MAPS TOO!

# Now generate the information
$biber->prepare;

my $section = $biber->sections->get_section(0);
my $bibentries = $section->bibentries;

my $w1 = ["Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Field 'school' invalid in data model - ignoring",
          "Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Invalid entry type 'thing' - defaulting to 'misc'",
          "Datamodel: thing entry 'alias2' (bibtex-aliases.bib): Invalid field 'institution' for entrytype 'misc'",
];

my $w2 = ["Datamodel: customa entry 'alias4' (bibtex-aliases.bib): Invalid field 'author' for entrytype 'customa'",
          "Datamodel: customa entry 'alias4' (bibtex-aliases.bib): Invalid field 'title' for entrytype 'customa'",
];

eq_or_diff($bibentries->entry('alias1')->get_field('entrytype'), 'thesis', 'Alias - 1' );
eq_or_diff($bibentries->entry('alias1')->get_field('type'), 'phdthesis', 'Alias - 2' );
is_deeply($bibentries->entry('alias1')->get_field('location'), ['Ivory Towers'], 'Alias - 3' );
eq_or_diff($bibentries->entry('alias1')->get_field('address'), undef, 'Alias - 4' );
eq_or_diff($bibentries->entry('alias2')->get_field('entrytype'), 'misc', 'Alias - 5' );
is_deeply($bibentries->entry('alias2')->get_field('warnings'), $w1, 'Alias - 6' ) ;
eq_or_diff($bibentries->entry('alias2')->get_field('school'), undef, 'Alias - 7' );
eq_or_diff($bibentries->entry('alias3')->get_field('entrytype'), 'customb', 'Alias - 8' );
eq_or_diff($bibentries->entry('alias4')->get_field('entrytype'), 'customa', 'Alias - 9' );
eq_or_diff($bibentries->entry('alias4')->get_field('verba'), 'conversation', 'Alias - 10' );
eq_or_diff($bibentries->entry('alias4')->get_field('verbb'), 'somevalue', 'Alias - 11' );
eq_or_diff($bibentries->entry('alias4')->get_field('eprint'), 'anid', 'Alias - 12' );
eq_or_diff($bibentries->entry('alias4')->get_field('eprinttype'), 'pubmedid', 'Alias - 13' );
eq_or_diff($bibentries->entry('alias4')->get_field('userd'), 'Some string of things', 'Alias - 14' );
eq_or_diff($bibentries->entry('alias4')->get_field('pubmedid'), undef, 'Alias - 15' );
eq_or_diff($bibentries->entry('alias4')->get_field('namea')->nth_name(1)->get_namepart('given'), 'Sam', 'Alias - 16' );
is_deeply($bibentries->entry('alias4')->get_field('warnings'), $w2, 'Alias - 17' ) ;

# Testing of .bcf field map match/replace
ok(is_undef($bibentries->entry('alias5')->get_field('abstract')), 'Alias - 18' );
eq_or_diff($biber->_liststring('alias5', 'listb'), 'REPlaCEDte!early', 'Alias - 19');
eq_or_diff($biber->_liststring('alias5', 'institution'), 'REPlaCEDte!early', 'Alias - 20');

# Testing of no target but just field additions
is_deeply($bibentries->entry('alias6')->get_field('keywords'), ['keyw1', 'keyw2'], 'Alias - 21' );

# Testing of no regexp match for field value
is_deeply($bibentries->entry('alias7')->get_field('lista'), ['listaval'], 'Alias - 22' );

# Testing append overwrites
eq_or_diff($bibentries->entry('alias7')->get_field('verbb'), 'val2val1', 'Alias - 23' );
eq_or_diff($bibentries->entry('alias7')->get_field('verbc'), 'val3val2val1', 'Alias - 24' );

# Testing appendstrict
ok(is_undef($bibentries->entry('alias8')->get_field('verbc')), 'Alias - 25' );
"#;

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_001_alias_1() {
    compare_upstream(
        "Alias - 1",
        r"$bibentries->entry('alias1')->get_field('entrytype')",
        r"'thesis'",
        r"eq_or_diff($bibentries->entry('alias1')->get_field('entrytype'), 'thesis', 'Alias - 1' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_002_alias_2() {
    compare_upstream(
        "Alias - 2",
        r"$bibentries->entry('alias1')->get_field('type')",
        r"'phdthesis'",
        r"eq_or_diff($bibentries->entry('alias1')->get_field('type'), 'phdthesis', 'Alias - 2' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_003_alias_3() {
    compare_upstream(
        "Alias - 3",
        r"$bibentries->entry('alias1')->get_field('location')",
        r"['Ivory Towers']",
        r"is_deeply($bibentries->entry('alias1')->get_field('location'), ['Ivory Towers'], 'Alias - 3' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_004_alias_4() {
    compare_upstream(
        "Alias - 4",
        r"$bibentries->entry('alias1')->get_field('address')",
        r"undef",
        r"eq_or_diff($bibentries->entry('alias1')->get_field('address'), undef, 'Alias - 4' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_005_alias_5() {
    compare_upstream(
        "Alias - 5",
        r"$bibentries->entry('alias2')->get_field('entrytype')",
        r"'misc'",
        r"eq_or_diff($bibentries->entry('alias2')->get_field('entrytype'), 'misc', 'Alias - 5' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_006_alias_6() {
    compare_upstream(
        "Alias - 6",
        r"$bibentries->entry('alias2')->get_field('warnings')",
        r"$w1",
        r"is_deeply($bibentries->entry('alias2')->get_field('warnings'), $w1, 'Alias - 6' ) ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_007_alias_7() {
    compare_upstream(
        "Alias - 7",
        r"$bibentries->entry('alias2')->get_field('school')",
        r"undef",
        r"eq_or_diff($bibentries->entry('alias2')->get_field('school'), undef, 'Alias - 7' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_008_alias_8() {
    compare_upstream(
        "Alias - 8",
        r"$bibentries->entry('alias3')->get_field('entrytype')",
        r"'customb'",
        r"eq_or_diff($bibentries->entry('alias3')->get_field('entrytype'), 'customb', 'Alias - 8' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_009_alias_9() {
    compare_upstream(
        "Alias - 9",
        r"$bibentries->entry('alias4')->get_field('entrytype')",
        r"'customa'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('entrytype'), 'customa', 'Alias - 9' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_010_alias_10() {
    compare_upstream(
        "Alias - 10",
        r"$bibentries->entry('alias4')->get_field('verba')",
        r"'conversation'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('verba'), 'conversation', 'Alias - 10' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_011_alias_11() {
    compare_upstream(
        "Alias - 11",
        r"$bibentries->entry('alias4')->get_field('verbb')",
        r"'somevalue'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('verbb'), 'somevalue', 'Alias - 11' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_012_alias_12() {
    compare_upstream(
        "Alias - 12",
        r"$bibentries->entry('alias4')->get_field('eprint')",
        r"'anid'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('eprint'), 'anid', 'Alias - 12' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_013_alias_13() {
    compare_upstream(
        "Alias - 13",
        r"$bibentries->entry('alias4')->get_field('eprinttype')",
        r"'pubmedid'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('eprinttype'), 'pubmedid', 'Alias - 13' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_014_alias_14() {
    compare_upstream(
        "Alias - 14",
        r"$bibentries->entry('alias4')->get_field('userd')",
        r"'Some string of things'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('userd'), 'Some string of things', 'Alias - 14' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_015_alias_15() {
    compare_upstream(
        "Alias - 15",
        r"$bibentries->entry('alias4')->get_field('pubmedid')",
        r"undef",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('pubmedid'), undef, 'Alias - 15' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_016_alias_16() {
    compare_upstream(
        "Alias - 16",
        r"$bibentries->entry('alias4')->get_field('namea')->nth_name(1)->get_namepart('given')",
        r"'Sam'",
        r"eq_or_diff($bibentries->entry('alias4')->get_field('namea')->nth_name(1)->get_namepart('given'), 'Sam', 'Alias - 16' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_017_alias_17() {
    compare_upstream(
        "Alias - 17",
        r"$bibentries->entry('alias4')->get_field('warnings')",
        r"$w2",
        r"is_deeply($bibentries->entry('alias4')->get_field('warnings'), $w2, 'Alias - 17' ) ;",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_018_alias_18() {
    compare_upstream(
        "Alias - 18",
        r"is_undef($bibentries->entry('alias5')->get_field('abstract'))",
        r"true",
        r"ok(is_undef($bibentries->entry('alias5')->get_field('abstract')), 'Alias - 18' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_019_alias_19() {
    compare_upstream(
        "Alias - 19",
        r"$biber->_liststring('alias5', 'listb')",
        r"'REPlaCEDte!early'",
        r"eq_or_diff($biber->_liststring('alias5', 'listb'), 'REPlaCEDte!early', 'Alias - 19');",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_020_alias_20() {
    compare_upstream(
        "Alias - 20",
        r"$biber->_liststring('alias5', 'institution')",
        r"'REPlaCEDte!early'",
        r"eq_or_diff($biber->_liststring('alias5', 'institution'), 'REPlaCEDte!early', 'Alias - 20');",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_021_alias_21() {
    compare_upstream(
        "Alias - 21",
        r"$bibentries->entry('alias6')->get_field('keywords')",
        r"['keyw1', 'keyw2']",
        r"is_deeply($bibentries->entry('alias6')->get_field('keywords'), ['keyw1', 'keyw2'], 'Alias - 21' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_022_alias_22() {
    compare_upstream(
        "Alias - 22",
        r"$bibentries->entry('alias7')->get_field('lista')",
        r"['listaval']",
        r"is_deeply($bibentries->entry('alias7')->get_field('lista'), ['listaval'], 'Alias - 22' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_023_alias_23() {
    compare_upstream(
        "Alias - 23",
        r"$bibentries->entry('alias7')->get_field('verbb')",
        r"'val2val1'",
        r"eq_or_diff($bibentries->entry('alias7')->get_field('verbb'), 'val2val1', 'Alias - 23' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_024_alias_24() {
    compare_upstream(
        "Alias - 24",
        r"$bibentries->entry('alias7')->get_field('verbc')",
        r"'val3val2val1'",
        r"eq_or_diff($bibentries->entry('alias7')->get_field('verbc'), 'val3val2val1', 'Alias - 24' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: exact upstream end-to-end behavior is not exposed by the public Rust API"]
fn assertion_025_alias_25() {
    compare_upstream(
        "Alias - 25",
        r"is_undef($bibentries->entry('alias8')->get_field('verbc'))",
        r"true",
        r"ok(is_undef($bibentries->entry('alias8')->get_field('verbc')), 'Alias - 25' );",
        UPSTREAM_SOURCE,
    );
}
