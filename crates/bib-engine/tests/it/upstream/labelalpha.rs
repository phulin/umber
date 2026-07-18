// Direct translation of upstream t/labelalpha.t at commit 74252e6.
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
    panic!("xfail: bib-engine has no public label-alpha data query API");
}

const UPSTREAM_SOURCE: &str = r####"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 122;
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

$biber->parse_ctrlfile('labelalpha.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxalphanames', 1);
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'labeldateparts', 0);

# Now generate the information, saving per-entry options or they are deleted
$biber->prepare;

my $section = $biber->sections->get_section(0);
my $main = $biber->datalists->get_list('custom/global//global/global/global');
my $bibentries = $section->bibentries;

# Test with useprefix=false
eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'Vaa99', 'useprefix=0 so not in label');

# useprefix=true
Biber::Config->setblxoption(undef,'useprefix', 1);
$biber->prepare;

eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'vdVaa99', 'Default prefix settings entry prefix1 labelalpha');
eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=1 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=1 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=1 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=1 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L4 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L4'), '3', 'maxalphanames=1 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L5 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L5'), '4', 'maxalphanames=1 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L6 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L6'), '5', 'maxalphanames=1 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L7 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L7'), '6', 'maxalphanames=1 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=1 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=1 minalphanames=1 entry L8 extraalpha');
ok(is_undef($main->get_extraalphadata_for_key('L9')), 'L9 extraalpha unset due to shorthand');
ok(is_undef($main->get_extraalphadata_for_key('L10')), 'L10 extraalpha unset due to shorthand');
eq_or_diff($main->get_extraalphadata_for_key('knuth:ct'), '1', 'YEAR with range needs label differentiating from individual volumes - 1');
eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:a'), '2', 'YEAR with range needs label differentiating from individual volumes - 2');
eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:b'), '1', 'YEAR with range needs label differentiating from individual volumes - 3');
eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:c'), '2', 'YEAR with range needs label differentiating from individual volumes - 4');
eq_or_diff($main->get_entryfield('ignore1', 'sortlabelalpha'), 'OTo07', 'Default ignore');
eq_or_diff($main->get_entryfield('ignore2', 'sortlabelalpha'), 'De 07', 'Default no ignore spaces');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxalphanames', 2);
Biber::Config->setblxoption(undef,'minalphanames', 1);
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 1);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=2 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L4 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=2 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L5 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=2 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L6 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L6'), '3', 'maxalphanames=2 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L7 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L7'), '4', 'maxalphanames=2 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=1 entry L8 extraalpha');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxalphanames', 2);
Biber::Config->setblxoption(undef,'minalphanames', 2);
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 2);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=2 entry L1 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=2 minalphanames=2 entry L1 extraalpha');
eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L2 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=2 entry L2 extraalpha');
eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L3 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=2 entry L3 extraalpha');
eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L4 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=2 minalphanames=2 entry L4 extraalpha');
eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L5 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=2 minalphanames=2 entry L5 extraalpha');
eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L6 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L6'), '1', 'maxalphanames=2 minalphanames=2 entry L6 extraalpha');
eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L7 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L7'), '2', 'maxalphanames=2 minalphanames=2 entry L7 extraalpha');
eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=2 entry L8 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=2 entry L8 extraalpha');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxalphanames', 3);
Biber::Config->setblxoption(undef,'minalphanames', 1);
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=3 minalphanames=1 entry L1 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L1')), 'maxalphanames=3 minalphanames=1 entry L1 extraalpha');
eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L2 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=3 minalphanames=1 entry L2 extraalpha');
eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L3 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=3 minalphanames=1 entry L3 extraalpha');
eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L4 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=3 minalphanames=1 entry L4 extraalpha');
eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L5 labelalpha');
eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=3 minalphanames=1 entry L5 extraalpha');
eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'DSE95', 'maxalphanames=3 minalphanames=1 entry L6 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L6')), 'maxalphanames=3 minalphanames=1 entry L6 extraalpha');
eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'DSJ95', 'maxalphanames=3 minalphanames=1 entry L7 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L7')), 'maxalphanames=3 minalphanames=1 entry L7 extraalpha');
eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=3 minalphanames=1 entry L8 labelalpha');
ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=3 minalphanames=1 entry L8 extraalpha');
eq_or_diff($main->get_entryfield('LDN1', 'sortlabelalpha'), 'VUR89', 'Testing compound lastnames 1');
eq_or_diff($main->get_entryfield('LDN2', 'sortlabelalpha'), 'VU45', 'Testing compound lastnames 2');
eq_or_diff($main->get_entryfield('LDN3', 'sortlabelalpha'), 'VisvSJRu45', 'Testing with multiple pre and main and width/side override');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxalphanames', 4);
Biber::Config->setblxoption(undef,'minalphanames', 4);
Biber::Config->setblxoption(undef,'maxcitenames', 4);
Biber::Config->setblxoption(undef,'mincitenames', 4);
Biber::Config->setblxoption(undef,'labelalpha', 1);
Biber::Config->setblxoption(undef,'labeldateparts', 1);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

