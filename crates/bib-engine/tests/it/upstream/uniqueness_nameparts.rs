// Direct passing translation of upstream t/uniqueness-nameparts.t at commit 74252e6.
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
    panic!("xfail: bib-engine has no public name-part uniqueness or BBL query API");
}

const UPSTREAM_SOURCE: &str = r####"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 36;
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

$biber->parse_ctrlfile('uniqueness-nameparts.bcf');
$biber->set_output_obj(Biber::Output::bbl->new());

# Options - we could set these in the control file but it's nice to see what we're
# relying on here for tests

# Biber options
Biber::Config->setoption('sortlocale', 'en_GB.UTF-8');

# Biblatex options
Biber::Config->setblxoption(undef,'uniquename', 'full');

# Now generate the information
$biber->prepare;
my $section = $biber->sections->get_section(0);
my $bibentries = $section->bibentries;

my $un1nlid = $bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->get_id;
my $un1nid = $bibentries->entry('un1')->get_field($bibentries->entry('un1')->get_labelname_info)->nth_name(1)->get_id;
my $un2nlid = $bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->get_id;
my $un2nid = $bibentries->entry('un2')->get_field($bibentries->entry('un2')->get_labelname_info)->nth_name(1)->get_id;
my $un3nlid = $bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->get_id;
my $un3nid = $bibentries->entry('un3')->get_field($bibentries->entry('un3')->get_labelname_info)->nth_name(1)->get_id;
my $un4nlid = $bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->get_id;
my $un4nid = $bibentries->entry('un4')->get_field($bibentries->entry('un4')->get_labelname_info)->nth_name(1)->get_id;
my $un5nlid = $bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->get_id;
my $un5nid = $bibentries->entry('un5')->get_field($bibentries->entry('un5')->get_labelname_info)->nth_name(1)->get_id;
my $un6nlid = $bibentries->entry('un6')->get_field($bibentries->entry('un6')->get_labelname_info)->get_id;
my $un6nid = $bibentries->entry('un6')->get_field($bibentries->entry('un6')->get_labelname_info)->nth_name(1)->get_id;
my $un7nlid = $bibentries->entry('un7')->get_field($bibentries->entry('un7')->get_labelname_info)->get_id;
my $un7nid = $bibentries->entry('un7')->get_field($bibentries->entry('un7')->get_labelname_info)->nth_name(1)->get_id;

my $out = $biber->get_output_obj;
my $un1 = q|    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=middle,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=1}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un2 = q|    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un3 = q|    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un4 = q|    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=given,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=1,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un1a = q|    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=middle,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=1}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un2a = q|    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un3a = q|    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;


my $un4a = q|    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=given,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=2,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

# because given is ignored and middle does not disambiguate even on full
# extradate is calculated on "Smith" only
my $un1b = q|    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{extraname}{5}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{5}
      \field{labelnamesource}{author}
    \endentry
|;

my $un2b = q|    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

my $un3b = q|    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
|;

# because given is ignored and middle does not disambiguate even on full
# extradate is calculated on "Smith" only
my $un4b = q|    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{extraname}{1}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{1}
      \field{labelnamesource}{author}
    \endentry
|;

