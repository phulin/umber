// Direct xfail translation of upstream t/tool-config.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::xfail_upstream;

const UPSTREAM_SOURCE: &str = r########"# -*- cperl -*-
use strict;
use warnings;
use Test::More tests => 12;
use Test::Differences;
unified_diff;

use Encode;
use Biber;
use Biber::Utils;
use Biber::Output::bibtex;
use Log::Log4perl;
use Unicode::Normalize;
use XML::LibXML;
use Cwd 'abs_path';
use List::Util qw( first );

no warnings 'utf8';
use utf8;

chdir("t/tdata");

# Set up Biber object
my $biber = Biber->new(tool => 1,
                       configtool => abs_path('../../data/biber-tool.conf'),
                       configfile => 'tool-testconfig.conf');

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
$biber->set_output_obj(Biber::Output::bibtex->new());
# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('tool', 1);

# THERE IS A CONFIG FILE BEING READ!

# Now generate the information
$ARGV[0] = 'tool.bib'; # fake this as we are not running through top-level biber program
$biber->tool_mode_setup;
$biber->prepare_tool;
my $dm = Biber::Config->get_dm;
eq_or_diff(Biber::Config->getoption('mincrossrefs'), 5, 'Options 1');
eq_or_diff(Biber::Config->getoption('listsep'), 'and', 'Options 2');
is_deeply (Biber::Config->getblxoption(0, 'sortingtemplate'), {tool => { locale => undef, spec => [[{}, { citeorderX => {} }]] }}, 'Options 3');
# This is only in the user conf datamodel
ok((first {$_ eq 'newliteralfield'} $dm->get_fields_of_type('field', 'literal')->@*), 'Options 4');
ok($dm->is_field_for_entrytype('article', 'newliteralfield'), 'Options 5');
ok($dm->is_field_for_entrytype('xyz', 'author'), 'Options 6');
ok($dm->is_field_for_entrytype('xyz', 'file'), 'Options 7');
ok($dm->is_field_for_entrytype('xyz', 'abc'), 'Options 8');
ok($dm->is_field_for_entrytype('article', 'abc'), 'Options 9');
ok($dm->is_field_for_entrytype('book', 'bookzzz'), 'Options 10');
ok($dm->is_field_for_entrytype('article', 'bookzzz')==0, 'Options 11');
ok((first {$_ eq 'month'} $dm->get_fields_of_type('field', 'literal')->@*), 'Options 12');
"########;
#[test]
fn assertion_001_options_1() {
    xfail_upstream(
        "Options 1",
        r########"Biber::Config->getoption('mincrossrefs')"########,
        r########"5"########,
        r########"eq_or_diff(Biber::Config->getoption('mincrossrefs'), 5, 'Options 1');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_options_2() {
    xfail_upstream(
        "Options 2",
        r########"Biber::Config->getoption('listsep')"########,
        r########"'and'"########,
        r########"eq_or_diff(Biber::Config->getoption('listsep'), 'and', 'Options 2');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_options_3() {
    xfail_upstream(
        "Options 3",
        r########"Biber::Config->getblxoption(0, 'sortingtemplate')"########,
        r########"{tool => { locale => undef, spec => [[{}, { citeorderX => {} }]] }}"########,
        r########"is_deeply (Biber::Config->getblxoption(0, 'sortingtemplate'), {tool => { locale => undef, spec => [[{}, { citeorderX => {} }]] }}, 'Options 3');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_options_4() {
    xfail_upstream(
        "Options 4",
        r########"(first {$_ eq 'newliteralfield'} $dm->get_fields_of_type('field', 'literal')->@*)"########,
        r########"true"########,
        r########"ok((first {$_ eq 'newliteralfield'} $dm->get_fields_of_type('field', 'literal')->@*), 'Options 4');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_options_5() {
    xfail_upstream(
        "Options 5",
        r########"$dm->is_field_for_entrytype('article', 'newliteralfield')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('article', 'newliteralfield'), 'Options 5');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_options_6() {
    xfail_upstream(
        "Options 6",
        r########"$dm->is_field_for_entrytype('xyz', 'author')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('xyz', 'author'), 'Options 6');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_options_7() {
    xfail_upstream(
        "Options 7",
        r########"$dm->is_field_for_entrytype('xyz', 'file')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('xyz', 'file'), 'Options 7');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_options_8() {
    xfail_upstream(
        "Options 8",
        r########"$dm->is_field_for_entrytype('xyz', 'abc')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('xyz', 'abc'), 'Options 8');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_009_options_9() {
    xfail_upstream(
        "Options 9",
        r########"$dm->is_field_for_entrytype('article', 'abc')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('article', 'abc'), 'Options 9');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_010_options_10() {
    xfail_upstream(
        "Options 10",
        r########"$dm->is_field_for_entrytype('book', 'bookzzz')"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('book', 'bookzzz'), 'Options 10');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_011_options_11() {
    xfail_upstream(
        "Options 11",
        r########"$dm->is_field_for_entrytype('article', 'bookzzz')==0"########,
        r########"true"########,
        r########"ok($dm->is_field_for_entrytype('article', 'bookzzz')==0, 'Options 11');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_012_options_12() {
    xfail_upstream(
        "Options 12",
        r########"(first {$_ eq 'month'} $dm->get_fields_of_type('field', 'literal')->@*)"########,
        r########"true"########,
        r########"ok((first {$_ eq 'month'} $dm->get_fields_of_type('field', 'literal')->@*), 'Options 12');"########,
        UPSTREAM_SOURCE,
    );
}