my $out = $biber->get_output_obj;
$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('L11', 'sortlabelalpha'), 'vRan22', 'prefix labelalpha 1');
eq_or_diff($main->get_entryfield('L12', 'sortlabelalpha'), 'vRvB2', 'prefix labelalpha 2');
# only the first name in the list is in the label due to namecount=1
eq_or_diff($main->get_entryfield('L13', 'sortlabelalpha'), 'vRa+-ksUnV', 'per-type labelalpha 1');
eq_or_diff($main->get_entryfield('L14', 'sortlabelalpha'), 'Alabel-ksUnW', 'per-type labelalpha 2');
eq_or_diff($main->get_entryfield('L15', 'sortlabelalpha'), 'AccBrClim', 'labelalpha disambiguation 1');
eq_or_diff($main->get_entryfield('L16', 'sortlabelalpha'), 'AccBaClim', 'labelalpha disambiguation 2');
eq_or_diff($main->get_entryfield('L16a', 'sortlabelalpha'), 'AccBaClim', 'labelalpha disambiguation 2a');
eq_or_diff($main->get_extraalphadata_for_key('L16'), '1', 'labelalpha disambiguation 2c');
eq_or_diff($main->get_extraalphadata_for_key('L16a'), '2', 'labelalpha disambiguation 2d');
eq_or_diff($main->get_entryfield('L17', 'sortlabelalpha'), 'AckBaClim', 'labelalpha disambiguation 3');
eq_or_diff($main->get_extraalphadata_for_key('L17a'), '2', 'custom labelalpha extradate 1');
eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'AgChLa', 'labelalpha disambiguation 4');
eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'AgConLe', 'labelalpha disambiguation 5');
eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'AgCouLa', 'labelalpha disambiguation 6');
eq_or_diff($main->get_entryfield('L21', 'sortlabelalpha'), 'BoConEdb', 'labelalpha disambiguation 7');
eq_or_diff($main->get_entryfield('L22', 'sortlabelalpha'), 'BoConEm', 'labelalpha disambiguation 8');
eq_or_diff($main->get_entryfield('L23', 'sortlabelalpha'), 'Sa', 'labelalpha disambiguation 9');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
             {
               labelpart => [
                 {
                  content         => "labelname",
                  substring_width => "vf",
                  namessep => "/",
                  substring_fixed_threshold => 2,
                  substring_side => "left"
                 },
               ],
               order => 1,
             },
           ],
  type  => "unpublished",
}, 'ENTRYTYPE', 'unpublished');


foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

# "Agas" and not "Aga" because the Schmidt/Schnee below need 4 chars to disambiguate
eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'Agas/Cha/Laver', 'labelalpha disambiguation 10');
eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'Agas/Con/Lendl', 'labelalpha disambiguation 11');
eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'Agas/Cou/Laver', 'labelalpha disambiguation 12');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
             {
               labelpart => [
                 {
                  content         => "labelname",
                  substring_width => "l",
                  substring_side => "left"
                 },
               ],
               order => 1,
             },
           ],
  type  => "unpublished",
}, 'ENTRYTYPE', 'unpublished');

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'AChL', 'labelalpha list disambiguation 1');
eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'ACoL', 'labelalpha list disambiguation 2');
eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'ACL', 'labelalpha list disambiguation 3');
eq_or_diff($main->get_entryfield('L21', 'sortlabelalpha'), 'BCEd', 'labelalpha list disambiguation 4');
eq_or_diff($main->get_entryfield('L22', 'sortlabelalpha'), 'BCE', 'labelalpha list disambiguation 5');
eq_or_diff($main->get_entryfield('L24', 'sortlabelalpha'), 'Z', 'labelalpha list disambiguation 6');
eq_or_diff($main->get_entryfield('L25', 'sortlabelalpha'), 'ZX', 'labelalpha list disambiguation 7');
eq_or_diff($main->get_entryfield('L26', 'sortlabelalpha'), 'ZX', 'labelalpha list disambiguation 8');
eq_or_diff(NFC($main->get_entryfield('title1', 'sortlabelalpha')), 'Tït', 'Title in braces with UTF-8 char - 1');

# reset options and regenerate information
Biber::Config->setblxoption(undef,'maxalphanames', 3);
Biber::Config->setblxoption(undef,'minalphanames', 1);
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);

Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
             {
               labelpart => [
                 { content => "shorthand", final => 1 },
                 { content => "label" },
                 {
                   content         => "labelname",
                   ifnames     => 1,
                   substring_side  => "left",
                   substring_width => 3,
                 },
                 { content => "labelname", substring_side => "left", substring_width => 1 },
               ],
               order => 1,
             },
             {
               labelpart => [
                 { content => "year", substring_side => "right", substring_width => 2 },
               ],
               order => 2,
             },
           ],
  type  => "global",
});

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('Schmidt2007', 'sortlabelalpha'), 'Sch+07', 'extraalpha ne extradate 1');
eq_or_diff($main->get_extraalphadata_for_key('Schmidt2007'), '1', 'extraalpha ne extradate 2');
eq_or_diff($main->get_entryfield('Schmidt2007a', 'sortlabelalpha'), 'Sch07', 'extraalpha ne extradate 3');
eq_or_diff($main->get_extraalphadata_for_key('Schmidt2007a'), '1', 'extraalpha ne extradate 4');

eq_or_diff($main->get_entryfield('Schnee2007', 'sortlabelalpha'), 'Sch+07', 'extraalpha ne extradate 5');
eq_or_diff($main->get_extraalphadata_for_key('Schnee2007'), '2', 'extraalpha ne extradate 6');
eq_or_diff($main->get_entryfield('Schnee2007a', 'sortlabelalpha'), 'Sch07', 'extraalpha ne extradate 7');
eq_or_diff($main->get_extraalphadata_for_key('Schnee2007a'), '2', 'extraalpha ne extradate 8');

Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
             {
               labelpart => [
                 {
                   content         => "citekey",
                   substring_side  => "left",
                   substring_width => 3,
                   uppercase => 1,
                 },
               ],
               order => 1,
             },
           ],
  type  => "global",
});

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('Schmidt2007', 'sortlabelalpha'), 'SCH', 'entrykey label 1');

Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
             {
               labelpart => [
                 {
                  content         => "labelyear",
                 }
               ],
              order => 1,
             },
             {
               labelpart => [
                 {
                  content         => "labelmonth",
                 }
               ],
              order => 2,
             },
             {
               labelpart => [
                 {
                  content         => "labelday",
                 }
               ],
              order => 3,
             }
           ],
  type  => "global",
});

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}

$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('labelstest', 'sortlabelalpha'), '200532', 'labeldate test - 1');
eq_or_diff($main->get_entryfield('padtest', 'labelalpha'), '\&Al\_\_{\textasciitilde}{\textasciitilde}T07', 'pad test - 1');
eq_or_diff($main->get_entryfield('padtest', 'sortlabelalpha'), '&Al__~~T07', 'pad test - 2');

my $lant = Biber::Config->getblxoption(undef,'labelalphanametemplate');
$lant->{global} = [
 {
    namepart => "prefix",
    pre => 1,
    substring_compound => 1,
    substring_width => 2,
    use => 1,
  },
  {
    namepart => "family",
    pre => undef,
    substring_compound => 1,
    substring_width => undef,
    use => undef,
  }];
