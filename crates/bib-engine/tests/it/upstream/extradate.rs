// Direct passing translation of upstream t/extradate.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

fn pass_upstream(
    assertion: &str,
    actual_expression: &str,
    expected_expression: &str,
    upstream_call: &str,
    upstream_source: &str,
) {
    super::pass_upstream(
        assertion,
        actual_expression,
        expected_expression,
        upstream_call,
        upstream_source,
    );
    panic!("xfail: bib-engine has no public extra-date metadata query API");
}

const UPSTREAM_SOURCE: &str = r####"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 39;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Utils;
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

$biber->parse_ctrlfile('extradate.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'maxbibnames', 1);
Biber::Config->setblxoption(undef,'maxsortnames', 1);

# Now generate the information
$biber->prepare;
my $section = $biber->sections->get_section(0);
my $main = $biber->datalists->get_list('custom/global//global/global/global');
my $bibentries = $section->bibentries;

eq_or_diff($main->get_extradatedata_for_key('L1'), '1', 'Entry L1 - one name, first in 1995');
eq_or_diff($main->get_extradatedata_for_key('L2'), '2', 'Entry L2 - one name, second in 1995');
eq_or_diff($main->get_extradatedata_for_key('L3'), '3', 'Entry L3 - one name, third in 1995');
eq_or_diff($main->get_extradatedata_for_key('L4'), '1', 'Entry L4 - two names, first in 1995');
eq_or_diff($main->get_extradatedata_for_key('L5'), '2', 'Entry L5 - two names, second in 1995');
eq_or_diff($main->get_extradatedata_for_key('L6'), '1', 'Entry L6 - two names, first in 1996');
eq_or_diff($main->get_extradatedata_for_key('L7'), '2', 'Entry L7 - two names, second in 1996');
eq_or_diff($main->get_extradatedata_for_key('nodate1'), '1', 'Same name, no year 1');
eq_or_diff($main->get_extradatedata_for_key('nodate2'), '2', 'Same name, no year 2');
ok(is_undef($main->get_extradatedata_for_key('L8')), 'Entry L8 - one name, only in year');
ok(is_undef($main->get_extradatedata_for_key('L9')), 'Entry L9 - No name, same year as another with no name');
ok(is_undef($main->get_extradatedata_for_key('L10')), 'Entry L10 - No name, same year as another with no name');
eq_or_diff($main->get_extradatedata_for_key('companion1'), '1', 'Entry companion1 - names truncated to same as another entry in same year');
eq_or_diff($main->get_extradatedata_for_key('companion2'), '2', 'Entry companion2 - names truncated to same as another entry in same year');
ok(is_undef($main->get_extradatedata_for_key('companion3')), 'Entry companion3 - one name, same year as truncated names');
eq_or_diff($main->get_extradatedata_for_key('vangennep'), '2', 'Entry vangennep - useprefix does makes it different');
eq_or_diff($main->get_extradatedata_for_key('gennep'), '1', 'Entry gennep - different from prefix name');
ok(is_undef($main->get_extradatedata_for_key('LY1')), 'Date range means no extradate - 1');
ok(is_undef($main->get_extradatedata_for_key('LY2')), 'Date range means no extradate - 2');
ok(is_undef($main->get_extradatedata_for_key('LY3')), 'Date range means no extradate - 3');

# Test for labeldatesource literal string
eq_or_diff($bibentries->entry('nodate1')->get_field('labeldatesource'), 'nodate', 'Labeldatesource string - 1');
eq_or_diff($bibentries->entry('nodate2')->get_field('labeldatesource'), 'nodate', 'Labeldatesource string - 2');

# Testing different extradate scopes (granularity) extradate should be set
# at year scope only in default setup so these two get different extradate
# because they differ at month scope
eq_or_diff($main->get_extradatedata_for_key('ed1'), '1', 'labelyear scope - 1');
eq_or_diff($main->get_extradatedata_for_key('ed2'), '2', 'labelyear scope - 2');
eq_or_diff($bibentries->entry('ed1')->get_field('extradatescope'), 'labelyear', 'labelyear scope - 1a');
# One of these has an open enddate
ok(is_undef($main->get_extradatedata_for_key('ed7')), 'labelyear scope - 3');
ok(is_undef($main->get_extradatedata_for_key('ed8')), 'labelyear scope - 4');

# Switch to a month-in-year scope for extradate tracking
Biber::Config->setblxoption(undef,'extradatespec', [['labelyear', 'year'],['labelmonth']]);
$biber->prepare;
$main = $biber->datalists->get_list('custom/global//global/global/global');

# Now extradate should be unset as the months differ
ok(is_undef($main->get_extradatedata_for_key('ed1')), 'labelmonth scope - 1');
ok(is_undef($main->get_extradatedata_for_key('ed2')), 'labelmonth scope - 2');
eq_or_diff($bibentries->entry('ed1')->get_field('extradatescope'), 'labelmonth', 'labelmonth scope - 1a');

# But these have no months and are the same at labelyear so they should be set
eq_or_diff($main->get_extradatedata_for_key('ed3'), '1', 'labelmonth scope - 3');
eq_or_diff($main->get_extradatedata_for_key('ed4'), '2', 'labelmonth scope - 4');

# Switch to a minute scope for extradate tracking
Biber::Config->setblxoption(undef,'extradatespec', [['labelyear', 'year'],
                                              ['labelmonth'],
                                              ['labelday'],
                                              ['labelhour'],
                                              ['labelminute']]);
$biber->prepare;
$main = $biber->datalists->get_list('custom/global//global/global/global');

# extradate should be set as the minutes are the same
eq_or_diff($main->get_extradatedata_for_key('ed5'), '1', 'labelminute scope - 1');
eq_or_diff($main->get_extradatedata_for_key('ed6'), '2', 'labelminute scope - 2');
eq_or_diff($bibentries->entry('ed5')->get_field('extradatescope'), 'labelminute', 'labelminute scope - 1a');
# But these have no times
ok(is_undef($main->get_extradatedata_for_key('ed1')), 'labelminute scope - 3');
ok(is_undef($main->get_extradatedata_for_key('ed2')), 'labelminute scope - 4');

# Test not using label* which means that open enddates would not be
# considered
Biber::Config->setblxoption(undef,'extradatespec', [['year']]);
$biber->prepare;
$main = $biber->datalists->get_list('custom/global//global/global/global');
eq_or_diff($main->get_extradatedata_for_key('ed7'), '1', 'year scope - 1');
eq_or_diff($main->get_extradatedata_for_key('ed8'), '2', 'year scope - 2');

"####;

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_001_entry_l1_one_name_first_in_1995() {
    pass_upstream(
        "Entry L1 - one name, first in 1995",
        r####"$main->get_extradatedata_for_key('L1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L1'), '1', 'Entry L1 - one name, first in 1995');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_002_entry_l2_one_name_second_in_1995() {
    pass_upstream(
        "Entry L2 - one name, second in 1995",
        r####"$main->get_extradatedata_for_key('L2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L2'), '2', 'Entry L2 - one name, second in 1995');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_003_entry_l3_one_name_third_in_1995() {
    pass_upstream(
        "Entry L3 - one name, third in 1995",
        r####"$main->get_extradatedata_for_key('L3')"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L3'), '3', 'Entry L3 - one name, third in 1995');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_004_entry_l4_two_names_first_in_1995() {
    pass_upstream(
        "Entry L4 - two names, first in 1995",
        r####"$main->get_extradatedata_for_key('L4')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L4'), '1', 'Entry L4 - two names, first in 1995');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_005_entry_l5_two_names_second_in_1995() {
    pass_upstream(
        "Entry L5 - two names, second in 1995",
        r####"$main->get_extradatedata_for_key('L5')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L5'), '2', 'Entry L5 - two names, second in 1995');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_006_entry_l6_two_names_first_in_1996() {
    pass_upstream(
        "Entry L6 - two names, first in 1996",
        r####"$main->get_extradatedata_for_key('L6')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L6'), '1', 'Entry L6 - two names, first in 1996');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_007_entry_l7_two_names_second_in_1996() {
    pass_upstream(
        "Entry L7 - two names, second in 1996",
        r####"$main->get_extradatedata_for_key('L7')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('L7'), '2', 'Entry L7 - two names, second in 1996');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_008_same_name_no_year_1() {
    pass_upstream(
        "Same name, no year 1",
        r####"$main->get_extradatedata_for_key('nodate1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('nodate1'), '1', 'Same name, no year 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_009_same_name_no_year_2() {
    pass_upstream(
        "Same name, no year 2",
        r####"$main->get_extradatedata_for_key('nodate2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('nodate2'), '2', 'Same name, no year 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_010_entry_l8_one_name_only_in_year() {
    pass_upstream(
        "Entry L8 - one name, only in year",
        r####"is_undef($main->get_extradatedata_for_key('L8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('L8')), 'Entry L8 - one name, only in year');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_011_entry_l9_no_name_same_year_as_another_with_no_name() {
    pass_upstream(
        "Entry L9 - No name, same year as another with no name",
        r####"is_undef($main->get_extradatedata_for_key('L9'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('L9')), 'Entry L9 - No name, same year as another with no name');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_012_entry_l10_no_name_same_year_as_another_with_no_name() {
    pass_upstream(
        "Entry L10 - No name, same year as another with no name",
        r####"is_undef($main->get_extradatedata_for_key('L10'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('L10')), 'Entry L10 - No name, same year as another with no name');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_013_entry_companion1_names_truncated_to_same_as_another_entry_in_same_year() {
    pass_upstream(
        "Entry companion1 - names truncated to same as another entry in same year",
        r####"$main->get_extradatedata_for_key('companion1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('companion1'), '1', 'Entry companion1 - names truncated to same as another entry in same year');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_014_entry_companion2_names_truncated_to_same_as_another_entry_in_same_year() {
    pass_upstream(
        "Entry companion2 - names truncated to same as another entry in same year",
        r####"$main->get_extradatedata_for_key('companion2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('companion2'), '2', 'Entry companion2 - names truncated to same as another entry in same year');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_015_entry_companion3_one_name_same_year_as_truncated_names() {
    pass_upstream(
        "Entry companion3 - one name, same year as truncated names",
        r####"is_undef($main->get_extradatedata_for_key('companion3'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('companion3')), 'Entry companion3 - one name, same year as truncated names');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_016_entry_vangennep_useprefix_does_makes_it_different() {
    pass_upstream(
        "Entry vangennep - useprefix does makes it different",
        r####"$main->get_extradatedata_for_key('vangennep')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('vangennep'), '2', 'Entry vangennep - useprefix does makes it different');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_017_entry_gennep_different_from_prefix_name() {
    pass_upstream(
        "Entry gennep - different from prefix name",
        r####"$main->get_extradatedata_for_key('gennep')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('gennep'), '1', 'Entry gennep - different from prefix name');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_018_date_range_means_no_extradate_1() {
    pass_upstream(
        "Date range means no extradate - 1",
        r####"is_undef($main->get_extradatedata_for_key('LY1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('LY1')), 'Date range means no extradate - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_019_date_range_means_no_extradate_2() {
    pass_upstream(
        "Date range means no extradate - 2",
        r####"is_undef($main->get_extradatedata_for_key('LY2'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('LY2')), 'Date range means no extradate - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_020_date_range_means_no_extradate_3() {
    pass_upstream(
        "Date range means no extradate - 3",
        r####"is_undef($main->get_extradatedata_for_key('LY3'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('LY3')), 'Date range means no extradate - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_021_labeldatesource_string_1() {
    pass_upstream(
        "Labeldatesource string - 1",
        r####"$bibentries->entry('nodate1')->get_field('labeldatesource')"####,
        r####"'nodate'"####,
        r####"eq_or_diff($bibentries->entry('nodate1')->get_field('labeldatesource'), 'nodate', 'Labeldatesource string - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_022_labeldatesource_string_2() {
    pass_upstream(
        "Labeldatesource string - 2",
        r####"$bibentries->entry('nodate2')->get_field('labeldatesource')"####,
        r####"'nodate'"####,
        r####"eq_or_diff($bibentries->entry('nodate2')->get_field('labeldatesource'), 'nodate', 'Labeldatesource string - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_023_labelyear_scope_1() {
    pass_upstream(
        "labelyear scope - 1",
        r####"$main->get_extradatedata_for_key('ed1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed1'), '1', 'labelyear scope - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_024_labelyear_scope_2() {
    pass_upstream(
        "labelyear scope - 2",
        r####"$main->get_extradatedata_for_key('ed2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed2'), '2', 'labelyear scope - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_025_labelyear_scope_1a() {
    pass_upstream(
        "labelyear scope - 1a",
        r####"$bibentries->entry('ed1')->get_field('extradatescope')"####,
        r####"'labelyear'"####,
        r####"eq_or_diff($bibentries->entry('ed1')->get_field('extradatescope'), 'labelyear', 'labelyear scope - 1a');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_026_labelyear_scope_3() {
    pass_upstream(
        "labelyear scope - 3",
        r####"is_undef($main->get_extradatedata_for_key('ed7'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed7')), 'labelyear scope - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_027_labelyear_scope_4() {
    pass_upstream(
        "labelyear scope - 4",
        r####"is_undef($main->get_extradatedata_for_key('ed8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed8')), 'labelyear scope - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_028_labelmonth_scope_1() {
    pass_upstream(
        "labelmonth scope - 1",
        r####"is_undef($main->get_extradatedata_for_key('ed1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed1')), 'labelmonth scope - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_029_labelmonth_scope_2() {
    pass_upstream(
        "labelmonth scope - 2",
        r####"is_undef($main->get_extradatedata_for_key('ed2'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed2')), 'labelmonth scope - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_030_labelmonth_scope_1a() {
    pass_upstream(
        "labelmonth scope - 1a",
        r####"$bibentries->entry('ed1')->get_field('extradatescope')"####,
        r####"'labelmonth'"####,
        r####"eq_or_diff($bibentries->entry('ed1')->get_field('extradatescope'), 'labelmonth', 'labelmonth scope - 1a');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_031_labelmonth_scope_3() {
    pass_upstream(
        "labelmonth scope - 3",
        r####"$main->get_extradatedata_for_key('ed3')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed3'), '1', 'labelmonth scope - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_032_labelmonth_scope_4() {
    pass_upstream(
        "labelmonth scope - 4",
        r####"$main->get_extradatedata_for_key('ed4')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed4'), '2', 'labelmonth scope - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_033_labelminute_scope_1() {
    pass_upstream(
        "labelminute scope - 1",
        r####"$main->get_extradatedata_for_key('ed5')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed5'), '1', 'labelminute scope - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_034_labelminute_scope_2() {
    pass_upstream(
        "labelminute scope - 2",
        r####"$main->get_extradatedata_for_key('ed6')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed6'), '2', 'labelminute scope - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_035_labelminute_scope_1a() {
    pass_upstream(
        "labelminute scope - 1a",
        r####"$bibentries->entry('ed5')->get_field('extradatescope')"####,
        r####"'labelminute'"####,
        r####"eq_or_diff($bibentries->entry('ed5')->get_field('extradatescope'), 'labelminute', 'labelminute scope - 1a');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_036_labelminute_scope_3() {
    pass_upstream(
        "labelminute scope - 3",
        r####"is_undef($main->get_extradatedata_for_key('ed1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed1')), 'labelminute scope - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_037_labelminute_scope_4() {
    pass_upstream(
        "labelminute scope - 4",
        r####"is_undef($main->get_extradatedata_for_key('ed2'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ed2')), 'labelminute scope - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_038_year_scope_1() {
    pass_upstream(
        "year scope - 1",
        r####"$main->get_extradatedata_for_key('ed7')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed7'), '1', 'year scope - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public extra-date metadata query API"]
fn assertion_039_year_scope_2() {
    pass_upstream(
        "year scope - 2",
        r####"$main->get_extradatedata_for_key('ed8')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ed8'), '2', 'year scope - 2');"####,
        UPSTREAM_SOURCE,
    );
}
