// Direct xfail translation of upstream t/sections-complex.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::xfail_upstream;

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 68;
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

$biber->parse_ctrlfile('sections-complex.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'maxalphanames', 1);
Biber::Config->setblxoption(undef,'labeldateparts', 0);

# Now generate the information
$biber->prepare;
my $section0 = $biber->sections->get_section(0);
my $bibentries0 = $section0->bibentries;
my $main0 = $biber->datalists->get_list('custom/global//global/global/global');
my $section1 = $biber->sections->get_section(1);
my $main1 = $biber->datalists->get_list('custom/global//global/global/global', 1);

my $bibentries1 = $section1->bibentries;

eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=1 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=1 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=1 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=1 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L4 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L4'), '3', 'maxalphanames=1 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L5 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L5'), '1', 'maxalphanames=1 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L6 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L6'), '2', 'maxalphanames=1 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L7 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L7'), '3', 'maxalphanames=1 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=1 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=1 minalphanames=1 entry L8 extraalpha');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'maxalphanames', 2);
Biber::Config->setblxoption(undef,'minalphanames', 1);

for (my $i=1; $i<5; $i++) {
  $bibentries0->entry("L$i")->del_field('sortlabelalpha');
  $bibentries0->entry("L$i")->del_field('labelalpha');
  $main0->set_extraalphadata_for_key("L$i", undef);
}
for (my $i=5; $i<9; $i++) {
  $bibentries1->entry("L$i")->del_field('sortlabelalpha');
  $bibentries1->entry("L$i")->del_field('labelalpha');
  $main1->set_extraalphadata_for_key("L$i", undef);
}
$biber->prepare;
$section0 = $biber->sections->get_section(0);
$bibentries0 = $section0->bibentries;
$main0 = $biber->datalists->get_list('custom/global//global/global/global');
$section1 = $biber->sections->get_section(1);
$main1 = $biber->datalists->get_list('custom/global//global/global/global', 1);

$bibentries1 = $section1->bibentries;

eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=2 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L4 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=2 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L5 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L5'), '1', 'maxalphanames=2 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L6 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L6'), '2', 'maxalphanames=2 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L7 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L7'), '3', 'maxalphanames=2 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=1 entry L8 extraalpha');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 2);
Biber::Config->setblxoption(undef,'maxalphanames', 2);
Biber::Config->setblxoption(undef,'minalphanames', 2);

for (my $i=1; $i<5; $i++) {
  $bibentries0->entry("L$i")->del_field('sortlabelalpha');
  $bibentries0->entry("L$i")->del_field('labelalpha');
  $main0->set_extraalphadata_for_key("L$i", undef);
}
for (my $i=5; $i<9; $i++) {
  $bibentries1->entry("L$i")->del_field('sortlabelalpha');
  $bibentries1->entry("L$i")->del_field('labelalpha');
  $main1->set_extraalphadata_for_key("L$i", undef);
}
$biber->prepare;
$section0 = $biber->sections->get_section(0);
$bibentries0 = $section0->bibentries;
$main0 = $biber->datalists->get_list('custom/global//global/global/global');
$section1 = $biber->sections->get_section(1);
$main1 = $biber->datalists->get_list('custom/global//global/global/global', 1);
$bibentries1 = $section1->bibentries;

eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=2 entry L1 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=2 minalphanames=2 entry L1 extraalpha');
eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L2 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=2 entry L2 extraalpha');
eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L3 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=2 entry L3 extraalpha');
eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L4 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=2 minalphanames=2 entry L4 extraalpha');
eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L5 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L5')), 'maxalphanames=2 minalphanames=2 entry L5 extraalpha');
eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L6 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L6'), '1', 'maxalphanames=2 minalphanames=2 entry L6 extraalpha');
eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L7 labelalpha');
eq_or_diff($main1->get_extraalphadata_for_key('L7'), '2', 'maxalphanames=2 minalphanames=2 entry L7 extraalpha');
eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=2 entry L8 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=2 entry L8 extraalpha');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'maxalphanames', 3);
Biber::Config->setblxoption(undef,'minalphanames', 1);

for (my $i=1; $i<5; $i++) {
  $bibentries0->entry("L$i")->del_field('sortlabelalpha');
  $bibentries0->entry("L$i")->del_field('labelalpha');
  $main0->set_extraalphadata_for_key("L$i", undef);
}
for (my $i=5; $i<9; $i++) {
  $bibentries1->entry("L$i")->del_field('sortlabelalpha');
  $bibentries1->entry("L$i")->del_field('labelalpha');
  $main1->set_extraalphadata_for_key("L$i", undef);
}

$biber->prepare;
$section0 = $biber->sections->get_section(0);
$bibentries0 = $section0->bibentries;
$main0 = $biber->datalists->get_list('custom/global//global/global/global');
$section1 = $biber->sections->get_section(1);
$main1 = $biber->datalists->get_list('custom/global//global/global/global', 1);
$bibentries1 = $section1->bibentries;

eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=3 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=3 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=3 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=3 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L4 labelalpha');
ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=3 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L5 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L5')), 'maxalphanames=3 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'DSE95', 'maxalphanames=3 minalphanames=1 entry L6 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L6')), 'maxalphanames=3 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'DSJ95', 'maxalphanames=3 minalphanames=1 entry L7 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L7')), 'maxalphanames=3 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=3 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=3 minalphanames=1 entry L8 extraalpha');
ok(is_undef($bibentries0->entry('m1')->get_field('keywords')), 'map refsection - 1');
eq_or_diff($bibentries0->entry('m1')->get_field('title'), 'Film title 1', 'map refsection - 2');
eq_or_diff($bibentries1->entry('m1')->get_field('keywords'), ['thing'], 'map refsection- 3');
eq_or_diff($bibentries1->entry('m1')->get_field('title'), 'Film title 11', 'map refsection - 4');
"#;