Biber::Config->setblxoption(undef,'labelalphanametemplate', $lant);

Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
                   {
                    labelpart => [
                                  {
                                   content         => "author",
                                   ifnames         => 1,
                                   substring_side  => "left",
                                   substring_width => 3,
                                  },
                                 ],
                    order => 1,
                   },
                   {
                    labelpart => [
                                  {
                                   content         => "title",
                                   substring_side  => "left",
                                   substring_width => 4,
                                  },
                                 ],
                    order => 2,
             },
           ],
  type  => "global",
});

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}
# The "o"s are ignored for width substring calculation - take note
Biber::Config->setoption('nolabelwidthcount', [ {value => q/o+/} ] );
$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('skipwidthtest1', 'sortlabelalpha'), 'OToolOToole', 'Skip width test - 1');
eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'vadeVaaThin', 'compound and string length entry prefix1 labelalpha');

Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
                   {
                    labelpart => [
                                  {
                   content         => "author",
                   names       => "2-7"
                                  },
                   ],
                   order => 1,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 2,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "editor",
                   names       => "--3"
                                  },
                   ],
                   order => 3,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 4,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "translator",
                   names       => "2",
                   noalphaothers   => "1"
                                  },
                   ],
                   order => 5,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 6,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "foreword",
                   names       => "3--"
                                  },
                   ],
                   order => 7,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 8,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "holder",
                   names       => "2-+"
                                  },
                   ],
                   order => 9,
                   },
                  ],
  type  => "global",
});
Biber::Config->setblxoption(undef,'minalphanames', 2);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}
$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('rangetest1', 'sortlabelalpha'), 'WAXAYAZA.VEWEXE+.VTWT.XFYFZF.WH+', 'Name range test - 1');


Biber::Config->setblxoption(undef,'labelalphatemplate', {
  labelelement => [
                   {
                    labelpart => [
                                  {
                   content         => "author",
                   ifnames     => "3-",
                   substring_side  => "left",
                   substring_width => 1,
                                  },
                   ],
                   order => 1,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 2,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "editor",
                   ifnames     => "-2",
                   substring_side  => "left",
                   substring_width => 1,
                                  },
                   ],
                   order => 3,
                   },
                   {
                    labelpart => [
                                  {
                   content         => ".",
                                  },
                   ],
                   order => 4,
                   },
                   {
                    labelpart => [
                                  {
                   content         => "translator",
                   ifnames     => "4-6",
                   namessep     => "/",
                   substring_side  => "left",
                   substring_width => 1,
                                  },
                   ],
                   order => 5,
                   },

                  ],
  type  => "global",
});

Biber::Config->setblxoption(undef,'maxalphanames', 10);
Biber::Config->setblxoption(undef,'minalphanames', 10);

foreach my $k ($section->get_citekeys) {
  $bibentries->entry($k)->del_field('sortlabelalpha');
  $bibentries->entry($k)->del_field('labelalpha');
  $main->set_extraalphadata_for_key($k, undef);
}
$biber->prepare;

$section = $biber->sections->get_section(0);
$main = $biber->datalists->get_list('custom/global//global/global/global');
$bibentries = $section->bibentries;

eq_or_diff($main->get_entryfield('rangetest1', 'sortlabelalpha'), 'VWXYZ..V/W/X/Y/Z', 'Name range test - 2');

