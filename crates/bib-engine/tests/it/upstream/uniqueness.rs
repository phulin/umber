// Direct translation of upstream t/uniqueness.t at commit 74252e6.
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
    panic!("xfail: bib-engine has no public name/list uniqueness query API");
}

const UPSTREAM_SOURCE: &str = r####"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 227;
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

$biber->parse_ctrlfile('uniqueness1.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'true');

# Now generate the information
$biber->prepare;
my $section = $biber->sections->get_section(0);
my $bibentries = $section->bibentries;
my $main = $biber->datalists->get_list('nty/global//global/global/global');

# Basic uniquename and hash testing
eq_or_diff($main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id,$bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id,$bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id,$bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 3');
ok(is_undef($main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id,$bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id)), 'Uniquename requiring initials name expansion (per-namelist uniquename) - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id,$bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename requiring initials name expansion - 2');
ok(is_undef($main->get_unsummary($bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->get_id,$bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->nth_name(1)->get_id)), 'per-entry uniquename');
eq_or_diff($main->get_entryfield('un6', 'namehash'), 'f8169a157f8d9209961157b8d23902db', 'Namehash and fullhash - 1');
eq_or_diff($main->get_entryfield('un6', 'fullhash'), 'f8169a157f8d9209961157b8d23902db', 'Namehash and fullhash - 2');
eq_or_diff($main->get_entryfield('un7', 'namehash'), 'b33fbd3f3349d1536dbcc14664f2cbbd', 'Fullnamehash ignores SHORT* names - 1');
eq_or_diff($main->get_entryfield('un7', 'fullhash'), 'f8169a157f8d9209961157b8d23902db', 'Fullnamehash ignores SHORT* names - 2');
eq_or_diff($main->get_entryfield('test1', 'namehash'), '07df5c892ba1452776abee0a867591f2', 'Namehash and fullhash - 3');
eq_or_diff($main->get_entryfield('test1', 'fullhash'), '637292dd2997a74c91847f1ec5081a46', 'Namehash and fullhash - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->get_id,$bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->get_id,$bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->get_id,$bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 3');
# Prefix/suffix
eq_or_diff($main->get_unsummary($bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->get_id,$bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->get_id,$bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->get_id,$bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->get_id,$bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->get_id,$bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 5');
eq_or_diff($main->get_unsummary($bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->get_id,$bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->nth_name(1)->get_id), '2', 'Prefix/Suffix - 6');
eq_or_diff($main->get_unsummary($bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->get_id,$bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->nth_name(1)->get_id), '2', 'Prefix/Suffix - 7');
eq_or_diff($main->get_unsummary($bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->get_id,$bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 8');
eq_or_diff($main->get_unsummary($bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->get_id,$bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 9');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness1.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'init');
Biber::Config->setblxoption(undef,'uniquelist', 'true');

# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->get_id,$bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->get_id,$bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->get_id,$bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->get_id,$bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename with inits and repeat - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->get_id,$bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename with inits and repeat - 5');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness2.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 5);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'true');

# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

# Hashes the same as uniquelist expansion expands to the whole list
eq_or_diff($main->get_entryfield('unall3', 'namehash'), 'f1c5973adbc2e674fa4d98164c9ba5d5', 'Namehash and fullhash - 5');
eq_or_diff($main->get_entryfield('unall3', 'fullhash'), 'f1c5973adbc2e674fa4d98164c9ba5d5', 'Namehash and fullhash - 6');
ok(is_undef($main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id)), 'Uniquelist edgecase - 1');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id), '6', 'Uniquelist edgecase - 2');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness1.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'init');
Biber::Config->setblxoption(undef,'uniquelist', 'false');
# Now generate the information
$biber->prepare;
$bibentries = $biber->sections->get_section('0')->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

ok(is_undef($main->get_unsummary($bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->get_id,$bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->nth_name(1)->get_id)), 'Uniquename 0 due to mincitenames truncation');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness2.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'init');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'mincitenames', 1);
# Now generate the information
$biber->prepare;
$bibentries = $biber->sections->get_section('0')->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Uniquename - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename - 5');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Uniquename - 6');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '0', 'Uniquename - 7');
eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 8');

eq_or_diff($main->get_uniquelist($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id), '3', 'Uniquelist - 1');
eq_or_diff($main->get_uniquelist($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id), '3', 'Uniquelist - 2');
ok(is_undef($main->get_uniquelist($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id)), 'Uniquelist - 3');
eq_or_diff($main->get_uniquelist($bibentries->entry('unapa1')->get_field($bibentries->entry('unapa1')->get_labelname_info)->get_id), '3', 'Uniquelist - 4');
eq_or_diff($main->get_uniquelist($bibentries->entry('unapa2')->get_field($bibentries->entry('unapa2')->get_labelname_info)->get_id), '3', 'Uniquelist - 5');
ok(is_undef($main->get_uniquelist($bibentries->entry('others1')->get_field($bibentries->entry('others1')->get_labelname_info)->get_id)), 'Uniquelist - 6');

# These next two should have uniquelist undef as they are identical author lists and so
# can't be disambiguated (and shouldn't be).
ok(is_undef($main->get_uniquelist($bibentries->entry('unall1')->get_field($bibentries->entry('unall1')->get_labelname_info)->get_id)), 'Uniquelist - 7');
ok(is_undef($main->get_uniquelist($bibentries->entry('unall2')->get_field($bibentries->entry('unall2')->get_labelname_info)->get_id)), 'Uniquelist - 8');

# These all should have uniquelist=5 as even though two are identical, they still both
# need disambiguating from the other one which differs in fifth place
eq_or_diff($main->get_uniquelist($bibentries->entry('unall5')->get_field($bibentries->entry('unall5')->get_labelname_info)->get_id), '5', 'Uniquelist - 9');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall6')->get_field($bibentries->entry('unall6')->get_labelname_info)->get_id), '5', 'Uniquelist - 10');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall7')->get_field($bibentries->entry('unall7')->get_labelname_info)->get_id), '5', 'Uniquelist - 11');
# unall8/unall9 are the same (ul=5) and unall10 is superset of them (ul=6)
# unall9a  is ul=undef due to per-entry settings (would otherwise be ul=5)
eq_or_diff($main->get_uniquelist($bibentries->entry('unall8')->get_field($bibentries->entry('unall8')->get_labelname_info)->get_id), '5', 'Uniquelist - 12');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall9')->get_field($bibentries->entry('unall9')->get_labelname_info)->get_id), '5', 'Uniquelist - 13');
ok(is_undef($main->get_uniquelist($bibentries->entry('unall9a')->get_field($bibentries->entry('unall9a')->get_labelname_info)->get_id)), 'Per-namelist Uniquelist - 1');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall10')->get_field($bibentries->entry('unall10')->get_labelname_info)->get_id), '6', 'Uniquelist - 14');

# These next two should have uniquelist 5/6 as they need disambiguating in place 5
eq_or_diff($main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id), '5', 'Uniquelist - 15');
eq_or_diff($main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id), '6', 'Uniquelist - 16');

