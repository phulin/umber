// Direct xfail translation of upstream t/sections.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::pass_upstream;

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 14;
use Test::Differences;
unified_diff;

use Biber;
use Biber::Constants;
use Biber::Utils;
use Biber::Output::bbl;
use Unicode::Normalize;
use Log::Log4perl;
chdir("t/tdata");

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

$biber->parse_ctrlfile('sections.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
Biber::Config->setoption('output_safechars', 1);

# Now generate the information
$biber->prepare;
my $out = $biber->get_output_obj;
my $section0 = $biber->sections->get_section(0);
my $main0 = $biber->datalists->get_list('custom/global//global/global/global');
my $shs0 = $biber->datalists->get_list('shorthand/global//global/global/global', 0, 'list');

my $section1 = $biber->sections->get_section(1);
my $main1 = $biber->datalists->get_list('custom/global//global/global/global', 1);
my $shs1 = $biber->datalists->get_list('shorthand/global//global/global/global', 1, 'list');

my $section2 = $biber->sections->get_section(2);
my $main2 = $biber->datalists->get_list('custom/global//global/global/global', 2);
my $shs2 = $biber->datalists->get_list('shorthand/global//global/global/global', 2, 'list');

my $section3 = $biber->sections->get_section(3);
my $main3 = $biber->datalists->get_list('custom/global//global/global/global', 3);
my $shs3 = $biber->datalists->get_list('shorthand/global//global/global/global', 3, 'list');

# Internal UTF-8 before output is always NFD so have to NFD bits of this
my $preamble = [
                NFD('Štring for Preamble 1'),
                'String for Preamble 2',
                'String for Preamble 3',
                'String for Preamble 4'
               ];

my $v = $Biber::Config::VERSION;
if ($Biber::Config::BETA_VERSION) {
  $v .= ' (beta)';
}

my $head = qq|% \$ biblatex auxiliary file \$
% \$ biblatex bbl format version $BBL_VERSION \$
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\\begingroup
\\makeatletter
\\\@ifundefined{ver\@biblatex.sty}
  {\\\@latex\@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \\aftergroup\\endinput}
  {}
\\endgroup

\\preamble{%
\\v{S}tring for Preamble 1%
String for Preamble 2%
String for Preamble 3%
String for Preamble 4%
}

|;

my $tail = qq||;

is_deeply($biber->get_preamble, $preamble, 'Preamble for all sections');
eq_or_diff($section0->bibentry('sect1')->get_field('note'), 'value1', 'Section 0 macro test');
# If macros were not reset between sections, this would give a macro redef error
eq_or_diff($section1->bibentry('sect4')->get_field('note'), 'value2', 'Section 1 macro test');
is_deeply($main0->get_keys, ['sect1', 'sect2', 'sect3', 'sect8'], 'Section 0 citekeys');
is_deeply($shs0->get_keys, ['sect1', 'sect2', 'sect8'], 'Section 0 shorthands');
is_deeply($main1->get_keys, ['sect4', 'sect5'], 'Section 1 citekeys');
is_deeply($shs1->get_keys, ['sect4', 'sect5'], 'Section 1 shorthands');
is_deeply($main2->get_keys, ['sect1', 'sect6', 'sect7'], 'Section 2 citekeys');
is_deeply($shs2->get_keys, ['sect1', 'sect6', 'sect7'], 'Section 2 shorthands');
is_deeply([$section3->get_orig_order_citekeys], ['sect1', 'sect2', 'sectall1'], 'Section 3 citekeys');
eq_or_diff($out->get_output_section(0)->number, '0', 'Checking output sections - 1');
eq_or_diff($out->get_output_section(1)->number, '1', 'Checking output sections - 2');
eq_or_diff($out->get_output_section(2)->number, '2', 'Checking output sections - 3');
eq_or_diff($out->get_output_head, $head, 'Preamble output check with output_safechars');
"#;

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_001_preamble_for_all_sections() {
    pass_upstream(
        "Preamble for all sections",
        r"$biber->get_preamble",
        r"$preamble",
        r"is_deeply($biber->get_preamble, $preamble, 'Preamble for all sections');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_002_section_0_macro_test() {
    pass_upstream(
        "Section 0 macro test",
        r"$section0->bibentry('sect1')->get_field('note')",
        r"'value1'",
        r"eq_or_diff($section0->bibentry('sect1')->get_field('note'), 'value1', 'Section 0 macro test');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_003_section_1_macro_test() {
    pass_upstream(
        "Section 1 macro test",
        r"$section1->bibentry('sect4')->get_field('note')",
        r"'value2'",
        r"eq_or_diff($section1->bibentry('sect4')->get_field('note'), 'value2', 'Section 1 macro test');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_004_section_0_citekeys() {
    pass_upstream(
        "Section 0 citekeys",
        r"$main0->get_keys",
        r"['sect1', 'sect2', 'sect3', 'sect8']",
        r"is_deeply($main0->get_keys, ['sect1', 'sect2', 'sect3', 'sect8'], 'Section 0 citekeys');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_005_section_0_shorthands() {
    pass_upstream(
        "Section 0 shorthands",
        r"$shs0->get_keys",
        r"['sect1', 'sect2', 'sect8']",
        r"is_deeply($shs0->get_keys, ['sect1', 'sect2', 'sect8'], 'Section 0 shorthands');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_006_section_1_citekeys() {
    pass_upstream(
        "Section 1 citekeys",
        r"$main1->get_keys",
        r"['sect4', 'sect5']",
        r"is_deeply($main1->get_keys, ['sect4', 'sect5'], 'Section 1 citekeys');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_007_section_1_shorthands() {
    pass_upstream(
        "Section 1 shorthands",
        r"$shs1->get_keys",
        r"['sect4', 'sect5']",
        r"is_deeply($shs1->get_keys, ['sect4', 'sect5'], 'Section 1 shorthands');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_008_section_2_citekeys() {
    pass_upstream(
        "Section 2 citekeys",
        r"$main2->get_keys",
        r"['sect1', 'sect6', 'sect7']",
        r"is_deeply($main2->get_keys, ['sect1', 'sect6', 'sect7'], 'Section 2 citekeys');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_009_section_2_shorthands() {
    pass_upstream(
        "Section 2 shorthands",
        r"$shs2->get_keys",
        r"['sect1', 'sect6', 'sect7']",
        r"is_deeply($shs2->get_keys, ['sect1', 'sect6', 'sect7'], 'Section 2 shorthands');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_010_section_3_citekeys() {
    pass_upstream(
        "Section 3 citekeys",
        r"[$section3->get_orig_order_citekeys]",
        r"['sect1', 'sect2', 'sectall1']",
        r"is_deeply([$section3->get_orig_order_citekeys], ['sect1', 'sect2', 'sectall1'], 'Section 3 citekeys');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_011_checking_output_sections_1() {
    pass_upstream(
        "Checking output sections - 1",
        r"$out->get_output_section(0)->number",
        r"'0'",
        r"eq_or_diff($out->get_output_section(0)->number, '0', 'Checking output sections - 1');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_012_checking_output_sections_2() {
    pass_upstream(
        "Checking output sections - 2",
        r"$out->get_output_section(1)->number",
        r"'1'",
        r"eq_or_diff($out->get_output_section(1)->number, '1', 'Checking output sections - 2');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_013_checking_output_sections_3() {
    pass_upstream(
        "Checking output sections - 3",
        r"$out->get_output_section(2)->number",
        r"'2'",
        r"eq_or_diff($out->get_output_section(2)->number, '2', 'Checking output sections - 3');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}

#[test]
#[ignore = "xfail: public bib-engine lacks exact Biber section processing parity"]
fn assertion_014_preamble_output_check_with_output_safechars() {
    pass_upstream(
        "Preamble output check with output_safechars",
        r"$out->get_output_head",
        r"$head",
        r"eq_or_diff($out->get_output_head, $head, 'Preamble output check with output_safechars');",
        UPSTREAM_SOURCE,
    );
    panic!("xfail: public bib-engine lacks exact Biber section processing parity");
}