"####;

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_001_useprefix_0_so_not_in_label() {
    pass_upstream(
        "useprefix=0 so not in label",
        r####"$main->get_entryfield('prefix1', 'sortlabelalpha')"####,
        r####"'Vaa99'"####,
        r####"eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'Vaa99', 'useprefix=0 so not in label');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_002_default_prefix_settings_entry_prefix1_labelalpha() {
    pass_upstream(
        "Default prefix settings entry prefix1 labelalpha",
        r####"$main->get_entryfield('prefix1', 'sortlabelalpha')"####,
        r####"'vdVaa99'"####,
        r####"eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'vdVaa99', 'Default prefix settings entry prefix1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_003_maxalphanames_1_minalphanames_1_entry_l1_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L1 labelalpha",
        r####"$main->get_entryfield('L1', 'sortlabelalpha')"####,
        r####"'Doe95'"####,
        r####"eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=1 minalphanames=1 entry L1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_004_maxalphanames_1_minalphanames_1_entry_l1_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L1 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('l1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=1 minalphanames=1 entry L1 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_005_maxalphanames_1_minalphanames_1_entry_l2_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L2 labelalpha",
        r####"$main->get_entryfield('L2', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L2 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_006_maxalphanames_1_minalphanames_1_entry_l2_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L2 extraalpha",
        r####"$main->get_extraalphadata_for_key('L2')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=1 minalphanames=1 entry L2 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_007_maxalphanames_1_minalphanames_1_entry_l3_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L3 labelalpha",
        r####"$main->get_entryfield('L3', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L3 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_008_maxalphanames_1_minalphanames_1_entry_l3_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L3 extraalpha",
        r####"$main->get_extraalphadata_for_key('L3')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=1 minalphanames=1 entry L3 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_009_maxalphanames_1_minalphanames_1_entry_l4_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L4 labelalpha",
        r####"$main->get_entryfield('L4', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L4 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_010_maxalphanames_1_minalphanames_1_entry_l4_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L4 extraalpha",
        r####"$main->get_extraalphadata_for_key('L4')"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L4'), '3', 'maxalphanames=1 minalphanames=1 entry L4 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_011_maxalphanames_1_minalphanames_1_entry_l5_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L5 labelalpha",
        r####"$main->get_entryfield('L5', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L5 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_012_maxalphanames_1_minalphanames_1_entry_l5_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L5 extraalpha",
        r####"$main->get_extraalphadata_for_key('L5')"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L5'), '4', 'maxalphanames=1 minalphanames=1 entry L5 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_013_maxalphanames_1_minalphanames_1_entry_l6_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L6 labelalpha",
        r####"$main->get_entryfield('L6', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L6 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_014_maxalphanames_1_minalphanames_1_entry_l6_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L6 extraalpha",
        r####"$main->get_extraalphadata_for_key('L6')"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L6'), '5', 'maxalphanames=1 minalphanames=1 entry L6 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_015_maxalphanames_1_minalphanames_1_entry_l7_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L7 labelalpha",
        r####"$main->get_entryfield('L7', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=1 minalphanames=1 entry L7 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_016_maxalphanames_1_minalphanames_1_entry_l7_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L7 extraalpha",
        r####"$main->get_extraalphadata_for_key('L7')"####,
        r####"'6'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L7'), '6', 'maxalphanames=1 minalphanames=1 entry L7 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_017_maxalphanames_1_minalphanames_1_entry_l8_labelalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L8 labelalpha",
        r####"$main->get_entryfield('L8', 'sortlabelalpha')"####,
        r####"'Sha85'"####,
        r####"eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=1 minalphanames=1 entry L8 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_018_maxalphanames_1_minalphanames_1_entry_l8_extraalpha() {
    pass_upstream(
        "maxalphanames=1 minalphanames=1 entry L8 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=1 minalphanames=1 entry L8 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_019_l9_extraalpha_unset_due_to_shorthand() {
    pass_upstream(
        "L9 extraalpha unset due to shorthand",
        r####"is_undef($main->get_extraalphadata_for_key('L9'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L9')), 'L9 extraalpha unset due to shorthand');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_020_l10_extraalpha_unset_due_to_shorthand() {
    pass_upstream(
        "L10 extraalpha unset due to shorthand",
        r####"is_undef($main->get_extraalphadata_for_key('L10'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L10')), 'L10 extraalpha unset due to shorthand');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_021_year_with_range_needs_label_differentiating_from_individual_volumes_1() {
    pass_upstream(
        "YEAR with range needs label differentiating from individual volumes - 1",
        r####"$main->get_extraalphadata_for_key('knuth:ct')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('knuth:ct'), '1', 'YEAR with range needs label differentiating from individual volumes - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_022_year_with_range_needs_label_differentiating_from_individual_volumes_2() {
    pass_upstream(
        "YEAR with range needs label differentiating from individual volumes - 2",
        r####"$main->get_extraalphadata_for_key('knuth:ct:a')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:a'), '2', 'YEAR with range needs label differentiating from individual volumes - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_023_year_with_range_needs_label_differentiating_from_individual_volumes_3() {
    pass_upstream(
        "YEAR with range needs label differentiating from individual volumes - 3",
        r####"$main->get_extraalphadata_for_key('knuth:ct:b')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:b'), '1', 'YEAR with range needs label differentiating from individual volumes - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_024_year_with_range_needs_label_differentiating_from_individual_volumes_4() {
    pass_upstream(
        "YEAR with range needs label differentiating from individual volumes - 4",
        r####"$main->get_extraalphadata_for_key('knuth:ct:c')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('knuth:ct:c'), '2', 'YEAR with range needs label differentiating from individual volumes - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_025_default_ignore() {
    pass_upstream(
        "Default ignore",
        r####"$main->get_entryfield('ignore1', 'sortlabelalpha')"####,
        r####"'OTo07'"####,
        r####"eq_or_diff($main->get_entryfield('ignore1', 'sortlabelalpha'), 'OTo07', 'Default ignore');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_026_default_no_ignore_spaces() {
    pass_upstream(
        "Default no ignore spaces",
        r####"$main->get_entryfield('ignore2', 'sortlabelalpha')"####,
        r####"'De 07'"####,
        r####"eq_or_diff($main->get_entryfield('ignore2', 'sortlabelalpha'), 'De 07', 'Default no ignore spaces');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_027_maxalphanames_2_minalphanames_1_entry_l1_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L1 labelalpha",
        r####"$main->get_entryfield('L1', 'sortlabelalpha')"####,
        r####"'Doe95'"####,
        r####"eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=1 entry L1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_028_maxalphanames_2_minalphanames_1_entry_l1_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L1 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('l1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=2 minalphanames=1 entry L1 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_029_maxalphanames_2_minalphanames_1_entry_l2_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L2 labelalpha",
        r####"$main->get_entryfield('L2', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L2 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_030_maxalphanames_2_minalphanames_1_entry_l2_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L2 extraalpha",
        r####"$main->get_extraalphadata_for_key('L2')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=1 entry L2 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_031_maxalphanames_2_minalphanames_1_entry_l3_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L3 labelalpha",
        r####"$main->get_entryfield('L3', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=1 entry L3 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_032_maxalphanames_2_minalphanames_1_entry_l3_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L3 extraalpha",
        r####"$main->get_extraalphadata_for_key('L3')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=1 entry L3 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_033_maxalphanames_2_minalphanames_1_entry_l4_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L4 labelalpha",
        r####"$main->get_entryfield('L4', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L4 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_034_maxalphanames_2_minalphanames_1_entry_l4_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L4 extraalpha",
        r####"$main->get_extraalphadata_for_key('L4')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=2 minalphanames=1 entry L4 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_035_maxalphanames_2_minalphanames_1_entry_l5_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L5 labelalpha",
        r####"$main->get_entryfield('L5', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L5 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_036_maxalphanames_2_minalphanames_1_entry_l5_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L5 extraalpha",
        r####"$main->get_extraalphadata_for_key('L5')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=2 minalphanames=1 entry L5 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_037_maxalphanames_2_minalphanames_1_entry_l6_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L6 labelalpha",
        r####"$main->get_entryfield('L6', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L6 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_038_maxalphanames_2_minalphanames_1_entry_l6_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L6 extraalpha",
        r####"$main->get_extraalphadata_for_key('L6')"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L6'), '3', 'maxalphanames=2 minalphanames=1 entry L6 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_039_maxalphanames_2_minalphanames_1_entry_l7_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L7 labelalpha",
        r####"$main->get_entryfield('L7', 'sortlabelalpha')"####,
        r####"'Doe+95'"####,
        r####"eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'Doe+95', 'maxalphanames=2 minalphanames=1 entry L7 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_040_maxalphanames_2_minalphanames_1_entry_l7_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L7 extraalpha",
        r####"$main->get_extraalphadata_for_key('L7')"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L7'), '4', 'maxalphanames=2 minalphanames=1 entry L7 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_041_maxalphanames_2_minalphanames_1_entry_l8_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L8 labelalpha",
        r####"$main->get_entryfield('L8', 'sortlabelalpha')"####,
        r####"'Sha85'"####,
        r####"eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=1 entry L8 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_042_maxalphanames_2_minalphanames_1_entry_l8_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=1 entry L8 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=1 entry L8 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_043_maxalphanames_2_minalphanames_2_entry_l1_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L1 labelalpha",
        r####"$main->get_entryfield('L1', 'sortlabelalpha')"####,
        r####"'Doe95'"####,
        r####"eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=2 minalphanames=2 entry L1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_044_maxalphanames_2_minalphanames_2_entry_l1_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L1 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('l1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('l1')), 'maxalphanames=2 minalphanames=2 entry L1 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_045_maxalphanames_2_minalphanames_2_entry_l2_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L2 labelalpha",
        r####"$main->get_entryfield('L2', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L2 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_046_maxalphanames_2_minalphanames_2_entry_l2_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L2 extraalpha",
        r####"$main->get_extraalphadata_for_key('L2')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=2 minalphanames=2 entry L2 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_047_maxalphanames_2_minalphanames_2_entry_l3_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L3 labelalpha",
        r####"$main->get_entryfield('L3', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=2 minalphanames=2 entry L3 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_048_maxalphanames_2_minalphanames_2_entry_l3_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L3 extraalpha",
        r####"$main->get_extraalphadata_for_key('L3')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=2 minalphanames=2 entry L3 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_049_maxalphanames_2_minalphanames_2_entry_l4_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L4 labelalpha",
        r####"$main->get_entryfield('L4', 'sortlabelalpha')"####,
        r####"'DA+95'"####,
        r####"eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L4 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_050_maxalphanames_2_minalphanames_2_entry_l4_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L4 extraalpha",
        r####"$main->get_extraalphadata_for_key('L4')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=2 minalphanames=2 entry L4 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_051_maxalphanames_2_minalphanames_2_entry_l5_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L5 labelalpha",
        r####"$main->get_entryfield('L5', 'sortlabelalpha')"####,
        r####"'DA+95'"####,
        r####"eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'DA+95', 'maxalphanames=2 minalphanames=2 entry L5 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_052_maxalphanames_2_minalphanames_2_entry_l5_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L5 extraalpha",
        r####"$main->get_extraalphadata_for_key('L5')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=2 minalphanames=2 entry L5 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_053_maxalphanames_2_minalphanames_2_entry_l6_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L6 labelalpha",
        r####"$main->get_entryfield('L6', 'sortlabelalpha')"####,
        r####"'DS+95'"####,
        r####"eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L6 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_054_maxalphanames_2_minalphanames_2_entry_l6_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L6 extraalpha",
        r####"$main->get_extraalphadata_for_key('L6')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L6'), '1', 'maxalphanames=2 minalphanames=2 entry L6 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_055_maxalphanames_2_minalphanames_2_entry_l7_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L7 labelalpha",
        r####"$main->get_entryfield('L7', 'sortlabelalpha')"####,
        r####"'DS+95'"####,
        r####"eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'DS+95', 'maxalphanames=2 minalphanames=2 entry L7 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_056_maxalphanames_2_minalphanames_2_entry_l7_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L7 extraalpha",
        r####"$main->get_extraalphadata_for_key('L7')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L7'), '2', 'maxalphanames=2 minalphanames=2 entry L7 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_057_maxalphanames_2_minalphanames_2_entry_l8_labelalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L8 labelalpha",
        r####"$main->get_entryfield('L8', 'sortlabelalpha')"####,
        r####"'Sha85'"####,
        r####"eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=2 minalphanames=2 entry L8 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_058_maxalphanames_2_minalphanames_2_entry_l8_extraalpha() {
    pass_upstream(
        "maxalphanames=2 minalphanames=2 entry L8 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=2 minalphanames=2 entry L8 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_059_maxalphanames_3_minalphanames_1_entry_l1_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L1 labelalpha",
        r####"$main->get_entryfield('L1', 'sortlabelalpha')"####,
        r####"'Doe95'"####,
        r####"eq_or_diff($main->get_entryfield('L1', 'sortlabelalpha'), 'Doe95', 'maxalphanames=3 minalphanames=1 entry L1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_060_maxalphanames_3_minalphanames_1_entry_l1_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L1 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L1')), 'maxalphanames=3 minalphanames=1 entry L1 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_061_maxalphanames_3_minalphanames_1_entry_l2_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L2 labelalpha",
        r####"$main->get_entryfield('L2', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L2', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L2 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_062_maxalphanames_3_minalphanames_1_entry_l2_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L2 extraalpha",
        r####"$main->get_extraalphadata_for_key('L2')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L2'), '1', 'maxalphanames=3 minalphanames=1 entry L2 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_063_maxalphanames_3_minalphanames_1_entry_l3_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L3 labelalpha",
        r####"$main->get_entryfield('L3', 'sortlabelalpha')"####,
        r####"'DA95'"####,
        r####"eq_or_diff($main->get_entryfield('L3', 'sortlabelalpha'), 'DA95', 'maxalphanames=3 minalphanames=1 entry L3 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_064_maxalphanames_3_minalphanames_1_entry_l3_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L3 extraalpha",
        r####"$main->get_extraalphadata_for_key('L3')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L3'), '2', 'maxalphanames=3 minalphanames=1 entry L3 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_065_maxalphanames_3_minalphanames_1_entry_l4_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L4 labelalpha",
        r####"$main->get_entryfield('L4', 'sortlabelalpha')"####,
        r####"'DAE95'"####,
        r####"eq_or_diff($main->get_entryfield('L4', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L4 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_066_maxalphanames_3_minalphanames_1_entry_l4_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L4 extraalpha",
        r####"$main->get_extraalphadata_for_key('L4')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L4'), '1', 'maxalphanames=3 minalphanames=1 entry L4 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_067_maxalphanames_3_minalphanames_1_entry_l5_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L5 labelalpha",
        r####"$main->get_entryfield('L5', 'sortlabelalpha')"####,
        r####"'DAE95'"####,
        r####"eq_or_diff($main->get_entryfield('L5', 'sortlabelalpha'), 'DAE95', 'maxalphanames=3 minalphanames=1 entry L5 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_068_maxalphanames_3_minalphanames_1_entry_l5_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L5 extraalpha",
        r####"$main->get_extraalphadata_for_key('L5')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L5'), '2', 'maxalphanames=3 minalphanames=1 entry L5 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_069_maxalphanames_3_minalphanames_1_entry_l6_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L6 labelalpha",
        r####"$main->get_entryfield('L6', 'sortlabelalpha')"####,
        r####"'DSE95'"####,
        r####"eq_or_diff($main->get_entryfield('L6', 'sortlabelalpha'), 'DSE95', 'maxalphanames=3 minalphanames=1 entry L6 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_070_maxalphanames_3_minalphanames_1_entry_l6_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L6 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L6'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L6')), 'maxalphanames=3 minalphanames=1 entry L6 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_071_maxalphanames_3_minalphanames_1_entry_l7_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L7 labelalpha",
        r####"$main->get_entryfield('L7', 'sortlabelalpha')"####,
        r####"'DSJ95'"####,
        r####"eq_or_diff($main->get_entryfield('L7', 'sortlabelalpha'), 'DSJ95', 'maxalphanames=3 minalphanames=1 entry L7 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_072_maxalphanames_3_minalphanames_1_entry_l7_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L7 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L7'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L7')), 'maxalphanames=3 minalphanames=1 entry L7 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_073_maxalphanames_3_minalphanames_1_entry_l8_labelalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L8 labelalpha",
        r####"$main->get_entryfield('L8', 'sortlabelalpha')"####,
        r####"'Sha85'"####,
        r####"eq_or_diff($main->get_entryfield('L8', 'sortlabelalpha'), 'Sha85', 'maxalphanames=3 minalphanames=1 entry L8 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_074_maxalphanames_3_minalphanames_1_entry_l8_extraalpha() {
    pass_upstream(
        "maxalphanames=3 minalphanames=1 entry L8 extraalpha",
        r####"is_undef($main->get_extraalphadata_for_key('L8'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extraalphadata_for_key('L8')), 'maxalphanames=3 minalphanames=1 entry L8 extraalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_075_testing_compound_lastnames_1() {
    pass_upstream(
        "Testing compound lastnames 1",
        r####"$main->get_entryfield('LDN1', 'sortlabelalpha')"####,
        r####"'VUR89'"####,
        r####"eq_or_diff($main->get_entryfield('LDN1', 'sortlabelalpha'), 'VUR89', 'Testing compound lastnames 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_076_testing_compound_lastnames_2() {
    pass_upstream(
        "Testing compound lastnames 2",
        r####"$main->get_entryfield('LDN2', 'sortlabelalpha')"####,
        r####"'VU45'"####,
        r####"eq_or_diff($main->get_entryfield('LDN2', 'sortlabelalpha'), 'VU45', 'Testing compound lastnames 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_077_testing_with_multiple_pre_and_main_and_width_side_override() {
    pass_upstream(
        "Testing with multiple pre and main and width/side override",
        r####"$main->get_entryfield('LDN3', 'sortlabelalpha')"####,
        r####"'VisvSJRu45'"####,
        r####"eq_or_diff($main->get_entryfield('LDN3', 'sortlabelalpha'), 'VisvSJRu45', 'Testing with multiple pre and main and width/side override');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_078_prefix_labelalpha_1() {
    pass_upstream(
        "prefix labelalpha 1",
        r####"$main->get_entryfield('L11', 'sortlabelalpha')"####,
        r####"'vRan22'"####,
        r####"eq_or_diff($main->get_entryfield('L11', 'sortlabelalpha'), 'vRan22', 'prefix labelalpha 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_079_prefix_labelalpha_2() {
    pass_upstream(
        "prefix labelalpha 2",
        r####"$main->get_entryfield('L12', 'sortlabelalpha')"####,
        r####"'vRvB2'"####,
        r####"eq_or_diff($main->get_entryfield('L12', 'sortlabelalpha'), 'vRvB2', 'prefix labelalpha 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_080_per_type_labelalpha_1() {
    pass_upstream(
        "per-type labelalpha 1",
        r####"$main->get_entryfield('L13', 'sortlabelalpha')"####,
        r####"'vRa+-ksUnV'"####,
        r####"eq_or_diff($main->get_entryfield('L13', 'sortlabelalpha'), 'vRa+-ksUnV', 'per-type labelalpha 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_081_per_type_labelalpha_2() {
    pass_upstream(
        "per-type labelalpha 2",
        r####"$main->get_entryfield('L14', 'sortlabelalpha')"####,
        r####"'Alabel-ksUnW'"####,
        r####"eq_or_diff($main->get_entryfield('L14', 'sortlabelalpha'), 'Alabel-ksUnW', 'per-type labelalpha 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_082_labelalpha_disambiguation_1() {
    pass_upstream(
        "labelalpha disambiguation 1",
        r####"$main->get_entryfield('L15', 'sortlabelalpha')"####,
        r####"'AccBrClim'"####,
        r####"eq_or_diff($main->get_entryfield('L15', 'sortlabelalpha'), 'AccBrClim', 'labelalpha disambiguation 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_083_labelalpha_disambiguation_2() {
    pass_upstream(
        "labelalpha disambiguation 2",
        r####"$main->get_entryfield('L16', 'sortlabelalpha')"####,
        r####"'AccBaClim'"####,
        r####"eq_or_diff($main->get_entryfield('L16', 'sortlabelalpha'), 'AccBaClim', 'labelalpha disambiguation 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_084_labelalpha_disambiguation_2a() {
    pass_upstream(
        "labelalpha disambiguation 2a",
        r####"$main->get_entryfield('L16a', 'sortlabelalpha')"####,
        r####"'AccBaClim'"####,
        r####"eq_or_diff($main->get_entryfield('L16a', 'sortlabelalpha'), 'AccBaClim', 'labelalpha disambiguation 2a');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_085_labelalpha_disambiguation_2c() {
    pass_upstream(
        "labelalpha disambiguation 2c",
        r####"$main->get_extraalphadata_for_key('L16')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L16'), '1', 'labelalpha disambiguation 2c');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_086_labelalpha_disambiguation_2d() {
    pass_upstream(
        "labelalpha disambiguation 2d",
        r####"$main->get_extraalphadata_for_key('L16a')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L16a'), '2', 'labelalpha disambiguation 2d');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_087_labelalpha_disambiguation_3() {
    pass_upstream(
        "labelalpha disambiguation 3",
        r####"$main->get_entryfield('L17', 'sortlabelalpha')"####,
        r####"'AckBaClim'"####,
        r####"eq_or_diff($main->get_entryfield('L17', 'sortlabelalpha'), 'AckBaClim', 'labelalpha disambiguation 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_088_custom_labelalpha_extradate_1() {
    pass_upstream(
        "custom labelalpha extradate 1",
        r####"$main->get_extraalphadata_for_key('L17a')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('L17a'), '2', 'custom labelalpha extradate 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_089_labelalpha_disambiguation_4() {
    pass_upstream(
        "labelalpha disambiguation 4",
        r####"$main->get_entryfield('L18', 'sortlabelalpha')"####,
        r####"'AgChLa'"####,
        r####"eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'AgChLa', 'labelalpha disambiguation 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_090_labelalpha_disambiguation_5() {
    pass_upstream(
        "labelalpha disambiguation 5",
        r####"$main->get_entryfield('L19', 'sortlabelalpha')"####,
        r####"'AgConLe'"####,
        r####"eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'AgConLe', 'labelalpha disambiguation 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_091_labelalpha_disambiguation_6() {
    pass_upstream(
        "labelalpha disambiguation 6",
        r####"$main->get_entryfield('L20', 'sortlabelalpha')"####,
        r####"'AgCouLa'"####,
        r####"eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'AgCouLa', 'labelalpha disambiguation 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_092_labelalpha_disambiguation_7() {
    pass_upstream(
        "labelalpha disambiguation 7",
        r####"$main->get_entryfield('L21', 'sortlabelalpha')"####,
        r####"'BoConEdb'"####,
        r####"eq_or_diff($main->get_entryfield('L21', 'sortlabelalpha'), 'BoConEdb', 'labelalpha disambiguation 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_093_labelalpha_disambiguation_8() {
    pass_upstream(
        "labelalpha disambiguation 8",
        r####"$main->get_entryfield('L22', 'sortlabelalpha')"####,
        r####"'BoConEm'"####,
        r####"eq_or_diff($main->get_entryfield('L22', 'sortlabelalpha'), 'BoConEm', 'labelalpha disambiguation 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_094_labelalpha_disambiguation_9() {
    pass_upstream(
        "labelalpha disambiguation 9",
        r####"$main->get_entryfield('L23', 'sortlabelalpha')"####,
        r####"'Sa'"####,
        r####"eq_or_diff($main->get_entryfield('L23', 'sortlabelalpha'), 'Sa', 'labelalpha disambiguation 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_095_labelalpha_disambiguation_10() {
    pass_upstream(
        "labelalpha disambiguation 10",
        r####"$main->get_entryfield('L18', 'sortlabelalpha')"####,
        r####"'Agas/Cha/Laver'"####,
        r####"eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'Agas/Cha/Laver', 'labelalpha disambiguation 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_096_labelalpha_disambiguation_11() {
    pass_upstream(
        "labelalpha disambiguation 11",
        r####"$main->get_entryfield('L19', 'sortlabelalpha')"####,
        r####"'Agas/Con/Lendl'"####,
        r####"eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'Agas/Con/Lendl', 'labelalpha disambiguation 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_097_labelalpha_disambiguation_12() {
    pass_upstream(
        "labelalpha disambiguation 12",
        r####"$main->get_entryfield('L20', 'sortlabelalpha')"####,
        r####"'Agas/Cou/Laver'"####,
        r####"eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'Agas/Cou/Laver', 'labelalpha disambiguation 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_098_labelalpha_list_disambiguation_1() {
    pass_upstream(
        "labelalpha list disambiguation 1",
        r####"$main->get_entryfield('L18', 'sortlabelalpha')"####,
        r####"'AChL'"####,
        r####"eq_or_diff($main->get_entryfield('L18', 'sortlabelalpha'), 'AChL', 'labelalpha list disambiguation 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_099_labelalpha_list_disambiguation_2() {
    pass_upstream(
        "labelalpha list disambiguation 2",
        r####"$main->get_entryfield('L19', 'sortlabelalpha')"####,
        r####"'ACoL'"####,
        r####"eq_or_diff($main->get_entryfield('L19', 'sortlabelalpha'), 'ACoL', 'labelalpha list disambiguation 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_100_labelalpha_list_disambiguation_3() {
    pass_upstream(
        "labelalpha list disambiguation 3",
        r####"$main->get_entryfield('L20', 'sortlabelalpha')"####,
        r####"'ACL'"####,
        r####"eq_or_diff($main->get_entryfield('L20', 'sortlabelalpha'), 'ACL', 'labelalpha list disambiguation 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_101_labelalpha_list_disambiguation_4() {
    pass_upstream(
        "labelalpha list disambiguation 4",
        r####"$main->get_entryfield('L21', 'sortlabelalpha')"####,
        r####"'BCEd'"####,
        r####"eq_or_diff($main->get_entryfield('L21', 'sortlabelalpha'), 'BCEd', 'labelalpha list disambiguation 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_102_labelalpha_list_disambiguation_5() {
    pass_upstream(
        "labelalpha list disambiguation 5",
        r####"$main->get_entryfield('L22', 'sortlabelalpha')"####,
        r####"'BCE'"####,
        r####"eq_or_diff($main->get_entryfield('L22', 'sortlabelalpha'), 'BCE', 'labelalpha list disambiguation 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_103_labelalpha_list_disambiguation_6() {
    pass_upstream(
        "labelalpha list disambiguation 6",
        r####"$main->get_entryfield('L24', 'sortlabelalpha')"####,
        r####"'Z'"####,
        r####"eq_or_diff($main->get_entryfield('L24', 'sortlabelalpha'), 'Z', 'labelalpha list disambiguation 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_104_labelalpha_list_disambiguation_7() {
    pass_upstream(
        "labelalpha list disambiguation 7",
        r####"$main->get_entryfield('L25', 'sortlabelalpha')"####,
        r####"'ZX'"####,
        r####"eq_or_diff($main->get_entryfield('L25', 'sortlabelalpha'), 'ZX', 'labelalpha list disambiguation 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_105_labelalpha_list_disambiguation_8() {
    pass_upstream(
        "labelalpha list disambiguation 8",
        r####"$main->get_entryfield('L26', 'sortlabelalpha')"####,
        r####"'ZX'"####,
        r####"eq_or_diff($main->get_entryfield('L26', 'sortlabelalpha'), 'ZX', 'labelalpha list disambiguation 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_106_title_in_braces_with_utf_8_char_1() {
    pass_upstream(
        "Title in braces with UTF-8 char - 1",
        r####"NFC($main->get_entryfield('title1', 'sortlabelalpha'))"####,
        r####"'Tït'"####,
        r####"eq_or_diff(NFC($main->get_entryfield('title1', 'sortlabelalpha')), 'Tït', 'Title in braces with UTF-8 char - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_107_extraalpha_ne_extradate_1() {
    pass_upstream(
        "extraalpha ne extradate 1",
        r####"$main->get_entryfield('Schmidt2007', 'sortlabelalpha')"####,
        r####"'Sch+07'"####,
        r####"eq_or_diff($main->get_entryfield('Schmidt2007', 'sortlabelalpha'), 'Sch+07', 'extraalpha ne extradate 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_108_extraalpha_ne_extradate_2() {
    pass_upstream(
        "extraalpha ne extradate 2",
        r####"$main->get_extraalphadata_for_key('Schmidt2007')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('Schmidt2007'), '1', 'extraalpha ne extradate 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_109_extraalpha_ne_extradate_3() {
    pass_upstream(
        "extraalpha ne extradate 3",
        r####"$main->get_entryfield('Schmidt2007a', 'sortlabelalpha')"####,
        r####"'Sch07'"####,
        r####"eq_or_diff($main->get_entryfield('Schmidt2007a', 'sortlabelalpha'), 'Sch07', 'extraalpha ne extradate 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_110_extraalpha_ne_extradate_4() {
    pass_upstream(
        "extraalpha ne extradate 4",
        r####"$main->get_extraalphadata_for_key('Schmidt2007a')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('Schmidt2007a'), '1', 'extraalpha ne extradate 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_111_extraalpha_ne_extradate_5() {
    pass_upstream(
        "extraalpha ne extradate 5",
        r####"$main->get_entryfield('Schnee2007', 'sortlabelalpha')"####,
        r####"'Sch+07'"####,
        r####"eq_or_diff($main->get_entryfield('Schnee2007', 'sortlabelalpha'), 'Sch+07', 'extraalpha ne extradate 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_112_extraalpha_ne_extradate_6() {
    pass_upstream(
        "extraalpha ne extradate 6",
        r####"$main->get_extraalphadata_for_key('Schnee2007')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('Schnee2007'), '2', 'extraalpha ne extradate 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_113_extraalpha_ne_extradate_7() {
    pass_upstream(
        "extraalpha ne extradate 7",
        r####"$main->get_entryfield('Schnee2007a', 'sortlabelalpha')"####,
        r####"'Sch07'"####,
        r####"eq_or_diff($main->get_entryfield('Schnee2007a', 'sortlabelalpha'), 'Sch07', 'extraalpha ne extradate 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_114_extraalpha_ne_extradate_8() {
    pass_upstream(
        "extraalpha ne extradate 8",
        r####"$main->get_extraalphadata_for_key('Schnee2007a')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extraalphadata_for_key('Schnee2007a'), '2', 'extraalpha ne extradate 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_115_entrykey_label_1() {
    pass_upstream(
        "entrykey label 1",
        r####"$main->get_entryfield('Schmidt2007', 'sortlabelalpha')"####,
        r####"'SCH'"####,
        r####"eq_or_diff($main->get_entryfield('Schmidt2007', 'sortlabelalpha'), 'SCH', 'entrykey label 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_116_labeldate_test_1() {
    pass_upstream(
        "labeldate test - 1",
        r####"$main->get_entryfield('labelstest', 'sortlabelalpha')"####,
        r####"'200532'"####,
        r####"eq_or_diff($main->get_entryfield('labelstest', 'sortlabelalpha'), '200532', 'labeldate test - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_117_pad_test_1() {
    pass_upstream(
        "pad test - 1",
        r####"$main->get_entryfield('padtest', 'labelalpha')"####,
        r####"'\&Al\_\_{\textasciitilde}{\textasciitilde}T07'"####,
        r####"eq_or_diff($main->get_entryfield('padtest', 'labelalpha'), '\&Al\_\_{\textasciitilde}{\textasciitilde}T07', 'pad test - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_118_pad_test_2() {
    pass_upstream(
        "pad test - 2",
        r####"$main->get_entryfield('padtest', 'sortlabelalpha')"####,
        r####"'&Al__~~T07'"####,
        r####"eq_or_diff($main->get_entryfield('padtest', 'sortlabelalpha'), '&Al__~~T07', 'pad test - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_119_skip_width_test_1() {
    pass_upstream(
        "Skip width test - 1",
        r####"$main->get_entryfield('skipwidthtest1', 'sortlabelalpha')"####,
        r####"'OToolOToole'"####,
        r####"eq_or_diff($main->get_entryfield('skipwidthtest1', 'sortlabelalpha'), 'OToolOToole', 'Skip width test - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_120_compound_and_string_length_entry_prefix1_labelalpha() {
    pass_upstream(
        "compound and string length entry prefix1 labelalpha",
        r####"$main->get_entryfield('prefix1', 'sortlabelalpha')"####,
        r####"'vadeVaaThin'"####,
        r####"eq_or_diff($main->get_entryfield('prefix1', 'sortlabelalpha'), 'vadeVaaThin', 'compound and string length entry prefix1 labelalpha');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_121_name_range_test_1() {
    pass_upstream(
        "Name range test - 1",
        r####"$main->get_entryfield('rangetest1', 'sortlabelalpha')"####,
        r####"'WAXAYAZA.VEWEXE+.VTWT.XFYFZF.WH+'"####,
        r####"eq_or_diff($main->get_entryfield('rangetest1', 'sortlabelalpha'), 'WAXAYAZA.VEWEXE+.VTWT.XFYFZF.WH+', 'Name range test - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public label-alpha data query API"]
fn assertion_122_name_range_test_2() {
    pass_upstream(
        "Name range test - 2",
        r####"$main->get_entryfield('rangetest1', 'sortlabelalpha')"####,
        r####"'VWXYZ..V/W/X/Y/Z'"####,
        r####"eq_or_diff($main->get_entryfield('rangetest1', 'sortlabelalpha'), 'VWXYZ..V/W/X/Y/Z', 'Name range test - 2');"####,
        UPSTREAM_SOURCE,
    );
}