#[test]
fn assertion_001_maxalphanames_1_minalphanames_1_entry_l1_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L1 labelalpha",
        r"$main0->get_entryfield('L1', 'sortlabelalpha')",
        r"'Doe95'",
        r"eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=1 minalphanames=1 entry L1 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_maxalphanames_1_minalphanames_1_entry_l1_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L1 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L1'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=1 minalphanames=1 entry L1 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_maxalphanames_1_minalphanames_1_entry_l2_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L2 labelalpha",
        r"$main0->get_entryfield('L2', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L2 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_maxalphanames_1_minalphanames_1_entry_l2_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L2 extraalpha",
        r"$main0->get_extraalphadata_for_key('L2')",
        r"'1'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=1 minalphanames=1 entry L2 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_maxalphanames_1_minalphanames_1_entry_l3_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L3 labelalpha",
        r"$main0->get_entryfield('L3', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L3 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_maxalphanames_1_minalphanames_1_entry_l3_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L3 extraalpha",
        r"$main0->get_extraalphadata_for_key('L3')",
        r"'2'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=1 minalphanames=1 entry L3 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_maxalphanames_1_minalphanames_1_entry_l4_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L4 labelalpha",
        r"$main0->get_entryfield('L4', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L4 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_maxalphanames_1_minalphanames_1_entry_l4_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L4 extraalpha",
        r"$main0->get_extraalphadata_for_key('L4')",
        r"'3'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L4'), '3', 'maxalphanames=1 minalphanames=1 entry L4 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_009_maxalphanames_1_minalphanames_1_entry_l5_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L5 labelalpha",
        r"$main1->get_entryfield('L5', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L5 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_010_maxalphanames_1_minalphanames_1_entry_l5_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L5 extraalpha",
        r"$main1->get_extraalphadata_for_key('L5')",
        r"'1'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L5'), '1', 'maxalphanames=1 minalphanames=1 entry L5 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_011_maxalphanames_1_minalphanames_1_entry_l6_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L6 labelalpha",
        r"$main1->get_entryfield('L6', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L6 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_012_maxalphanames_1_minalphanames_1_entry_l6_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L6 extraalpha",
        r"$main1->get_extraalphadata_for_key('L6')",
        r"'2'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L6'), '2', 'maxalphanames=1 minalphanames=1 entry L6 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_013_maxalphanames_1_minalphanames_1_entry_l7_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L7 labelalpha",
        r"$main1->get_entryfield('L7', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L7 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_014_maxalphanames_1_minalphanames_1_entry_l7_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L7 extraalpha",
        r"$main1->get_extraalphadata_for_key('L7')",
        r"'3'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L7'), '3', 'maxalphanames=1 minalphanames=1 entry L7 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_015_maxalphanames_1_minalphanames_1_entry_l8_labelalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L8 labelalpha",
        r"$main1->get_entryfield('L8', 'sortlabelalpha')",
        r"'Sha85'",
        r"eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=1 minalphanames=1 entry L8 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_016_maxalphanames_1_minalphanames_1_entry_l8_extraalpha() {
    xfail_upstream(
        "maxalphanames=1 minalphanames=1 entry L8 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L8'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=1 minalphanames=1 entry L8 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_017_maxalphanames_2_minalphanames_1_entry_l1_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L1 labelalpha",
        r"$main0->get_entryfield('L1', 'sortlabelalpha')",
        r"'Doe95'",
        r"eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=1 entry L1 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_018_maxalphanames_2_minalphanames_1_entry_l1_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L1 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L1'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=2 minalphanames=1 entry L1 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_019_maxalphanames_2_minalphanames_1_entry_l2_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L2 labelalpha",
        r"$main0->get_entryfield('L2', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L2 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_020_maxalphanames_2_minalphanames_1_entry_l2_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L2 extraalpha",
        r"$main0->get_extraalphadata_for_key('L2')",
        r"'1'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=1 entry L2 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_021_maxalphanames_2_minalphanames_1_entry_l3_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L3 labelalpha",
        r"$main0->get_entryfield('L3', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L3 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_022_maxalphanames_2_minalphanames_1_entry_l3_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L3 extraalpha",
        r"$main0->get_extraalphadata_for_key('L3')",
        r"'2'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=1 entry L3 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_023_maxalphanames_2_minalphanames_1_entry_l4_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L4 labelalpha",
        r"$main0->get_entryfield('L4', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L4 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_024_maxalphanames_2_minalphanames_1_entry_l4_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L4 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L4'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=2 minalphanames=1 entry L4 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_025_maxalphanames_2_minalphanames_1_entry_l5_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L5 labelalpha",
        r"$main1->get_entryfield('L5', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L5 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_026_maxalphanames_2_minalphanames_1_entry_l5_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L5 extraalpha",
        r"$main1->get_extraalphadata_for_key('L5')",
        r"'1'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L5'), '1', 'maxalphanames=2 minalphanames=1 entry L5 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_027_maxalphanames_2_minalphanames_1_entry_l6_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L6 labelalpha",
        r"$main1->get_entryfield('L6', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L6 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_028_maxalphanames_2_minalphanames_1_entry_l6_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L6 extraalpha",
        r"$main1->get_extraalphadata_for_key('L6')",
        r"'2'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L6'), '2', 'maxalphanames=2 minalphanames=1 entry L6 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_029_maxalphanames_2_minalphanames_1_entry_l7_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L7 labelalpha",
        r"$main1->get_entryfield('L7', 'sortlabelalpha')",
        r"'Doe+95'",
        r"eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L7 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_030_maxalphanames_2_minalphanames_1_entry_l7_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L7 extraalpha",
        r"$main1->get_extraalphadata_for_key('L7')",
        r"'3'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L7'), '3', 'maxalphanames=2 minalphanames=1 entry L7 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_031_maxalphanames_2_minalphanames_1_entry_l8_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L8 labelalpha",
        r"$main1->get_entryfield('L8', 'sortlabelalpha')",
        r"'Sha85'",
        r"eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=1 entry L8 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_032_maxalphanames_2_minalphanames_1_entry_l8_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=1 entry L8 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L8'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=1 entry L8 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_033_maxalphanames_2_minalphanames_2_entry_l1_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L1 labelalpha",
        r"$main0->get_entryfield('L1', 'sortlabelalpha')",
        r"'Doe95'",
        r"eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=2 entry L1 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_034_maxalphanames_2_minalphanames_2_entry_l1_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L1 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L1'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=2 minalphanames=2 entry L1 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_035_maxalphanames_2_minalphanames_2_entry_l2_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L2 labelalpha",
        r"$main0->get_entryfield('L2', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L2 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_036_maxalphanames_2_minalphanames_2_entry_l2_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L2 extraalpha",
        r"$main0->get_extraalphadata_for_key('L2')",
        r"'1'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=2 entry L2 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_037_maxalphanames_2_minalphanames_2_entry_l3_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L3 labelalpha",
        r"$main0->get_entryfield('L3', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L3 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_038_maxalphanames_2_minalphanames_2_entry_l3_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L3 extraalpha",
        r"$main0->get_extraalphadata_for_key('L3')",
        r"'2'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=2 entry L3 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_039_maxalphanames_2_minalphanames_2_entry_l4_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L4 labelalpha",
        r"$main0->get_entryfield('L4', 'sortlabelalpha')",
        r"'DA+95'",
        r"eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L4 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_040_maxalphanames_2_minalphanames_2_entry_l4_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L4 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L4'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=2 minalphanames=2 entry L4 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_041_maxalphanames_2_minalphanames_2_entry_l5_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L5 labelalpha",
        r"$main1->get_entryfield('L5', 'sortlabelalpha')",
        r"'DA+95'",
        r"eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L5 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_042_maxalphanames_2_minalphanames_2_entry_l5_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L5 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L5'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L5')), 'maxalphanames=2 minalphanames=2 entry L5 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_043_maxalphanames_2_minalphanames_2_entry_l6_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L6 labelalpha",
        r"$main1->get_entryfield('L6', 'sortlabelalpha')",
        r"'DS+95'",
        r"eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L6 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_044_maxalphanames_2_minalphanames_2_entry_l6_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L6 extraalpha",
        r"$main1->get_extraalphadata_for_key('L6')",
        r"'1'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L6'), '1', 'maxalphanames=2 minalphanames=2 entry L6 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_045_maxalphanames_2_minalphanames_2_entry_l7_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L7 labelalpha",
        r"$main1->get_entryfield('L7', 'sortlabelalpha')",
        r"'DS+95'",
        r"eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L7 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_046_maxalphanames_2_minalphanames_2_entry_l7_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L7 extraalpha",
        r"$main1->get_extraalphadata_for_key('L7')",
        r"'2'",
        r"eq_or_diff($main1->get_extraalphadata_for_key('L7'), '2', 'maxalphanames=2 minalphanames=2 entry L7 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_047_maxalphanames_2_minalphanames_2_entry_l8_labelalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L8 labelalpha",
        r"$main1->get_entryfield('L8', 'sortlabelalpha')",
        r"'Sha85'",
        r"eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=2 entry L8 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_048_maxalphanames_2_minalphanames_2_entry_l8_extraalpha() {
    xfail_upstream(
        "maxalphanames=2 minalphanames=2 entry L8 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L8'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=2 entry L8 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_049_maxalphanames_3_minalphanames_1_entry_l1_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L1 labelalpha",
        r"$main0->get_entryfield('L1', 'sortlabelalpha')",
        r"'Doe95'",
        r"eq_or_diff($main0->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=3 minalphanames=1 entry L1 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_050_maxalphanames_3_minalphanames_1_entry_l1_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L1 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L1'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L1')), 'maxalphanames=3 minalphanames=1 entry L1 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_051_maxalphanames_3_minalphanames_1_entry_l2_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L2 labelalpha",
        r"$main0->get_entryfield('L2', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L2 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_052_maxalphanames_3_minalphanames_1_entry_l2_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L2 extraalpha",
        r"$main0->get_extraalphadata_for_key('L2')",
        r"'1'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=3 minalphanames=1 entry L2 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_053_maxalphanames_3_minalphanames_1_entry_l3_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L3 labelalpha",
        r"$main0->get_entryfield('L3', 'sortlabelalpha')",
        r"'DA95'",
        r"eq_or_diff($main0->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L3 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_054_maxalphanames_3_minalphanames_1_entry_l3_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L3 extraalpha",
        r"$main0->get_extraalphadata_for_key('L3')",
        r"'2'",
        r"eq_or_diff($main0->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=3 minalphanames=1 entry L3 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_055_maxalphanames_3_minalphanames_1_entry_l4_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L4 labelalpha",
        r"$main0->get_entryfield('L4', 'sortlabelalpha')",
        r"'DAE95'",
        r"eq_or_diff($main0->get_entryfield('L4', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L4 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_056_maxalphanames_3_minalphanames_1_entry_l4_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L4 extraalpha",
        r"is_undef($main0->get_extraalphadata_for_key('L4'))",
        r"true",
        r"ok(is_undef($main0->get_extraalphadata_for_key('L4')), 'maxalphanames=3 minalphanames=1 entry L4 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_057_maxalphanames_3_minalphanames_1_entry_l5_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L5 labelalpha",
        r"$main1->get_entryfield('L5', 'sortlabelalpha')",
        r"'DAE95'",
        r"eq_or_diff($main1->get_entryfield('L5', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L5 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_058_maxalphanames_3_minalphanames_1_entry_l5_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L5 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L5'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L5')), 'maxalphanames=3 minalphanames=1 entry L5 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_059_maxalphanames_3_minalphanames_1_entry_l6_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L6 labelalpha",
        r"$main1->get_entryfield('L6', 'sortlabelalpha')",
        r"'DSE95'",
        r"eq_or_diff($main1->get_entryfield('L6', 'sortlabelalpha'), 'DSE95', 'maxalphanames=3 minalphanames=1 entry L6 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_060_maxalphanames_3_minalphanames_1_entry_l6_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L6 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L6'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L6')), 'maxalphanames=3 minalphanames=1 entry L6 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_061_maxalphanames_3_minalphanames_1_entry_l7_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L7 labelalpha",
        r"$main1->get_entryfield('L7', 'sortlabelalpha')",
        r"'DSJ95'",
        r"eq_or_diff($main1->get_entryfield('L7', 'sortlabelalpha'), 'DSJ95', 'maxalphanames=3 minalphanames=1 entry L7 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_062_maxalphanames_3_minalphanames_1_entry_l7_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L7 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L7'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L7')), 'maxalphanames=3 minalphanames=1 entry L7 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_063_maxalphanames_3_minalphanames_1_entry_l8_labelalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L8 labelalpha",
        r"$main1->get_entryfield('L8', 'sortlabelalpha')",
        r"'Sha85'",
        r"eq_or_diff($main1->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=3 minalphanames=1 entry L8 labelalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_064_maxalphanames_3_minalphanames_1_entry_l8_extraalpha() {
    xfail_upstream(
        "maxalphanames=3 minalphanames=1 entry L8 extraalpha",
        r"is_undef($main1->get_extraalphadata_for_key('L8'))",
        r"true",
        r"ok(is_undef($main1->get_extraalphadata_for_key('L8')), 'maxalphanames=3 minalphanames=1 entry L8 extraalpha');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_065_map_refsection_1() {
    xfail_upstream(
        "map refsection - 1",
        r"is_undef($bibentries0->entry('m1')->get_field('keywords'))",
        r"true",
        r"ok(is_undef($bibentries0->entry('m1')->get_field('keywords')), 'map refsection - 1');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_066_map_refsection_2() {
    xfail_upstream(
        "map refsection - 2",
        r"$bibentries0->entry('m1')->get_field('title')",
        r"'Film title 1'",
        r"eq_or_diff($bibentries0->entry('m1')->get_field('title'), 'Film title 1', 'map refsection - 2');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_067_map_refsection_3() {
    xfail_upstream(
        "map refsection- 3",
        r"$bibentries1->entry('m1')->get_field('keywords')",
        r"['thing']",
        r"eq_or_diff($bibentries1->entry('m1')->get_field('keywords'), ['thing'], 'map refsection- 3');",
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_068_map_refsection_4() {
    xfail_upstream(
        "map refsection - 4",
        r"$bibentries1->entry('m1')->get_field('title')",
        r"'Film title 11'",
        r"eq_or_diff($bibentries1->entry('m1')->get_field('title'), 'Film title 11', 'map refsection - 4');",
        UPSTREAM_SOURCE,
    );
}
