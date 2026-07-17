// Direct xfail translation of upstream t/maps.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::xfail_upstream;

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 8;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Utils;
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

$biber->parse_ctrlfile('maps.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Now generate the information
$biber->prepare;
my $out = $biber->get_output_obj;
my $section = $biber->sections->get_section(0);
my $bibentries = $section->bibentries;

# Explicitly cited ARTICLE, not deleted by map
ok(defined($bibentries->entry('maps1')), 'Maps test - 1' );
# \nocite{*} ARTICLE, deleted by map
ok(is_undef($bibentries->entry('maps2')), 'Maps test - 2' );
# \nocite{*} COLLECTION, not deleted by map
ok(defined($bibentries->entry('maps3')), 'Maps test - 3' );
# \nocited{*} BOOK, deleted by map
ok(is_undef($bibentries->entry('maps4')), 'Maps test - 4' );
# Specifically cited ARTICLE, field set
eq_or_diff($bibentries->entry('maps1')->get_field('verba'), 'somevalue', 'Maps test - 5' );
ok(is_undef($bibentries->entry('maps3')->get_field('verba')), 'Maps test - 6' );
eq_or_diff($bibentries->entry('maps1')->get_field('verbb'), 'somevalue1', 'Maps test - 7' );
ok(is_undef($bibentries->entry('maps3')->get_field('verbb')), 'Maps test - 8' );
"#;

#[test]
fn assertion_001_maps_test_1() {
    xfail_upstream(
        "Maps test - 1",
        r"defined($bibentries->entry('maps1'))",
        r"true",
        r"ok(defined($bibentries->entry('maps1')), 'Maps test - 1' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_maps_test_2() {
    xfail_upstream(
        "Maps test - 2",
        r"is_undef($bibentries->entry('maps2'))",
        r"true",
        r"ok(is_undef($bibentries->entry('maps2')), 'Maps test - 2' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_maps_test_3() {
    xfail_upstream(
        "Maps test - 3",
        r"defined($bibentries->entry('maps3'))",
        r"true",
        r"ok(defined($bibentries->entry('maps3')), 'Maps test - 3' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_maps_test_4() {
    xfail_upstream(
        "Maps test - 4",
        r"is_undef($bibentries->entry('maps4'))",
        r"true",
        r"ok(is_undef($bibentries->entry('maps4')), 'Maps test - 4' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_maps_test_5() {
    xfail_upstream(
        "Maps test - 5",
        r"$bibentries->entry('maps1')->get_field('verba')",
        r"'somevalue'",
        r"eq_or_diff($bibentries->entry('maps1')->get_field('verba'), 'somevalue', 'Maps test - 5' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_maps_test_6() {
    xfail_upstream(
        "Maps test - 6",
        r"is_undef($bibentries->entry('maps3')->get_field('verba'))",
        r"true",
        r"ok(is_undef($bibentries->entry('maps3')->get_field('verba')), 'Maps test - 6' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_maps_test_7() {
    xfail_upstream(
        "Maps test - 7",
        r"$bibentries->entry('maps1')->get_field('verbb')",
        r"'somevalue1'",
        r"eq_or_diff($bibentries->entry('maps1')->get_field('verbb'), 'somevalue1', 'Maps test - 7' );",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_maps_test_8() {
    xfail_upstream(
        "Maps test - 8",
        r"is_undef($bibentries->entry('maps3')->get_field('verbb'))",
        r"true",
        r"ok(is_undef($bibentries->entry('maps3')->get_field('verbb')), 'Maps test - 8' );",
        UPSTREAM_SOURCE,
    );
}