# Testing "et al" counting as a uniquelist position
# ul01 = 3
# ul02 = 3 (because it will be "XXX and YYY and ZZZ et al" which disambiguated the list from
# "XXX and YYY and ZZZ"
eq_or_diff($main->get_uniquelist($bibentries->entry('ul01')->get_field($bibentries->entry('ul01')->get_labelname_info)->get_id), '3', 'Uniquelist - 17');
eq_or_diff($main->get_uniquelist($bibentries->entry('ul02')->get_field($bibentries->entry('ul02')->get_labelname_info)->get_id), '3', 'Uniquelist - 18');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness1.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
# Now generate the information
$biber->prepare;
$bibentries = $biber->sections->get_section('0')->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_uniquelist($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id), '2', 'Uniquelist - 19');
eq_or_diff($main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 9');
eq_or_diff($main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename - 10');

eq_or_diff($main->get_uniquelist($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id), '2', 'Uniquelist - 20');
eq_or_diff($main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 11');
eq_or_diff($main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename - 12');

eq_or_diff($main->get_uniquelist($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id), '2', 'Uniquelist - 21');
eq_or_diff($main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 13');
eq_or_diff($main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename - 14');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness4.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 3);
Biber::Config->setblxoption(undef,'uniquename', 'minfull');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 5');
eq_or_diff($main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 6');
eq_or_diff($main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 7');
eq_or_diff($main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 8');
eq_or_diff($main->get_unsummary($bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->get_id,$bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 9');
eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 10');
eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 11');
eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 12');
eq_or_diff($main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 13');
eq_or_diff($main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 14');
eq_or_diff($main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 15');
eq_or_diff($main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 16');
eq_or_diff($main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 17');
eq_or_diff($main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 18');
eq_or_diff($main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 19');
eq_or_diff($main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename sparse - 20');
eq_or_diff($main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 21');
eq_or_diff($main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename sparse - 22');
eq_or_diff($main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 23');
eq_or_diff($main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 24');
eq_or_diff($main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 25');
eq_or_diff($main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 26');

# maxcitenames/mincitenames is 3 in but us14 is still "et al" so it's a "different list
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 27');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 28');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 29');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 30');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 31');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 32');

eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 33');
eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 34');
eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 35');
ok(is_undef($main->get_uniquelist($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id)), 'Uniquename sparse - 36');
eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 37');
eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 38');
eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 39');
eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(4)->get_id), '0', 'Uniquename sparse - 40');
eq_or_diff($main->get_uniquelist($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id), '4', 'Uniquename sparse - 41');
eq_or_diff($main->get_unsummary($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id,$bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 42');
eq_or_diff($main->get_unsummary($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id,$bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 43');
ok(is_undef($main->get_uniquelist($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id)), 'Uniquename sparse - 44');
eq_or_diff($main->get_uniquelist($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id), '4', 'Uniquename sparse - 45');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness4.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'minfull');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

# maxcitenames/mincitenames = 3/1 so these will not truncate to the same list (since
# us15 would not be truncated at all) and they therefore would not need disambiguating with
# uniquename = 5 or 6
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 46');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 47');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 48');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 49');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 50');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 51');

#
eq_or_diff($main->get_unsummary($bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->get_id,$bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 52');
eq_or_diff($main->get_unsummary($bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->get_id,$bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 53');
eq_or_diff($main->get_unsummary($bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->get_id,$bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 54');
eq_or_diff($main->get_unsummary($bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->get_id,$bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 55');
eq_or_diff($main->get_unsummary($bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->get_id,$bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 56');
eq_or_diff($main->get_unsummary($bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->get_id,$bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 57');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness4.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 2);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'minfull');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

# maxcitenames/mincitenames = 2/1 so list are the same and need disambiguating but only in the first
# name as the others are not visible

eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 58');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 59');
eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 60');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 61');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 62');
eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 63');
eq_or_diff($main->get_unsummary($bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->get_id,$bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 64');
eq_or_diff($main->get_unsummary($bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->get_id,$bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 65');
eq_or_diff($main->get_unsummary($bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->get_id,$bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 66');
eq_or_diff($main->get_unsummary($bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->get_id,$bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 67');
eq_or_diff($main->get_unsummary($bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->get_id,$bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 68');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness5.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 1);
Biber::Config->setblxoption(undef,'mincitenames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'minyear');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

ok(is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id)), 'Uniquelist strict - 1');
ok(is_undef($main->get_uniquelist($bibentries->entry('uls2')->get_field($bibentries->entry('uls2')->get_labelname_info)->get_id)), 'Uniquelist strict - 2');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls3')->get_field($bibentries->entry('uls3')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 3');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls4')->get_field($bibentries->entry('uls4')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 4');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls5')->get_field($bibentries->entry('uls5')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 5');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls6')->get_field($bibentries->entry('uls6')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 6');
ok(is_undef($main->get_uniquelist($bibentries->entry('uls7')->get_field($bibentries->entry('uls7')->get_labelname_info)->get_id)), 'Uniquelist strict - 7');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness5.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxnames', 3);
Biber::Config->setblxoption(undef,'minnames', 1);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'minyear');
Biber::Config->setblxoption(undef,'labeldateparts', 'true');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_uniquelist($bibentries->entry('ulmy1')->get_field($bibentries->entry('ulmy1')->get_labelname_info)->get_id), '2', 'Uniquelist minyear - 1');
eq_or_diff($main->get_uniquelist($bibentries->entry('ulmy2')->get_field($bibentries->entry('ulmy2')->get_labelname_info)->get_id), '2', 'Uniquelist minyear - 2');
ok(is_undef($main->get_uniquelist($bibentries->entry('ulmy3')->get_field($bibentries->entry('ulmy3')->get_labelname_info)->get_id)), 'Uniquelist minyear - 3');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness5.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 2);
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'minyear');
Biber::Config->setblxoption(undef,'singletitle', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

ok(is_undef($main->get_uniquelist($bibentries->entry('uls8')->get_field($bibentries->entry('uls8')->get_labelname_info)->get_id)), 'Uniquelist strict - 8');
ok(is_undef($main->get_uniquelist($bibentries->entry('uls9')->get_field($bibentries->entry('uls9')->get_labelname_info)->get_id)),'Uniquelist strict - 9');
ok(is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id)),'Uniquelist strict - 10');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls10')->get_field($bibentries->entry('uls10')->get_labelname_info)->get_id), '3', 'Uniquelist strict - 11');
eq_or_diff($main->get_uniquelist($bibentries->entry('uls11')->get_field($bibentries->entry('uls11')->get_labelname_info)->get_id), '3', 'Uniquelist strict - 12');
ok(is_undef($main->get_uniquelist($bibentries->entry('uls12')->get_field($bibentries->entry('uls12')->get_labelname_info)->get_id)), 'Uniquelist strict - 13');


#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness3.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'init');
Biber::Config->setblxoption(undef,'uniquelist', 'false');
Biber::Config->setblxoption(undef,'singletitle', 1);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_extradatedata_for_key('ey1'), '1', 'Extrayear - 1');
eq_or_diff($main->get_extradatedata_for_key('ey2'), '2', 'Extrayear - 2');
eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 3');
eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 4');
eq_or_diff($main->get_extradatedata_for_key('ey5'), '1', 'Extrayear - 5');
eq_or_diff($main->get_extradatedata_for_key('ey6'), '2', 'Extrayear - 6');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness3.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'full');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'singletitle', 1);
Biber::Config->setblxoption(undef,'uniquetitle', 1);
Biber::Config->setblxoption(undef,'uniquebaretitle', 1);
Biber::Config->setblxoption(undef,'uniquework', 1);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

ok(is_undef($main->get_extradatedata_for_key('ey1')), 'Extrayear - 7');
ok(is_undef($main->get_extradatedata_for_key('ey2')), 'Extrayear - 8');
eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 9');
eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 10');
ok(is_undef($main->get_extradatedata_for_key('ey5')), 'Extrayear - 11');
ok(is_undef($main->get_extradatedata_for_key('ey6')), 'Extrayear - 12');

ok(is_undef($main->get_entryfield('ey1', 'singletitle')), 'singletitle - 1');
eq_or_diff($main->get_entryfield('ey2', 'singletitle'), '1', 'singletitle - 2');
ok(is_undef($main->get_entryfield('ey3', 'singletitle')), 'singletitle - 3');
ok(is_undef($main->get_entryfield('ey4', 'singletitle')), 'singletitle - 4');
eq_or_diff($main->get_entryfield('ey5', 'singletitle'), '1', 'singletitle - 5');
eq_or_diff($main->get_entryfield('ey6', 'singletitle'), '1', 'singletitle - 6');

ok(is_undef($main->get_entryfield('ey1', 'uniquetitle')), 'uniquetitle - 1');
eq_or_diff($main->get_entryfield('ey2', 'uniquetitle'), '1', 'uniquetitle - 2');
ok(is_undef($main->get_entryfield('ey3', 'uniquetitle')), 'uniquetitle - 3');
eq_or_diff($main->get_entryfield('ey4', 'uniquetitle'), '1', 'uniquetitle - 4');
ok(is_undef($main->get_entryfield('ey5', 'uniquetitle')), 'uniquetitle - 5');
eq_or_diff($main->get_entryfield('ey6', 'uniquetitle'), '1', 'uniquetitle - 6');

ok(is_undef($main->get_entryfield('ey7', 'uniquebaretitle')), 'uniquebaretitle - 1');
ok(is_undef($main->get_entryfield('ey8', 'uniquebaretitle')), 'uniquebaretitle - 2');
eq_or_diff($main->get_entryfield('ey9', 'uniquebaretitle'), '1', 'uniquebaretitle - 3');

ok(is_undef($main->get_entryfield('ey1', 'uniquework')), 'uniquework - 1');
eq_or_diff($main->get_entryfield('ey2', 'uniquework'), '1', 'uniquework - 2');
eq_or_diff($main->get_entryfield('ey3', 'uniquework'), '1', 'uniquework - 3');
eq_or_diff($main->get_entryfield('ey4', 'uniquework'), '1', 'uniquework - 4');
eq_or_diff($main->get_entryfield('ey5', 'uniquework'), '1', 'uniquework - 5');
eq_or_diff($main->get_entryfield('ey6', 'uniquework'), '1', 'uniquework - 6');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness3.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'false');
Biber::Config->setblxoption(undef,'uniquelist', 'false');
Biber::Config->setblxoption(undef,'singletitle', 1);
Biber::Config->setblxoption(undef,'uniquetitle', 0);
Biber::Config->setblxoption(undef,'uniquework', 0);
Biber::Config->setblxoption(undef,'labeldatespec', [ {content => 'date', type => 'field'}, {content => 'year', type => 'field'} ]);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_extradatedata_for_key('ey1'), '1', 'Extrayear - 13');
eq_or_diff($main->get_extradatedata_for_key('ey2'), '2', 'Extrayear - 14');
eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 15');
eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 16');
eq_or_diff($main->get_extradatedata_for_key('ey5'), '1', 'Extrayear - 17');
eq_or_diff($main->get_extradatedata_for_key('ey6'), '2', 'Extrayear - 18');

#############################################################################

# Testing uniquename = allinit
$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness2.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'allinit');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Forced init expansion - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced init expansion - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced init expansion - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Forced init expansion - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced init expansion - 5');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced init expansion - 6');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '1', 'Forced init expansion - 7');
eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Forced init expansion - 8');

#############################################################################

# Testing uniquename = allfull
$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness2.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'allfull');
Biber::Config->setblxoption(undef,'uniquelist', 'true');
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '2', 'Forced name expansion - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced name expansion - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced name expansion - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '2', 'Forced name expansion - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced name expansion - 5');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced name expansion - 6');
eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '1', 'Forced name expansion - 7');
eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Forced name expansion - 8');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness6.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquelist', 'true');
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_uniquelist($bibentries->entry('entry1a')->get_field($bibentries->entry('entry1a')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 1');
eq_or_diff($main->get_uniquelist($bibentries->entry('entry1b')->get_field($bibentries->entry('entry1b')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 2');
eq_or_diff($main->get_uniquelist($bibentries->entry('entry2a')->get_field($bibentries->entry('entry2a')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 3');
eq_or_diff($main->get_uniquelist($bibentries->entry('entry2b')->get_field($bibentries->entry('entry2b')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 4');
eq_or_diff($main->get_uniquelist($bibentries->entry('A')->get_field($bibentries->entry('A')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 5');
eq_or_diff($main->get_uniquelist($bibentries->entry('B')->get_field($bibentries->entry('B')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 6');
eq_or_diff($main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 7');

#############################################################################

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness6.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');
# Biblatex options
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'uniquename', 'false');
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id), '2', 'Uniquelist true/Uniquename false - 1');

#############################################################################
# Testing pluralothers without uniquelist

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness7.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biblatex options
Biber::Config->setblxoption(undef,'uniquelist', 'false');
Biber::Config->setblxoption(undef,'pluralothers', 'true');
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 3);
# Now generate the information
$biber->prepare;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_visible_cite($bibentries->entry('po1')->get_field($bibentries->entry('po1')->get_labelname_info)->get_id), '4', 'Pluralothers test - 1');
ok(is_undef($main->get_extranamedata_for_key('po1')), 'Pluralothers test - 2');

#############################################################################
# Testing pluralothers with uniquelist

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness7.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biblatex options
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'uniquename', 'init');
Biber::Config->setblxoption(undef,'pluralothers', 'true');
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);
# Now generate the information
$biber->prepare;
my $out = $biber->get_output_obj;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');


my $po3 = q|    \entry{po3}{book}{}{}
      \name{author}{4}{ul=4}{%
        {{un=1,uniquepart=given,hash=c2ab7e2b5663336cc4e65c8bcf1a280d}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={A.},
           giveni={A\bibinitperiod},
           givenun=1}}%
        {{un=0,uniquepart=base,hash=1f4cf713d86f6083087eb3085db7815a}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={B.},
           giveni={B\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=a44def9031aa70c9f458f5b47a34c451}{%
           family={Cuthbert},
           familyi={C\bibinitperiod},
           given={C.},
           giveni={C\bibinitperiod},
           givenun=0}}%
        {{un=1,uniquepart=given,hash=91876a448dc35952ca94dc92cee07f89}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={D.},
           giveni={D\bibinitperiod},
           givenun=1}}%
      }
      \strng{namehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{bibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorbibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authornamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \field{labelalpha}{Abr\textbf{+}22}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title One}
      \field{year}{2022}
      \field{dateera}{ce}
    \endentry
|;

eq_or_diff($main->get_visible_cite($bibentries->entry('po3')->get_field($bibentries->entry('po3')->get_labelname_info)->get_id), '4', 'Pluralothers test - 3');
ok(is_undef($main->get_extranamedata_for_key('po3')), 'Pluralothers test - 4');
eq_or_diff( $out->get_output_entry('po3', $main), $po3, 'Pluralothers test - 5');

#############################################################################
# Testing uniquename minyearinit and minyearfull 

$biber = Biber->new(noconf => 1);
$biber->parse_ctrlfile('uniqueness7.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());
# Biblatex options
Biber::Config->setblxoption(undef,'uniquelist', 'true');
Biber::Config->setblxoption(undef,'uniquename', 'minyearinit');
Biber::Config->setblxoption(undef,'pluralothers', 'false');
Biber::Config->setblxoption(undef,'maxcitenames', 3);
Biber::Config->setblxoption(undef,'mincitenames', 1);
# Now generate the information
$biber->prepare;
$out = $biber->get_output_obj;
$section = $biber->sections->get_section(0);
$bibentries = $section->bibentries;
$main = $biber->datalists->get_list('nty/global//global/global/global');

eq_or_diff($main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id, $bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 1');
eq_or_diff($main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id, $bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 2');
eq_or_diff($main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id, $bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 3');
eq_or_diff($main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id, $bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename minyearinit - 4');
eq_or_diff($main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id, $bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename minyearinit - 5');
"####;

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_001_uniquename_requiring_full_name_expansion_1() {
    pass_upstream(
        "Uniquename requiring full name expansion - 1",
        r####"$main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id,$bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id,$bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_002_uniquename_requiring_full_name_expansion_2() {
    pass_upstream(
        "Uniquename requiring full name expansion - 2",
        r####"$main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id,$bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id,$bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_003_uniquename_requiring_full_name_expansion_3() {
    pass_upstream(
        "Uniquename requiring full name expansion - 3",
        r####"$main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id,$bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id,$bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename requiring full name expansion - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_004_uniquename_requiring_initials_name_expansion_per_namelist_uniquename_1() {
    pass_upstream(
        "Uniquename requiring initials name expansion (per-namelist uniquename) - 1",
        r####"is_undef($main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id,$bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id,$bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id)), 'Uniquename requiring initials name expansion (per-namelist uniquename) - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_005_uniquename_requiring_initials_name_expansion_2() {
    pass_upstream(
        "Uniquename requiring initials name expansion - 2",
        r####"$main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id,$bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id,$bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename requiring initials name expansion - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_006_per_entry_uniquename() {
    pass_upstream(
        "per-entry uniquename",
        r####"is_undef($main->get_unsummary($bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->get_id,$bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->nth_name(1)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_unsummary($bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->get_id,$bibentries->entry('un4a')->get_field($bibentries->entry('un4a')->get_labelname_info)->nth_name(1)->get_id)), 'per-entry uniquename');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_007_namehash_and_fullhash_1() {
    pass_upstream(
        "Namehash and fullhash - 1",
        r####"$main->get_entryfield('un6', 'namehash')"####,
        r####"'f8169a157f8d9209961157b8d23902db'"####,
        r####"eq_or_diff($main->get_entryfield('un6', 'namehash'), 'f8169a157f8d9209961157b8d23902db', 'Namehash and fullhash - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_008_namehash_and_fullhash_2() {
    pass_upstream(
        "Namehash and fullhash - 2",
        r####"$main->get_entryfield('un6', 'fullhash')"####,
        r####"'f8169a157f8d9209961157b8d23902db'"####,
        r####"eq_or_diff($main->get_entryfield('un6', 'fullhash'), 'f8169a157f8d9209961157b8d23902db', 'Namehash and fullhash - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_009_fullnamehash_ignores_short_names_1() {
    pass_upstream(
        "Fullnamehash ignores SHORT* names - 1",
        r####"$main->get_entryfield('un7', 'namehash')"####,
        r####"'b33fbd3f3349d1536dbcc14664f2cbbd'"####,
        r####"eq_or_diff($main->get_entryfield('un7', 'namehash'), 'b33fbd3f3349d1536dbcc14664f2cbbd', 'Fullnamehash ignores SHORT* names - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_010_fullnamehash_ignores_short_names_2() {
    pass_upstream(
        "Fullnamehash ignores SHORT* names - 2",
        r####"$main->get_entryfield('un7', 'fullhash')"####,
        r####"'f8169a157f8d9209961157b8d23902db'"####,
        r####"eq_or_diff($main->get_entryfield('un7', 'fullhash'), 'f8169a157f8d9209961157b8d23902db', 'Fullnamehash ignores SHORT* names - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_011_namehash_and_fullhash_3() {
    pass_upstream(
        "Namehash and fullhash - 3",
        r####"$main->get_entryfield('test1', 'namehash')"####,
        r####"'07df5c892ba1452776abee0a867591f2'"####,
        r####"eq_or_diff($main->get_entryfield('test1', 'namehash'), '07df5c892ba1452776abee0a867591f2', 'Namehash and fullhash - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_012_namehash_and_fullhash_4() {
    pass_upstream(
        "Namehash and fullhash - 4",
        r####"$main->get_entryfield('test1', 'fullhash')"####,
        r####"'637292dd2997a74c91847f1ec5081a46'"####,
        r####"eq_or_diff($main->get_entryfield('test1', 'fullhash'), '637292dd2997a74c91847f1ec5081a46', 'Namehash and fullhash - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_013_uniquename_with_full_and_repeat_1() {
    pass_upstream(
        "Uniquename with full and repeat - 1",
        r####"$main->get_unsummary($bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->get_id,$bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->get_id,$bibentries->entry('untf1')->get_field($bibentries->entry('untf1')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_014_uniquename_with_full_and_repeat_2() {
    pass_upstream(
        "Uniquename with full and repeat - 2",
        r####"$main->get_unsummary($bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->get_id,$bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->get_id,$bibentries->entry('untf2')->get_field($bibentries->entry('untf2')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_015_uniquename_with_full_and_repeat_3() {
    pass_upstream(
        "Uniquename with full and repeat - 3",
        r####"$main->get_unsummary($bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->get_id,$bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->get_id,$bibentries->entry('untf3')->get_field($bibentries->entry('untf3')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename with full and repeat - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_016_prefix_suffix_1() {
    pass_upstream(
        "Prefix/Suffix - 1",
        r####"$main->get_unsummary($bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->get_id,$bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->get_id,$bibentries->entry('sp1')->get_field($bibentries->entry('sp1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_017_prefix_suffix_2() {
    pass_upstream(
        "Prefix/Suffix - 2",
        r####"$main->get_unsummary($bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->get_id,$bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->get_id,$bibentries->entry('sp2')->get_field($bibentries->entry('sp2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_018_prefix_suffix_3() {
    pass_upstream(
        "Prefix/Suffix - 3",
        r####"$main->get_unsummary($bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->get_id,$bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->get_id,$bibentries->entry('sp3')->get_field($bibentries->entry('sp3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_019_prefix_suffix_4() {
    pass_upstream(
        "Prefix/Suffix - 4",
        r####"$main->get_unsummary($bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->get_id,$bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->get_id,$bibentries->entry('sp4')->get_field($bibentries->entry('sp4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_020_prefix_suffix_5() {
    pass_upstream(
        "Prefix/Suffix - 5",
        r####"$main->get_unsummary($bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->get_id,$bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->get_id,$bibentries->entry('sp5')->get_field($bibentries->entry('sp5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_021_prefix_suffix_6() {
    pass_upstream(
        "Prefix/Suffix - 6",
        r####"$main->get_unsummary($bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->get_id,$bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->get_id,$bibentries->entry('sp6')->get_field($bibentries->entry('sp6')->get_labelname_info)->nth_name(1)->get_id), '2', 'Prefix/Suffix - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_022_prefix_suffix_7() {
    pass_upstream(
        "Prefix/Suffix - 7",
        r####"$main->get_unsummary($bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->get_id,$bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->get_id,$bibentries->entry('sp7')->get_field($bibentries->entry('sp7')->get_labelname_info)->nth_name(1)->get_id), '2', 'Prefix/Suffix - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_023_prefix_suffix_8() {
    pass_upstream(
        "Prefix/Suffix - 8",
        r####"$main->get_unsummary($bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->get_id,$bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->get_id,$bibentries->entry('sp8')->get_field($bibentries->entry('sp8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_024_prefix_suffix_9() {
    pass_upstream(
        "Prefix/Suffix - 9",
        r####"$main->get_unsummary($bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->get_id,$bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->get_id,$bibentries->entry('sp9')->get_field($bibentries->entry('sp9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Prefix/Suffix - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_025_uniquename_with_inits_and_repeat_1() {
    pass_upstream(
        "Uniquename with inits and repeat - 1",
        r####"$main->get_unsummary($bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->get_id,$bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->get_id,$bibentries->entry('unt1')->get_field($bibentries->entry('unt1')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_026_uniquename_with_inits_and_repeat_2() {
    pass_upstream(
        "Uniquename with inits and repeat - 2",
        r####"$main->get_unsummary($bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->get_id,$bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->get_id,$bibentries->entry('unt2')->get_field($bibentries->entry('unt2')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_027_uniquename_with_inits_and_repeat_3() {
    pass_upstream(
        "Uniquename with inits and repeat - 3",
        r####"$main->get_unsummary($bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->get_id,$bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->get_id,$bibentries->entry('unt3')->get_field($bibentries->entry('unt3')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename with inits and repeat - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_028_uniquename_with_inits_and_repeat_4() {
    pass_upstream(
        "Uniquename with inits and repeat - 4",
        r####"$main->get_unsummary($bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->get_id,$bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->get_id,$bibentries->entry('unt4')->get_field($bibentries->entry('unt4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename with inits and repeat - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_029_uniquename_with_inits_and_repeat_5() {
    pass_upstream(
        "Uniquename with inits and repeat - 5",
        r####"$main->get_unsummary($bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->get_id,$bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->get_id,$bibentries->entry('unt5')->get_field($bibentries->entry('unt5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename with inits and repeat - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_030_namehash_and_fullhash_5() {
    pass_upstream(
        "Namehash and fullhash - 5",
        r####"$main->get_entryfield('unall3', 'namehash')"####,
        r####"'f1c5973adbc2e674fa4d98164c9ba5d5'"####,
        r####"eq_or_diff($main->get_entryfield('unall3', 'namehash'), 'f1c5973adbc2e674fa4d98164c9ba5d5', 'Namehash and fullhash - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_031_namehash_and_fullhash_6() {
    pass_upstream(
        "Namehash and fullhash - 6",
        r####"$main->get_entryfield('unall3', 'fullhash')"####,
        r####"'f1c5973adbc2e674fa4d98164c9ba5d5'"####,
        r####"eq_or_diff($main->get_entryfield('unall3', 'fullhash'), 'f1c5973adbc2e674fa4d98164c9ba5d5', 'Namehash and fullhash - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_032_uniquelist_edgecase_1() {
    pass_upstream(
        "Uniquelist edgecase - 1",
        r####"is_undef($main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id)), 'Uniquelist edgecase - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_033_uniquelist_edgecase_2() {
    pass_upstream(
        "Uniquelist edgecase - 2",
        r####"$main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id)"####,
        r####"'6'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id), '6', 'Uniquelist edgecase - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_034_uniquename_0_due_to_mincitenames_truncation() {
    pass_upstream(
        "Uniquename 0 due to mincitenames truncation",
        r####"is_undef($main->get_unsummary($bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->get_id,$bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->nth_name(1)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_unsummary($bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->get_id,$bibentries->entry('test2')->get_field($bibentries->entry('test2')->get_labelname_info)->nth_name(1)->get_id)), 'Uniquename 0 due to mincitenames truncation');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_035_uniquename_1() {
    pass_upstream(
        "Uniquename - 1",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_036_uniquename_2() {
    pass_upstream(
        "Uniquename - 2",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_037_uniquename_3() {
    pass_upstream(
        "Uniquename - 3",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Uniquename - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_038_uniquename_4() {
    pass_upstream(
        "Uniquename - 4",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_039_uniquename_5() {
    pass_upstream(
        "Uniquename - 5",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_040_uniquename_6() {
    pass_upstream(
        "Uniquename - 6",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Uniquename - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_041_uniquename_7() {
    pass_upstream(
        "Uniquename - 7",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '0', 'Uniquename - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_042_uniquename_8() {
    pass_upstream(
        "Uniquename - 8",
        r####"$main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_043_uniquelist_1() {
    pass_upstream(
        "Uniquelist - 1",
        r####"$main->get_uniquelist($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id), '3', 'Uniquelist - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_044_uniquelist_2() {
    pass_upstream(
        "Uniquelist - 2",
        r####"$main->get_uniquelist($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id), '3', 'Uniquelist - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_045_uniquelist_3() {
    pass_upstream(
        "Uniquelist - 3",
        r####"is_undef($main->get_uniquelist($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id)), 'Uniquelist - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_046_uniquelist_4() {
    pass_upstream(
        "Uniquelist - 4",
        r####"$main->get_uniquelist($bibentries->entry('unapa1')->get_field($bibentries->entry('unapa1')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unapa1')->get_field($bibentries->entry('unapa1')->get_labelname_info)->get_id), '3', 'Uniquelist - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_047_uniquelist_5() {
    pass_upstream(
        "Uniquelist - 5",
        r####"$main->get_uniquelist($bibentries->entry('unapa2')->get_field($bibentries->entry('unapa2')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unapa2')->get_field($bibentries->entry('unapa2')->get_labelname_info)->get_id), '3', 'Uniquelist - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_048_uniquelist_6() {
    pass_upstream(
        "Uniquelist - 6",
        r####"is_undef($main->get_uniquelist($bibentries->entry('others1')->get_field($bibentries->entry('others1')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('others1')->get_field($bibentries->entry('others1')->get_labelname_info)->get_id)), 'Uniquelist - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_049_uniquelist_7() {
    pass_upstream(
        "Uniquelist - 7",
        r####"is_undef($main->get_uniquelist($bibentries->entry('unall1')->get_field($bibentries->entry('unall1')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('unall1')->get_field($bibentries->entry('unall1')->get_labelname_info)->get_id)), 'Uniquelist - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_050_uniquelist_8() {
    pass_upstream(
        "Uniquelist - 8",
        r####"is_undef($main->get_uniquelist($bibentries->entry('unall2')->get_field($bibentries->entry('unall2')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('unall2')->get_field($bibentries->entry('unall2')->get_labelname_info)->get_id)), 'Uniquelist - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_051_uniquelist_9() {
    pass_upstream(
        "Uniquelist - 9",
        r####"$main->get_uniquelist($bibentries->entry('unall5')->get_field($bibentries->entry('unall5')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall5')->get_field($bibentries->entry('unall5')->get_labelname_info)->get_id), '5', 'Uniquelist - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_052_uniquelist_10() {
    pass_upstream(
        "Uniquelist - 10",
        r####"$main->get_uniquelist($bibentries->entry('unall6')->get_field($bibentries->entry('unall6')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall6')->get_field($bibentries->entry('unall6')->get_labelname_info)->get_id), '5', 'Uniquelist - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_053_uniquelist_11() {
    pass_upstream(
        "Uniquelist - 11",
        r####"$main->get_uniquelist($bibentries->entry('unall7')->get_field($bibentries->entry('unall7')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall7')->get_field($bibentries->entry('unall7')->get_labelname_info)->get_id), '5', 'Uniquelist - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_054_uniquelist_12() {
    pass_upstream(
        "Uniquelist - 12",
        r####"$main->get_uniquelist($bibentries->entry('unall8')->get_field($bibentries->entry('unall8')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall8')->get_field($bibentries->entry('unall8')->get_labelname_info)->get_id), '5', 'Uniquelist - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_055_uniquelist_13() {
    pass_upstream(
        "Uniquelist - 13",
        r####"$main->get_uniquelist($bibentries->entry('unall9')->get_field($bibentries->entry('unall9')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall9')->get_field($bibentries->entry('unall9')->get_labelname_info)->get_id), '5', 'Uniquelist - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_056_per_namelist_uniquelist_1() {
    pass_upstream(
        "Per-namelist Uniquelist - 1",
        r####"is_undef($main->get_uniquelist($bibentries->entry('unall9a')->get_field($bibentries->entry('unall9a')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('unall9a')->get_field($bibentries->entry('unall9a')->get_labelname_info)->get_id)), 'Per-namelist Uniquelist - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_057_uniquelist_14() {
    pass_upstream(
        "Uniquelist - 14",
        r####"$main->get_uniquelist($bibentries->entry('unall10')->get_field($bibentries->entry('unall10')->get_labelname_info)->get_id)"####,
        r####"'6'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall10')->get_field($bibentries->entry('unall10')->get_labelname_info)->get_id), '6', 'Uniquelist - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_058_uniquelist_15() {
    pass_upstream(
        "Uniquelist - 15",
        r####"$main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id)"####,
        r####"'5'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall3')->get_field($bibentries->entry('unall3')->get_labelname_info)->get_id), '5', 'Uniquelist - 15');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_059_uniquelist_16() {
    pass_upstream(
        "Uniquelist - 16",
        r####"$main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id)"####,
        r####"'6'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('unall4')->get_field($bibentries->entry('unall4')->get_labelname_info)->get_id), '6', 'Uniquelist - 16');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_060_uniquelist_17() {
    pass_upstream(
        "Uniquelist - 17",
        r####"$main->get_uniquelist($bibentries->entry('ul01')->get_field($bibentries->entry('ul01')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('ul01')->get_field($bibentries->entry('ul01')->get_labelname_info)->get_id), '3', 'Uniquelist - 17');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_061_uniquelist_18() {
    pass_upstream(
        "Uniquelist - 18",
        r####"$main->get_uniquelist($bibentries->entry('ul02')->get_field($bibentries->entry('ul02')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('ul02')->get_field($bibentries->entry('ul02')->get_labelname_info)->get_id), '3', 'Uniquelist - 18');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_062_uniquelist_19() {
    pass_upstream(
        "Uniquelist - 19",
        r####"$main->get_uniquelist($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id), '2', 'Uniquelist - 19');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_063_uniquename_9() {
    pass_upstream(
        "Uniquename - 9",
        r####"$main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_064_uniquename_10() {
    pass_upstream(
        "Uniquename - 10",
        r####"$main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->get_id,$bibentries->entry('test3')->get_field($bibentries->entry('test3')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_065_uniquelist_20() {
    pass_upstream(
        "Uniquelist - 20",
        r####"$main->get_uniquelist($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id), '2', 'Uniquelist - 20');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_066_uniquename_11() {
    pass_upstream(
        "Uniquename - 11",
        r####"$main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_067_uniquename_12() {
    pass_upstream(
        "Uniquename - 12",
        r####"$main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->get_id,$bibentries->entry('test4')->get_field($bibentries->entry('test4')->get_labelname_info)->nth_name(2)->get_id), '2', 'Uniquename - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_068_uniquelist_21() {
    pass_upstream(
        "Uniquelist - 21",
        r####"$main->get_uniquelist($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id), '2', 'Uniquelist - 21');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_069_uniquename_13() {
    pass_upstream(
        "Uniquename - 13",
        r####"$main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_070_uniquename_14() {
    pass_upstream(
        "Uniquename - 14",
        r####"$main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->get_id,$bibentries->entry('test5')->get_field($bibentries->entry('test5')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_071_uniquename_sparse_1() {
    pass_upstream(
        "Uniquename sparse - 1",
        r####"$main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_072_uniquename_sparse_2() {
    pass_upstream(
        "Uniquename sparse - 2",
        r####"$main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->get_id,$bibentries->entry('us1')->get_field($bibentries->entry('us1')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_073_uniquename_sparse_3() {
    pass_upstream(
        "Uniquename sparse - 3",
        r####"$main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_074_uniquename_sparse_4() {
    pass_upstream(
        "Uniquename sparse - 4",
        r####"$main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->get_id,$bibentries->entry('us2')->get_field($bibentries->entry('us2')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_075_uniquename_sparse_5() {
    pass_upstream(
        "Uniquename sparse - 5",
        r####"$main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_076_uniquename_sparse_6() {
    pass_upstream(
        "Uniquename sparse - 6",
        r####"$main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->get_id,$bibentries->entry('us3')->get_field($bibentries->entry('us3')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_077_uniquename_sparse_7() {
    pass_upstream(
        "Uniquename sparse - 7",
        r####"$main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_078_uniquename_sparse_8() {
    pass_upstream(
        "Uniquename sparse - 8",
        r####"$main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->get_id,$bibentries->entry('us4')->get_field($bibentries->entry('us4')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_079_uniquename_sparse_9() {
    pass_upstream(
        "Uniquename sparse - 9",
        r####"$main->get_unsummary($bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->get_id,$bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->get_id,$bibentries->entry('us5')->get_field($bibentries->entry('us5')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_080_uniquename_sparse_10() {
    pass_upstream(
        "Uniquename sparse - 10",
        r####"$main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_081_uniquename_sparse_11() {
    pass_upstream(
        "Uniquename sparse - 11",
        r####"$main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_082_uniquename_sparse_12() {
    pass_upstream(
        "Uniquename sparse - 12",
        r####"$main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->get_id,$bibentries->entry('us6')->get_field($bibentries->entry('us6')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_083_uniquename_sparse_13() {
    pass_upstream(
        "Uniquename sparse - 13",
        r####"$main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_084_uniquename_sparse_14() {
    pass_upstream(
        "Uniquename sparse - 14",
        r####"$main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->get_id,$bibentries->entry('us7')->get_field($bibentries->entry('us7')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_085_uniquename_sparse_15() {
    pass_upstream(
        "Uniquename sparse - 15",
        r####"$main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 15');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_086_uniquename_sparse_16() {
    pass_upstream(
        "Uniquename sparse - 16",
        r####"$main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->get_id,$bibentries->entry('us8')->get_field($bibentries->entry('us8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 16');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_087_uniquename_sparse_17() {
    pass_upstream(
        "Uniquename sparse - 17",
        r####"$main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 17');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_088_uniquename_sparse_18() {
    pass_upstream(
        "Uniquename sparse - 18",
        r####"$main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->get_id,$bibentries->entry('us9')->get_field($bibentries->entry('us9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 18');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_089_uniquename_sparse_19() {
    pass_upstream(
        "Uniquename sparse - 19",
        r####"$main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 19');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_090_uniquename_sparse_20() {
    pass_upstream(
        "Uniquename sparse - 20",
        r####"$main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->get_id,$bibentries->entry('us10')->get_field($bibentries->entry('us10')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename sparse - 20');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_091_uniquename_sparse_21() {
    pass_upstream(
        "Uniquename sparse - 21",
        r####"$main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 21');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_092_uniquename_sparse_22() {
    pass_upstream(
        "Uniquename sparse - 22",
        r####"$main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->get_id,$bibentries->entry('us11')->get_field($bibentries->entry('us11')->get_labelname_info)->nth_name(2)->get_id), '1', 'Uniquename sparse - 22');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_093_uniquename_sparse_23() {
    pass_upstream(
        "Uniquename sparse - 23",
        r####"$main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 23');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_094_uniquename_sparse_24() {
    pass_upstream(
        "Uniquename sparse - 24",
        r####"$main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->get_id,$bibentries->entry('us12')->get_field($bibentries->entry('us12')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 24');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_095_uniquename_sparse_25() {
    pass_upstream(
        "Uniquename sparse - 25",
        r####"$main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 25');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_096_uniquename_sparse_26() {
    pass_upstream(
        "Uniquename sparse - 26",
        r####"$main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->get_id,$bibentries->entry('us13')->get_field($bibentries->entry('us13')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 26');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_097_uniquename_sparse_27() {
    pass_upstream(
        "Uniquename sparse - 27",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 27');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_098_uniquename_sparse_28() {
    pass_upstream(
        "Uniquename sparse - 28",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 28');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_099_uniquename_sparse_29() {
    pass_upstream(
        "Uniquename sparse - 29",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 29');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_100_uniquename_sparse_30() {
    pass_upstream(
        "Uniquename sparse - 30",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 30');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_101_uniquename_sparse_31() {
    pass_upstream(
        "Uniquename sparse - 31",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 31');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_102_uniquename_sparse_32() {
    pass_upstream(
        "Uniquename sparse - 32",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 32');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_103_uniquename_sparse_33() {
    pass_upstream(
        "Uniquename sparse - 33",
        r####"$main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 33');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_104_uniquename_sparse_34() {
    pass_upstream(
        "Uniquename sparse - 34",
        r####"$main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 34');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_105_uniquename_sparse_35() {
    pass_upstream(
        "Uniquename sparse - 35",
        r####"$main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id,$bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 35');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_106_uniquename_sparse_36() {
    pass_upstream(
        "Uniquename sparse - 36",
        r####"is_undef($main->get_uniquelist($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('us16')->get_field($bibentries->entry('us16')->get_labelname_info)->get_id)), 'Uniquename sparse - 36');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_107_uniquename_sparse_37() {
    pass_upstream(
        "Uniquename sparse - 37",
        r####"$main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 37');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_108_uniquename_sparse_38() {
    pass_upstream(
        "Uniquename sparse - 38",
        r####"$main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 38');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_109_uniquename_sparse_39() {
    pass_upstream(
        "Uniquename sparse - 39",
        r####"$main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 39');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_110_uniquename_sparse_40() {
    pass_upstream(
        "Uniquename sparse - 40",
        r####"$main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(4)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id,$bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->nth_name(4)->get_id), '0', 'Uniquename sparse - 40');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_111_uniquename_sparse_41() {
    pass_upstream(
        "Uniquename sparse - 41",
        r####"$main->get_uniquelist($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id)"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('us17')->get_field($bibentries->entry('us17')->get_labelname_info)->get_id), '4', 'Uniquename sparse - 41');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_112_uniquename_sparse_42() {
    pass_upstream(
        "Uniquename sparse - 42",
        r####"$main->get_unsummary($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id,$bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id,$bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 42');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_113_uniquename_sparse_43() {
    pass_upstream(
        "Uniquename sparse - 43",
        r####"$main->get_unsummary($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id,$bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id,$bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 43');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_114_uniquename_sparse_44() {
    pass_upstream(
        "Uniquename sparse - 44",
        r####"is_undef($main->get_uniquelist($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('us18')->get_field($bibentries->entry('us18')->get_labelname_info)->get_id)), 'Uniquename sparse - 44');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_115_uniquename_sparse_45() {
    pass_upstream(
        "Uniquename sparse - 45",
        r####"$main->get_uniquelist($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id)"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('us19')->get_field($bibentries->entry('us19')->get_labelname_info)->get_id), '4', 'Uniquename sparse - 45');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_116_uniquename_sparse_46() {
    pass_upstream(
        "Uniquename sparse - 46",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 46');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_117_uniquename_sparse_47() {
    pass_upstream(
        "Uniquename sparse - 47",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 47');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_118_uniquename_sparse_48() {
    pass_upstream(
        "Uniquename sparse - 48",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 48');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_119_uniquename_sparse_49() {
    pass_upstream(
        "Uniquename sparse - 49",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 49');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_120_uniquename_sparse_50() {
    pass_upstream(
        "Uniquename sparse - 50",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 50');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_121_uniquename_sparse_51() {
    pass_upstream(
        "Uniquename sparse - 51",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 51');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_122_uniquename_sparse_52() {
    pass_upstream(
        "Uniquename sparse - 52",
        r####"$main->get_unsummary($bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->get_id,$bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->get_id,$bibentries->entry('us20')->get_field($bibentries->entry('us20')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 52');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_123_uniquename_sparse_53() {
    pass_upstream(
        "Uniquename sparse - 53",
        r####"$main->get_unsummary($bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->get_id,$bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->get_id,$bibentries->entry('us21')->get_field($bibentries->entry('us21')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename sparse - 53');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_124_uniquename_sparse_54() {
    pass_upstream(
        "Uniquename sparse - 54",
        r####"$main->get_unsummary($bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->get_id,$bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->get_id,$bibentries->entry('us22')->get_field($bibentries->entry('us22')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 54');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_125_uniquename_sparse_55() {
    pass_upstream(
        "Uniquename sparse - 55",
        r####"$main->get_unsummary($bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->get_id,$bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->get_id,$bibentries->entry('us23')->get_field($bibentries->entry('us23')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 55');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_126_uniquename_sparse_56() {
    pass_upstream(
        "Uniquename sparse - 56",
        r####"$main->get_unsummary($bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->get_id,$bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->get_id,$bibentries->entry('us24')->get_field($bibentries->entry('us24')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 56');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_127_uniquename_sparse_57() {
    pass_upstream(
        "Uniquename sparse - 57",
        r####"$main->get_unsummary($bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->get_id,$bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->get_id,$bibentries->entry('us25')->get_field($bibentries->entry('us25')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 57');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_128_uniquename_sparse_58() {
    pass_upstream(
        "Uniquename sparse - 58",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 58');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_129_uniquename_sparse_59() {
    pass_upstream(
        "Uniquename sparse - 59",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 59');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_130_uniquename_sparse_60() {
    pass_upstream(
        "Uniquename sparse - 60",
        r####"$main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->get_id,$bibentries->entry('us14')->get_field($bibentries->entry('us14')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 60');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_131_uniquename_sparse_61() {
    pass_upstream(
        "Uniquename sparse - 61",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 61');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_132_uniquename_sparse_62() {
    pass_upstream(
        "Uniquename sparse - 62",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(2)->get_id), '0', 'Uniquename sparse - 62');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_133_uniquename_sparse_63() {
    pass_upstream(
        "Uniquename sparse - 63",
        r####"$main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->get_id,$bibentries->entry('us15')->get_field($bibentries->entry('us15')->get_labelname_info)->nth_name(3)->get_id), '0', 'Uniquename sparse - 63');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_134_uniquename_sparse_64() {
    pass_upstream(
        "Uniquename sparse - 64",
        r####"$main->get_unsummary($bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->get_id,$bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->get_id,$bibentries->entry('us26')->get_field($bibentries->entry('us26')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 64');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_135_uniquename_sparse_65() {
    pass_upstream(
        "Uniquename sparse - 65",
        r####"$main->get_unsummary($bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->get_id,$bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->get_id,$bibentries->entry('us27')->get_field($bibentries->entry('us27')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 65');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_136_uniquename_sparse_66() {
    pass_upstream(
        "Uniquename sparse - 66",
        r####"$main->get_unsummary($bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->get_id,$bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->get_id,$bibentries->entry('us28')->get_field($bibentries->entry('us28')->get_labelname_info)->nth_name(1)->get_id), '2', 'Uniquename sparse - 66');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_137_uniquename_sparse_67() {
    pass_upstream(
        "Uniquename sparse - 67",
        r####"$main->get_unsummary($bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->get_id,$bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->get_id,$bibentries->entry('us29')->get_field($bibentries->entry('us29')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 67');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_138_uniquename_sparse_68() {
    pass_upstream(
        "Uniquename sparse - 68",
        r####"$main->get_unsummary($bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->get_id,$bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->get_id,$bibentries->entry('us30')->get_field($bibentries->entry('us30')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename sparse - 68');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_139_uniquelist_strict_1() {
    pass_upstream(
        "Uniquelist strict - 1",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id)), 'Uniquelist strict - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_140_uniquelist_strict_2() {
    pass_upstream(
        "Uniquelist strict - 2",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls2')->get_field($bibentries->entry('uls2')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls2')->get_field($bibentries->entry('uls2')->get_labelname_info)->get_id)), 'Uniquelist strict - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_141_uniquelist_strict_3() {
    pass_upstream(
        "Uniquelist strict - 3",
        r####"$main->get_uniquelist($bibentries->entry('uls3')->get_field($bibentries->entry('uls3')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls3')->get_field($bibentries->entry('uls3')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_142_uniquelist_strict_4() {
    pass_upstream(
        "Uniquelist strict - 4",
        r####"$main->get_uniquelist($bibentries->entry('uls4')->get_field($bibentries->entry('uls4')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls4')->get_field($bibentries->entry('uls4')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_143_uniquelist_strict_5() {
    pass_upstream(
        "Uniquelist strict - 5",
        r####"$main->get_uniquelist($bibentries->entry('uls5')->get_field($bibentries->entry('uls5')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls5')->get_field($bibentries->entry('uls5')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_144_uniquelist_strict_6() {
    pass_upstream(
        "Uniquelist strict - 6",
        r####"$main->get_uniquelist($bibentries->entry('uls6')->get_field($bibentries->entry('uls6')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls6')->get_field($bibentries->entry('uls6')->get_labelname_info)->get_id), '2', 'Uniquelist strict - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_145_uniquelist_strict_7() {
    pass_upstream(
        "Uniquelist strict - 7",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls7')->get_field($bibentries->entry('uls7')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls7')->get_field($bibentries->entry('uls7')->get_labelname_info)->get_id)), 'Uniquelist strict - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_146_uniquelist_minyear_1() {
    pass_upstream(
        "Uniquelist minyear - 1",
        r####"$main->get_uniquelist($bibentries->entry('ulmy1')->get_field($bibentries->entry('ulmy1')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('ulmy1')->get_field($bibentries->entry('ulmy1')->get_labelname_info)->get_id), '2', 'Uniquelist minyear - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_147_uniquelist_minyear_2() {
    pass_upstream(
        "Uniquelist minyear - 2",
        r####"$main->get_uniquelist($bibentries->entry('ulmy2')->get_field($bibentries->entry('ulmy2')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('ulmy2')->get_field($bibentries->entry('ulmy2')->get_labelname_info)->get_id), '2', 'Uniquelist minyear - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_148_uniquelist_minyear_3() {
    pass_upstream(
        "Uniquelist minyear - 3",
        r####"is_undef($main->get_uniquelist($bibentries->entry('ulmy3')->get_field($bibentries->entry('ulmy3')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('ulmy3')->get_field($bibentries->entry('ulmy3')->get_labelname_info)->get_id)), 'Uniquelist minyear - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_149_uniquelist_strict_8() {
    pass_upstream(
        "Uniquelist strict - 8",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls8')->get_field($bibentries->entry('uls8')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls8')->get_field($bibentries->entry('uls8')->get_labelname_info)->get_id)), 'Uniquelist strict - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_150_uniquelist_strict_9() {
    pass_upstream(
        "Uniquelist strict - 9",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls9')->get_field($bibentries->entry('uls9')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls9')->get_field($bibentries->entry('uls9')->get_labelname_info)->get_id)),'Uniquelist strict - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_151_uniquelist_strict_10() {
    pass_upstream(
        "Uniquelist strict - 10",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls1')->get_field($bibentries->entry('uls1')->get_labelname_info)->get_id)),'Uniquelist strict - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_152_uniquelist_strict_11() {
    pass_upstream(
        "Uniquelist strict - 11",
        r####"$main->get_uniquelist($bibentries->entry('uls10')->get_field($bibentries->entry('uls10')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls10')->get_field($bibentries->entry('uls10')->get_labelname_info)->get_id), '3', 'Uniquelist strict - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_153_uniquelist_strict_12() {
    pass_upstream(
        "Uniquelist strict - 12",
        r####"$main->get_uniquelist($bibentries->entry('uls11')->get_field($bibentries->entry('uls11')->get_labelname_info)->get_id)"####,
        r####"'3'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('uls11')->get_field($bibentries->entry('uls11')->get_labelname_info)->get_id), '3', 'Uniquelist strict - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_154_uniquelist_strict_13() {
    pass_upstream(
        "Uniquelist strict - 13",
        r####"is_undef($main->get_uniquelist($bibentries->entry('uls12')->get_field($bibentries->entry('uls12')->get_labelname_info)->get_id))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_uniquelist($bibentries->entry('uls12')->get_field($bibentries->entry('uls12')->get_labelname_info)->get_id)), 'Uniquelist strict - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_155_extrayear_1() {
    pass_upstream(
        "Extrayear - 1",
        r####"$main->get_extradatedata_for_key('ey1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey1'), '1', 'Extrayear - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_156_extrayear_2() {
    pass_upstream(
        "Extrayear - 2",
        r####"$main->get_extradatedata_for_key('ey2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey2'), '2', 'Extrayear - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_157_extrayear_3() {
    pass_upstream(
        "Extrayear - 3",
        r####"$main->get_extradatedata_for_key('ey3')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_158_extrayear_4() {
    pass_upstream(
        "Extrayear - 4",
        r####"$main->get_extradatedata_for_key('ey4')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_159_extrayear_5() {
    pass_upstream(
        "Extrayear - 5",
        r####"$main->get_extradatedata_for_key('ey5')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey5'), '1', 'Extrayear - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_160_extrayear_6() {
    pass_upstream(
        "Extrayear - 6",
        r####"$main->get_extradatedata_for_key('ey6')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey6'), '2', 'Extrayear - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_161_extrayear_7() {
    pass_upstream(
        "Extrayear - 7",
        r####"is_undef($main->get_extradatedata_for_key('ey1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ey1')), 'Extrayear - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_162_extrayear_8() {
    pass_upstream(
        "Extrayear - 8",
        r####"is_undef($main->get_extradatedata_for_key('ey2'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ey2')), 'Extrayear - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_163_extrayear_9() {
    pass_upstream(
        "Extrayear - 9",
        r####"$main->get_extradatedata_for_key('ey3')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_164_extrayear_10() {
    pass_upstream(
        "Extrayear - 10",
        r####"$main->get_extradatedata_for_key('ey4')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_165_extrayear_11() {
    pass_upstream(
        "Extrayear - 11",
        r####"is_undef($main->get_extradatedata_for_key('ey5'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ey5')), 'Extrayear - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_166_extrayear_12() {
    pass_upstream(
        "Extrayear - 12",
        r####"is_undef($main->get_extradatedata_for_key('ey6'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extradatedata_for_key('ey6')), 'Extrayear - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_167_singletitle_1() {
    pass_upstream(
        "singletitle - 1",
        r####"is_undef($main->get_entryfield('ey1', 'singletitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey1', 'singletitle')), 'singletitle - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_168_singletitle_2() {
    pass_upstream(
        "singletitle - 2",
        r####"$main->get_entryfield('ey2', 'singletitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey2', 'singletitle'), '1', 'singletitle - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_169_singletitle_3() {
    pass_upstream(
        "singletitle - 3",
        r####"is_undef($main->get_entryfield('ey3', 'singletitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey3', 'singletitle')), 'singletitle - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_170_singletitle_4() {
    pass_upstream(
        "singletitle - 4",
        r####"is_undef($main->get_entryfield('ey4', 'singletitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey4', 'singletitle')), 'singletitle - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_171_singletitle_5() {
    pass_upstream(
        "singletitle - 5",
        r####"$main->get_entryfield('ey5', 'singletitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey5', 'singletitle'), '1', 'singletitle - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_172_singletitle_6() {
    pass_upstream(
        "singletitle - 6",
        r####"$main->get_entryfield('ey6', 'singletitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey6', 'singletitle'), '1', 'singletitle - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_173_uniquetitle_1() {
    pass_upstream(
        "uniquetitle - 1",
        r####"is_undef($main->get_entryfield('ey1', 'uniquetitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey1', 'uniquetitle')), 'uniquetitle - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_174_uniquetitle_2() {
    pass_upstream(
        "uniquetitle - 2",
        r####"$main->get_entryfield('ey2', 'uniquetitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey2', 'uniquetitle'), '1', 'uniquetitle - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_175_uniquetitle_3() {
    pass_upstream(
        "uniquetitle - 3",
        r####"is_undef($main->get_entryfield('ey3', 'uniquetitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey3', 'uniquetitle')), 'uniquetitle - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_176_uniquetitle_4() {
    pass_upstream(
        "uniquetitle - 4",
        r####"$main->get_entryfield('ey4', 'uniquetitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey4', 'uniquetitle'), '1', 'uniquetitle - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_177_uniquetitle_5() {
    pass_upstream(
        "uniquetitle - 5",
        r####"is_undef($main->get_entryfield('ey5', 'uniquetitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey5', 'uniquetitle')), 'uniquetitle - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_178_uniquetitle_6() {
    pass_upstream(
        "uniquetitle - 6",
        r####"$main->get_entryfield('ey6', 'uniquetitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey6', 'uniquetitle'), '1', 'uniquetitle - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_179_uniquebaretitle_1() {
    pass_upstream(
        "uniquebaretitle - 1",
        r####"is_undef($main->get_entryfield('ey7', 'uniquebaretitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey7', 'uniquebaretitle')), 'uniquebaretitle - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_180_uniquebaretitle_2() {
    pass_upstream(
        "uniquebaretitle - 2",
        r####"is_undef($main->get_entryfield('ey8', 'uniquebaretitle'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey8', 'uniquebaretitle')), 'uniquebaretitle - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_181_uniquebaretitle_3() {
    pass_upstream(
        "uniquebaretitle - 3",
        r####"$main->get_entryfield('ey9', 'uniquebaretitle')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey9', 'uniquebaretitle'), '1', 'uniquebaretitle - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_182_uniquework_1() {
    pass_upstream(
        "uniquework - 1",
        r####"is_undef($main->get_entryfield('ey1', 'uniquework'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_entryfield('ey1', 'uniquework')), 'uniquework - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_183_uniquework_2() {
    pass_upstream(
        "uniquework - 2",
        r####"$main->get_entryfield('ey2', 'uniquework')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey2', 'uniquework'), '1', 'uniquework - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_184_uniquework_3() {
    pass_upstream(
        "uniquework - 3",
        r####"$main->get_entryfield('ey3', 'uniquework')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey3', 'uniquework'), '1', 'uniquework - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_185_uniquework_4() {
    pass_upstream(
        "uniquework - 4",
        r####"$main->get_entryfield('ey4', 'uniquework')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey4', 'uniquework'), '1', 'uniquework - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_186_uniquework_5() {
    pass_upstream(
        "uniquework - 5",
        r####"$main->get_entryfield('ey5', 'uniquework')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey5', 'uniquework'), '1', 'uniquework - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_187_uniquework_6() {
    pass_upstream(
        "uniquework - 6",
        r####"$main->get_entryfield('ey6', 'uniquework')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_entryfield('ey6', 'uniquework'), '1', 'uniquework - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_188_extrayear_13() {
    pass_upstream(
        "Extrayear - 13",
        r####"$main->get_extradatedata_for_key('ey1')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey1'), '1', 'Extrayear - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_189_extrayear_14() {
    pass_upstream(
        "Extrayear - 14",
        r####"$main->get_extradatedata_for_key('ey2')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey2'), '2', 'Extrayear - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_190_extrayear_15() {
    pass_upstream(
        "Extrayear - 15",
        r####"$main->get_extradatedata_for_key('ey3')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey3'), '1', 'Extrayear - 15');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_191_extrayear_16() {
    pass_upstream(
        "Extrayear - 16",
        r####"$main->get_extradatedata_for_key('ey4')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey4'), '2', 'Extrayear - 16');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_192_extrayear_17() {
    pass_upstream(
        "Extrayear - 17",
        r####"$main->get_extradatedata_for_key('ey5')"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey5'), '1', 'Extrayear - 17');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_193_extrayear_18() {
    pass_upstream(
        "Extrayear - 18",
        r####"$main->get_extradatedata_for_key('ey6')"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_extradatedata_for_key('ey6'), '2', 'Extrayear - 18');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_194_forced_init_expansion_1() {
    pass_upstream(
        "Forced init expansion - 1",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '0', 'Forced init expansion - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_195_forced_init_expansion_2() {
    pass_upstream(
        "Forced init expansion - 2",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced init expansion - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_196_forced_init_expansion_3() {
    pass_upstream(
        "Forced init expansion - 3",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced init expansion - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_197_forced_init_expansion_4() {
    pass_upstream(
        "Forced init expansion - 4",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '0', 'Forced init expansion - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_198_forced_init_expansion_5() {
    pass_upstream(
        "Forced init expansion - 5",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced init expansion - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_199_forced_init_expansion_6() {
    pass_upstream(
        "Forced init expansion - 6",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced init expansion - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_200_forced_init_expansion_7() {
    pass_upstream(
        "Forced init expansion - 7",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '1', 'Forced init expansion - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_201_forced_init_expansion_8() {
    pass_upstream(
        "Forced init expansion - 8",
        r####"$main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Forced init expansion - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_202_forced_name_expansion_1() {
    pass_upstream(
        "Forced name expansion - 1",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(1)->get_id), '2', 'Forced name expansion - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_203_forced_name_expansion_2() {
    pass_upstream(
        "Forced name expansion - 2",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced name expansion - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_204_forced_name_expansion_3() {
    pass_upstream(
        "Forced name expansion - 3",
        r####"$main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->get_id,$bibentries->entry('un8')->get_field($bibentries->entry('un8')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced name expansion - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_205_forced_name_expansion_4() {
    pass_upstream(
        "Forced name expansion - 4",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(1)->get_id), '2', 'Forced name expansion - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_206_forced_name_expansion_5() {
    pass_upstream(
        "Forced name expansion - 5",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(2)->get_id), '0', 'Forced name expansion - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_207_forced_name_expansion_6() {
    pass_upstream(
        "Forced name expansion - 6",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(3)->get_id), '1', 'Forced name expansion - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_208_forced_name_expansion_7() {
    pass_upstream(
        "Forced name expansion - 7",
        r####"$main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->get_id,$bibentries->entry('un9')->get_field($bibentries->entry('un9')->get_labelname_info)->nth_name(4)->get_id), '1', 'Forced name expansion - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_209_forced_name_expansion_8() {
    pass_upstream(
        "Forced name expansion - 8",
        r####"$main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->get_id,$bibentries->entry('un10')->get_field($bibentries->entry('un10')->get_labelname_info)->nth_name(1)->get_id), '1', 'Forced name expansion - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_210_uniquelist_duplicates_1() {
    pass_upstream(
        "Uniquelist duplicates - 1",
        r####"$main->get_uniquelist($bibentries->entry('entry1a')->get_field($bibentries->entry('entry1a')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('entry1a')->get_field($bibentries->entry('entry1a')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_211_uniquelist_duplicates_2() {
    pass_upstream(
        "Uniquelist duplicates - 2",
        r####"$main->get_uniquelist($bibentries->entry('entry1b')->get_field($bibentries->entry('entry1b')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('entry1b')->get_field($bibentries->entry('entry1b')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_212_uniquelist_duplicates_3() {
    pass_upstream(
        "Uniquelist duplicates - 3",
        r####"$main->get_uniquelist($bibentries->entry('entry2a')->get_field($bibentries->entry('entry2a')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('entry2a')->get_field($bibentries->entry('entry2a')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_213_uniquelist_duplicates_4() {
    pass_upstream(
        "Uniquelist duplicates - 4",
        r####"$main->get_uniquelist($bibentries->entry('entry2b')->get_field($bibentries->entry('entry2b')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('entry2b')->get_field($bibentries->entry('entry2b')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_214_uniquelist_duplicates_5() {
    pass_upstream(
        "Uniquelist duplicates - 5",
        r####"$main->get_uniquelist($bibentries->entry('A')->get_field($bibentries->entry('A')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('A')->get_field($bibentries->entry('A')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_215_uniquelist_duplicates_6() {
    pass_upstream(
        "Uniquelist duplicates - 6",
        r####"$main->get_uniquelist($bibentries->entry('B')->get_field($bibentries->entry('B')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('B')->get_field($bibentries->entry('B')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_216_uniquelist_duplicates_7() {
    pass_upstream(
        "Uniquelist duplicates - 7",
        r####"$main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id), '2', 'Uniquelist duplicates - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_217_uniquelist_true_uniquename_false_1() {
    pass_upstream(
        "Uniquelist true/Uniquename false - 1",
        r####"$main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id)"####,
        r####"'2'"####,
        r####"eq_or_diff($main->get_uniquelist($bibentries->entry('C')->get_field($bibentries->entry('C')->get_labelname_info)->get_id), '2', 'Uniquelist true/Uniquename false - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_218_pluralothers_test_1() {
    pass_upstream(
        "Pluralothers test - 1",
        r####"$main->get_visible_cite($bibentries->entry('po1')->get_field($bibentries->entry('po1')->get_labelname_info)->get_id)"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_visible_cite($bibentries->entry('po1')->get_field($bibentries->entry('po1')->get_labelname_info)->get_id), '4', 'Pluralothers test - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_219_pluralothers_test_2() {
    pass_upstream(
        "Pluralothers test - 2",
        r####"is_undef($main->get_extranamedata_for_key('po1'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extranamedata_for_key('po1')), 'Pluralothers test - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_220_pluralothers_test_3() {
    pass_upstream(
        "Pluralothers test - 3",
        r####"$main->get_visible_cite($bibentries->entry('po3')->get_field($bibentries->entry('po3')->get_labelname_info)->get_id)"####,
        r####"'4'"####,
        r####"eq_or_diff($main->get_visible_cite($bibentries->entry('po3')->get_field($bibentries->entry('po3')->get_labelname_info)->get_id), '4', 'Pluralothers test - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_221_pluralothers_test_4() {
    pass_upstream(
        "Pluralothers test - 4",
        r####"is_undef($main->get_extranamedata_for_key('po3'))"####,
        r####"true"####,
        r####"ok(is_undef($main->get_extranamedata_for_key('po3')), 'Pluralothers test - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_222_pluralothers_test_5() {
    pass_upstream(
        "Pluralothers test - 5",
        r####"$out->get_output_entry('po3', $main)"####,
        r####"$po3"####,
        r####"eq_or_diff( $out->get_output_entry('po3', $main), $po3, 'Pluralothers test - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_223_uniquename_minyearinit_1() {
    pass_upstream(
        "Uniquename minyearinit - 1",
        r####"$main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id, $bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id, $bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_224_uniquename_minyearinit_2() {
    pass_upstream(
        "Uniquename minyearinit - 2",
        r####"$main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id, $bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id, $bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_225_uniquename_minyearinit_3() {
    pass_upstream(
        "Uniquename minyearinit - 3",
        r####"$main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id, $bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'0'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id, $bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id), '0', 'Uniquename minyearinit - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_226_uniquename_minyearinit_4() {
    pass_upstream(
        "Uniquename minyearinit - 4",
        r####"$main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id, $bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id, $bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename minyearinit - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name/list uniqueness query API"]
fn assertion_227_uniquename_minyearinit_5() {
    pass_upstream(
        "Uniquename minyearinit - 5",
        r####"$main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id, $bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id)"####,
        r####"'1'"####,
        r####"eq_or_diff($main->get_unsummary($bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id, $bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id), '1', 'Uniquename minyearinit - 5');"####,
        UPSTREAM_SOURCE,
    );
}
