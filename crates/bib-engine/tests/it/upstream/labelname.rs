// Direct xfail translation of upstream t/labelname.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::xfail_upstream;

const UPSTREAM_SOURCE: &str = r####"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 4;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Output::bbl;
use Log::Log4perl;
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

Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
$biber->parse_ctrlfile("general.bcf");
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biblatex options
Biber::Config->setblxoption(undef,'labelnamespec', [ {content => 'shortauthor'},
                                               {content => 'author'},
                                               {content => 'shorteditor'},
                                               {content => 'editor'},
                                               {content => 'translator'}]);
Biber::Config->setblxoption(undef,'labelnamespec', [ {content => 'editor'},
                                               {content => 'translator'}], 'ENTRYTYPE', 'book');
Biber::Config->setblxoption(undef,'labelnamespec', [ {content => 'namea'},
                                               {content => 'author' }], 'ENTRYTYPE', 'misc');

# Now generate the information
$biber->prepare;
my $bibentries = $biber->sections->get_section(0)->bibentries;

eq_or_diff($bibentries->entry('angenendtsa')->get_labelname_info, 'shortauthor', 'global shortauthor' );
eq_or_diff($bibentries->entry('stdmodel')->get_labelname_info, 'author', 'global author' );
eq_or_diff($bibentries->entry('aristotle:anima')->get_labelname_info, 'editor', 'type-specific editor' );
eq_or_diff($bibentries->entry('lne1')->get_labelname_info, 'namea', 'type-specific exotic name' );
"####;

#[test]
fn assertion_001_global_shortauthor() {
    xfail_upstream(
        "global shortauthor",
        r####"$bibentries->entry('angenendtsa')->get_labelname_info"####,
        r####"'shortauthor'"####,
        r####"eq_or_diff($bibentries->entry('angenendtsa')->get_labelname_info, 'shortauthor', 'global shortauthor' );"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_global_author() {
    xfail_upstream(
        "global author",
        r####"$bibentries->entry('stdmodel')->get_labelname_info"####,
        r####"'author'"####,
        r####"eq_or_diff($bibentries->entry('stdmodel')->get_labelname_info, 'author', 'global author' );"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_type_specific_editor() {
    xfail_upstream(
        "type-specific editor",
        r####"$bibentries->entry('aristotle:anima')->get_labelname_info"####,
        r####"'editor'"####,
        r####"eq_or_diff($bibentries->entry('aristotle:anima')->get_labelname_info, 'editor', 'type-specific editor' );"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_type_specific_exotic_name() {
    xfail_upstream(
        "type-specific exotic name",
        r####"$bibentries->entry('lne1')->get_labelname_info"####,
        r####"'namea'"####,
        r####"eq_or_diff($bibentries->entry('lne1')->get_labelname_info, 'namea', 'type-specific exotic name' );"####,
        UPSTREAM_SOURCE,
    );
}