# because given is ignored and middle does not disambiguate on full
# extradate is calculated on "Smith" only
my $un5 = q|    \entry{un5}{article}{uniquenametemplatename=test3}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=74fba0d07ca65976bbff1034f9bb22e6}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{fullhash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{fullhashraw}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{bibnamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorbibnamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authornamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorfullhash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorfullhashraw}{74fba0d07ca65976bbff1034f9bb22e6}
      \field{extraname}{2}
      \field{labelalpha}{SmiArtSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{2}
      \field{labelnamesource}{author}
    \endentry
|;

# because given is ignored and middle does not disambiguate on init
# extradate is calculated on "Smith" only
my $un6 = q|    \entry{un6}{article}{}{}
      \name{author}{1}{uniquenametemplatename=test4}{%
        {{un=0,uniquepart=base,hash=8100e7d06d05938e91bf8863f5c20e33}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Smythe},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{fullhash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{fullhashraw}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{bibnamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorbibnamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authornamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorfullhash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorfullhashraw}{8100e7d06d05938e91bf8863f5c20e33}
      \field{extraname}{3}
      \field{labelalpha}{SmiArtSmy}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{3}
      \field{labelnamesource}{author}
    \\endentry
|;

# because there is nothing to disambiguate the base at all
# extradate is calculated on "Smith" only
my $un7 = q|    \entry{un7}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,uniquenametemplatename=test5,hash=c21736158273b6f2f368818459734e04}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Smedley},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{c21736158273b6f2f368818459734e04}
      \strng{fullhash}{c21736158273b6f2f368818459734e04}
      \strng{fullhashraw}{c21736158273b6f2f368818459734e04}
      \strng{bibnamehash}{c21736158273b6f2f368818459734e04}
      \strng{authorbibnamehash}{c21736158273b6f2f368818459734e04}
      \strng{authornamehash}{c21736158273b6f2f368818459734e04}
      \strng{authorfullhash}{c21736158273b6f2f368818459734e04}
      \strng{authorfullhashraw}{c21736158273b6f2f368818459734e04}
      \field{extraname}{4}
      \field{labelalpha}{SmiArtSme}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{4}
      \field{labelnamesource}{author}
    \endentry
|;

my $main = $biber->datalists->get_list('nty/global//global/global/global');
my $main1 = $biber->datalists->get_list('nty/global//test1/global');
my $main2 = $biber->datalists->get_list('nty/global//test2/global');

eq_or_diff($out->get_output_entry('un1', $main), $un1, 'Uniquename namepart - 1');
eq_or_diff($out->get_output_entry('un2', $main), $un2, 'Uniquename namepart - 2');
eq_or_diff($out->get_output_entry('un3', $main), $un3, 'Uniquename namepart - 3');
eq_or_diff($out->get_output_entry('un4', $main), $un4, 'Uniquename namepart - 4');

eq_or_diff($out->get_output_entry('un1', $main1), $un1a, 'Uniquename namepart - 5');
eq_or_diff($out->get_output_entry('un2', $main1), $un2a, 'Uniquename namepart - 6');
eq_or_diff($out->get_output_entry('un3', $main1), $un3a, 'Uniquename namepart - 7');
eq_or_diff($out->get_output_entry('un4', $main1), $un4a, 'Uniquename namepart - 8');

eq_or_diff($main2->get_namestring($un1nlid, $un1nid), 'SmithSimon', 'Uniquename metadata - 1');
is_deeply($main2->get_namestrings($un1nlid, $un1nid),
          ['Smith', 'SmithS', 'SmithSimon'], 'Uniquename metadata - 2');
is_deeply($main2->get_namedisschema($un1nlid, $un1nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 3');
eq_or_diff($main2->get_namestring($un2nlid, $un2nid), 'SmithAlan', 'Uniquename metadata - 4');
is_deeply($main2->get_namestrings($un2nlid, $un2nid),
          ['Smith', 'SmithA', 'SmithAlan'], 'Uniquename metadata - 5');
is_deeply($main2->get_namedisschema($un2nlid, $un2nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 6');
eq_or_diff($main2->get_namestring($un3nlid, $un3nid), 'SmithArthur', 'Uniquename metadata - 7');
is_deeply($main2->get_namestrings($un3nlid, $un3nid),
          ['Smith', 'SmithA', 'SmithArthur'], 'Uniquename metadata - 8');
is_deeply($main2->get_namedisschema($un3nlid, $un3nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 9');
eq_or_diff($main2->get_namestring($un4nlid, $un4nid), 'SmithSimon', 'Uniquename metadata - 10');
is_deeply($main2->get_namestrings($un4nlid, $un4nid),
          ['Smith', 'SmithS', 'SmithSimon'], 'Uniquename metadata - 11');
is_deeply($main2->get_namedisschema($un4nlid, $un4nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 12');
eq_or_diff($main2->get_namestring($un5nlid, $un5nid), 'SmithSimon', 'Uniquename metadata - 13');
is_deeply($main2->get_namestrings($un5nlid, $un5nid),
          ['Smith', 'SmithSimon'], 'Uniquename metadata - 14');
is_deeply($main2->get_namedisschema($un5nlid, $un5nid),
          [['base', ['family']], ['middle', 'fullonly']], 'Uniquename metadata - 15');
eq_or_diff($main2->get_namestring($un6nlid, $un6nid), 'SmithSmythe', 'Uniquename metadata - 16');
is_deeply($main2->get_namestrings($un6nlid, $un6nid),
          ['Smith', 'SmithS'], 'Uniquename metadata - 17');
is_deeply($main2->get_namedisschema($un6nlid, $un6nid),
          [['base', ['family']], ['middle', 'init']], 'Uniquename metadata - 18');
eq_or_diff($main2->get_namestring($un7nlid, $un7nid), 'Smith', 'Uniquename metadata - 19');
is_deeply($main2->get_namestrings($un7nlid, $un7nid),
          ['Smith'], 'Uniquename metadata - 20');
is_deeply($main2->get_namedisschema($un7nlid, $un7nid),
          [['base', ['family']]], 'Uniquename metadata - 21');

eq_or_diff($out->get_output_entry('un1', $main2), $un1b, 'Uniquename namepart - 9');
eq_or_diff($out->get_output_entry('un2', $main2), $un2b, 'Uniquename namepart - 10');
eq_or_diff($out->get_output_entry('un3', $main2), $un3b, 'Uniquename namepart - 11');
eq_or_diff($out->get_output_entry('un4', $main2), $un4b, 'Uniquename namepart - 12');

# Note that these are all being tested against $main2, not the default list
eq_or_diff($out->get_output_entry('un5', $main2), $un5, 'Uniquename namepart - 13');
eq_or_diff($out->get_output_entry('un6', $main2), $un6, 'Uniquename namepart - 14');
eq_or_diff($out->get_output_entry('un7', $main2), $un7, 'Uniquename namepart - 15');
"####;

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_001_uniquename_namepart_1() {
    pass_upstream(
        "Uniquename namepart - 1",
        r####"$out->get_output_entry('un1', $main)"####,
        r####"$un1"####,
        r####"eq_or_diff($out->get_output_entry('un1', $main), $un1, 'Uniquename namepart - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_002_uniquename_namepart_2() {
    pass_upstream(
        "Uniquename namepart - 2",
        r####"$out->get_output_entry('un2', $main)"####,
        r####"$un2"####,
        r####"eq_or_diff($out->get_output_entry('un2', $main), $un2, 'Uniquename namepart - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_003_uniquename_namepart_3() {
    pass_upstream(
        "Uniquename namepart - 3",
        r####"$out->get_output_entry('un3', $main)"####,
        r####"$un3"####,
        r####"eq_or_diff($out->get_output_entry('un3', $main), $un3, 'Uniquename namepart - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_004_uniquename_namepart_4() {
    pass_upstream(
        "Uniquename namepart - 4",
        r####"$out->get_output_entry('un4', $main)"####,
        r####"$un4"####,
        r####"eq_or_diff($out->get_output_entry('un4', $main), $un4, 'Uniquename namepart - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_005_uniquename_namepart_5() {
    pass_upstream(
        "Uniquename namepart - 5",
        r####"$out->get_output_entry('un1', $main1)"####,
        r####"$un1a"####,
        r####"eq_or_diff($out->get_output_entry('un1', $main1), $un1a, 'Uniquename namepart - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_006_uniquename_namepart_6() {
    pass_upstream(
        "Uniquename namepart - 6",
        r####"$out->get_output_entry('un2', $main1)"####,
        r####"$un2a"####,
        r####"eq_or_diff($out->get_output_entry('un2', $main1), $un2a, 'Uniquename namepart - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_007_uniquename_namepart_7() {
    pass_upstream(
        "Uniquename namepart - 7",
        r####"$out->get_output_entry('un3', $main1)"####,
        r####"$un3a"####,
        r####"eq_or_diff($out->get_output_entry('un3', $main1), $un3a, 'Uniquename namepart - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_008_uniquename_namepart_8() {
    pass_upstream(
        "Uniquename namepart - 8",
        r####"$out->get_output_entry('un4', $main1)"####,
        r####"$un4a"####,
        r####"eq_or_diff($out->get_output_entry('un4', $main1), $un4a, 'Uniquename namepart - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_009_uniquename_metadata_1() {
    pass_upstream(
        "Uniquename metadata - 1",
        r####"$main2->get_namestring($un1nlid, $un1nid)"####,
        r####"'SmithSimon'"####,
        r####"eq_or_diff($main2->get_namestring($un1nlid, $un1nid), 'SmithSimon', 'Uniquename metadata - 1');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_010_uniquename_metadata_2() {
    pass_upstream(
        "Uniquename metadata - 2",
        r####"$main2->get_namestrings($un1nlid, $un1nid)"####,
        r####"['Smith', 'SmithS', 'SmithSimon']"####,
        r####"is_deeply($main2->get_namestrings($un1nlid, $un1nid),
          ['Smith', 'SmithS', 'SmithSimon'], 'Uniquename metadata - 2');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_011_uniquename_metadata_3() {
    pass_upstream(
        "Uniquename metadata - 3",
        r####"$main2->get_namedisschema($un1nlid, $un1nid)"####,
        r####"[['base', ['family']], ['middle', 'init'], ['middle', 'full']]"####,
        r####"is_deeply($main2->get_namedisschema($un1nlid, $un1nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 3');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_012_uniquename_metadata_4() {
    pass_upstream(
        "Uniquename metadata - 4",
        r####"$main2->get_namestring($un2nlid, $un2nid)"####,
        r####"'SmithAlan'"####,
        r####"eq_or_diff($main2->get_namestring($un2nlid, $un2nid), 'SmithAlan', 'Uniquename metadata - 4');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_013_uniquename_metadata_5() {
    pass_upstream(
        "Uniquename metadata - 5",
        r####"$main2->get_namestrings($un2nlid, $un2nid)"####,
        r####"['Smith', 'SmithA', 'SmithAlan']"####,
        r####"is_deeply($main2->get_namestrings($un2nlid, $un2nid),
          ['Smith', 'SmithA', 'SmithAlan'], 'Uniquename metadata - 5');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_014_uniquename_metadata_6() {
    pass_upstream(
        "Uniquename metadata - 6",
        r####"$main2->get_namedisschema($un2nlid, $un2nid)"####,
        r####"[['base', ['family']], ['middle', 'init'], ['middle', 'full']]"####,
        r####"is_deeply($main2->get_namedisschema($un2nlid, $un2nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 6');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_015_uniquename_metadata_7() {
    pass_upstream(
        "Uniquename metadata - 7",
        r####"$main2->get_namestring($un3nlid, $un3nid)"####,
        r####"'SmithArthur'"####,
        r####"eq_or_diff($main2->get_namestring($un3nlid, $un3nid), 'SmithArthur', 'Uniquename metadata - 7');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_016_uniquename_metadata_8() {
    pass_upstream(
        "Uniquename metadata - 8",
        r####"$main2->get_namestrings($un3nlid, $un3nid)"####,
        r####"['Smith', 'SmithA', 'SmithArthur']"####,
        r####"is_deeply($main2->get_namestrings($un3nlid, $un3nid),
          ['Smith', 'SmithA', 'SmithArthur'], 'Uniquename metadata - 8');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_017_uniquename_metadata_9() {
    pass_upstream(
        "Uniquename metadata - 9",
        r####"$main2->get_namedisschema($un3nlid, $un3nid)"####,
        r####"[['base', ['family']], ['middle', 'init'], ['middle', 'full']]"####,
        r####"is_deeply($main2->get_namedisschema($un3nlid, $un3nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_018_uniquename_metadata_10() {
    pass_upstream(
        "Uniquename metadata - 10",
        r####"$main2->get_namestring($un4nlid, $un4nid)"####,
        r####"'SmithSimon'"####,
        r####"eq_or_diff($main2->get_namestring($un4nlid, $un4nid), 'SmithSimon', 'Uniquename metadata - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_019_uniquename_metadata_11() {
    pass_upstream(
        "Uniquename metadata - 11",
        r####"$main2->get_namestrings($un4nlid, $un4nid)"####,
        r####"['Smith', 'SmithS', 'SmithSimon']"####,
        r####"is_deeply($main2->get_namestrings($un4nlid, $un4nid),
          ['Smith', 'SmithS', 'SmithSimon'], 'Uniquename metadata - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_020_uniquename_metadata_12() {
    pass_upstream(
        "Uniquename metadata - 12",
        r####"$main2->get_namedisschema($un4nlid, $un4nid)"####,
        r####"[['base', ['family']], ['middle', 'init'], ['middle', 'full']]"####,
        r####"is_deeply($main2->get_namedisschema($un4nlid, $un4nid),
          [['base', ['family']], ['middle', 'init'], ['middle', 'full']], 'Uniquename metadata - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_021_uniquename_metadata_13() {
    pass_upstream(
        "Uniquename metadata - 13",
        r####"$main2->get_namestring($un5nlid, $un5nid)"####,
        r####"'SmithSimon'"####,
        r####"eq_or_diff($main2->get_namestring($un5nlid, $un5nid), 'SmithSimon', 'Uniquename metadata - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_022_uniquename_metadata_14() {
    pass_upstream(
        "Uniquename metadata - 14",
        r####"$main2->get_namestrings($un5nlid, $un5nid)"####,
        r####"['Smith', 'SmithSimon']"####,
        r####"is_deeply($main2->get_namestrings($un5nlid, $un5nid),
          ['Smith', 'SmithSimon'], 'Uniquename metadata - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_023_uniquename_metadata_15() {
    pass_upstream(
        "Uniquename metadata - 15",
        r####"$main2->get_namedisschema($un5nlid, $un5nid)"####,
        r####"[['base', ['family']], ['middle', 'fullonly']]"####,
        r####"is_deeply($main2->get_namedisschema($un5nlid, $un5nid),
          [['base', ['family']], ['middle', 'fullonly']], 'Uniquename metadata - 15');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_024_uniquename_metadata_16() {
    pass_upstream(
        "Uniquename metadata - 16",
        r####"$main2->get_namestring($un6nlid, $un6nid)"####,
        r####"'SmithSmythe'"####,
        r####"eq_or_diff($main2->get_namestring($un6nlid, $un6nid), 'SmithSmythe', 'Uniquename metadata - 16');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_025_uniquename_metadata_17() {
    pass_upstream(
        "Uniquename metadata - 17",
        r####"$main2->get_namestrings($un6nlid, $un6nid)"####,
        r####"['Smith', 'SmithS']"####,
        r####"is_deeply($main2->get_namestrings($un6nlid, $un6nid),
          ['Smith', 'SmithS'], 'Uniquename metadata - 17');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_026_uniquename_metadata_18() {
    pass_upstream(
        "Uniquename metadata - 18",
        r####"$main2->get_namedisschema($un6nlid, $un6nid)"####,
        r####"[['base', ['family']], ['middle', 'init']]"####,
        r####"is_deeply($main2->get_namedisschema($un6nlid, $un6nid),
          [['base', ['family']], ['middle', 'init']], 'Uniquename metadata - 18');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_027_uniquename_metadata_19() {
    pass_upstream(
        "Uniquename metadata - 19",
        r####"$main2->get_namestring($un7nlid, $un7nid)"####,
        r####"'Smith'"####,
        r####"eq_or_diff($main2->get_namestring($un7nlid, $un7nid), 'Smith', 'Uniquename metadata - 19');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_028_uniquename_metadata_20() {
    pass_upstream(
        "Uniquename metadata - 20",
        r####"$main2->get_namestrings($un7nlid, $un7nid)"####,
        r####"['Smith']"####,
        r####"is_deeply($main2->get_namestrings($un7nlid, $un7nid),
          ['Smith'], 'Uniquename metadata - 20');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_029_uniquename_metadata_21() {
    pass_upstream(
        "Uniquename metadata - 21",
        r####"$main2->get_namedisschema($un7nlid, $un7nid)"####,
        r####"[['base', ['family']]]"####,
        r####"is_deeply($main2->get_namedisschema($un7nlid, $un7nid),
          [['base', ['family']]], 'Uniquename metadata - 21');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_030_uniquename_namepart_9() {
    pass_upstream(
        "Uniquename namepart - 9",
        r####"$out->get_output_entry('un1', $main2)"####,
        r####"$un1b"####,
        r####"eq_or_diff($out->get_output_entry('un1', $main2), $un1b, 'Uniquename namepart - 9');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_031_uniquename_namepart_10() {
    pass_upstream(
        "Uniquename namepart - 10",
        r####"$out->get_output_entry('un2', $main2)"####,
        r####"$un2b"####,
        r####"eq_or_diff($out->get_output_entry('un2', $main2), $un2b, 'Uniquename namepart - 10');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_032_uniquename_namepart_11() {
    pass_upstream(
        "Uniquename namepart - 11",
        r####"$out->get_output_entry('un3', $main2)"####,
        r####"$un3b"####,
        r####"eq_or_diff($out->get_output_entry('un3', $main2), $un3b, 'Uniquename namepart - 11');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_033_uniquename_namepart_12() {
    pass_upstream(
        "Uniquename namepart - 12",
        r####"$out->get_output_entry('un4', $main2)"####,
        r####"$un4b"####,
        r####"eq_or_diff($out->get_output_entry('un4', $main2), $un4b, 'Uniquename namepart - 12');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_034_uniquename_namepart_13() {
    pass_upstream(
        "Uniquename namepart - 13",
        r####"$out->get_output_entry('un5', $main2)"####,
        r####"$un5"####,
        r####"eq_or_diff($out->get_output_entry('un5', $main2), $un5, 'Uniquename namepart - 13');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_035_uniquename_namepart_14() {
    pass_upstream(
        "Uniquename namepart - 14",
        r####"$out->get_output_entry('un6', $main2)"####,
        r####"$un6"####,
        r####"eq_or_diff($out->get_output_entry('un6', $main2), $un6, 'Uniquename namepart - 14');"####,
        UPSTREAM_SOURCE,
    );
}

#[test]
#[ignore = "xfail: bib-engine has no public name-part uniqueness or BBL query API"]
fn assertion_036_uniquename_namepart_15() {
    pass_upstream(
        "Uniquename namepart - 15",
        r####"$out->get_output_entry('un7', $main2)"####,
        r####"$un7"####,
        r####"eq_or_diff($out->get_output_entry('un7', $main2), $un7, 'Uniquename namepart - 15');"####,
        UPSTREAM_SOURCE,
    );
}
