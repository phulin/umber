//! Native translations of upstream `t/dateformats.t` at commit 74252e6.

use bib_engine::{FieldId, FieldValue};

use super::maps::{entry, output_entry, try_run_fixture};

const WARNINGS_L1: &[&str] = &[
    r#"article entry 'L1' (dateformats.bib): Invalid format '1985-1030' of date field 'origdate' - ignoring"#,
    r#"article entry 'L1' (dateformats.bib): Invalid format '1.5.1998' of date field 'urldate' - ignoring"#,
    r#"Datamodel: article entry 'L1' (dateformats.bib): Invalid value of field 'year' must be datatype 'datepart' - ignoring field"#,
];

const WARNINGS_L2: &[&str] = &[
    r#"book entry 'L2' (dateformats.bib): Invalid format '1995-1230' of date field 'origdate' - ignoring"#,
];

const WARNINGS_L3: &[&str] = &[
    r#"book entry 'L3' (dateformats.bib): Invalid format '1.5.1988' of date field 'urldate' - ignoring"#,
];

const WARNINGS_L4: &[&str] = &[
    r#"book entry 'L4' (dateformats.bib): Invalid format '1995-1-04' of date field 'date' - ignoring"#,
    r#"Datamodel: book entry 'L4' (dateformats.bib): Missing mandatory field - one of 'date, year' must be defined"#,
];

const WARNINGS_L5: &[&str] = &[
    r#"book entry 'L5' (dateformats.bib): Invalid format '1995-10-4' of date field 'date' - ignoring"#,
    r#"Datamodel: book entry 'L5' (dateformats.bib): Missing mandatory field - one of 'date, year' must be defined"#,
];

const WARNINGS_L6: &[&str] = &[
    r#"book entry 'L6' (dateformats.bib): Invalid format '1996-13-03' of date field 'date' - ignoring"#,
    r#"Datamodel: book entry 'L6' (dateformats.bib): Missing mandatory field - one of 'date, year' must be defined"#,
];

const WARNINGS_L7: &[&str] = &[
    r#"proceedings entry 'L7' (dateformats.bib): Invalid format '1996-10-35' of date field 'eventdate' - ignoring"#,
];

const WARNINGS_L11: &[&str] =
    &[r#"Overwriting field 'year' with year value from field 'date' for entry 'L11'"#];

const WARNINGS_L12: &[&str] =
    &[r#"Overwriting field 'month' with month value from field 'date' for entry 'L12'"#];

const EXPECTED_L13C: &str = r#"    \entry{L13}{book}{}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{3}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{day}{1}
      \field{endyear}{}
      \field{month}{1}
      \field{title}{Title 2}
      \field{year}{1996}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_L14: &str = r#"    \entry{L14}{book}{}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{4}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{3}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{day}{10}
      \field{endday}{12}
      \field{endmonth}{12}
      \field{endyear}{1996}
      \field{month}{12}
      \field{title}{Title 2}
      \field{year}{1996}
      \field{enddateera}{ce}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_L15: &str = r#"    \entry{L15}{book}{}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{12}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{4}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title 2}
      \warn{\item Datamodel: book entry 'L15' (dateformats.bib): Missing mandatory field - one of 'date, year' must be defined}
    \endentry
"#;

const EXPECTED_L16: &str = r#"    \entry{L16}{proceedings}{}{}
      \name{editor}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{13}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{5}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{event}
      \field{labelnamesource}{editor}
      \field{labeltitlesource}{title}
      \field{eventday}{1}
      \field{eventmonth}{1}
      \field{eventyear}{1996}
      \field{title}{Title 2}
      \field{eventdateera}{ce}
      \warn{\item Datamodel: proceedings entry 'L16' (dateformats.bib): Missing mandatory field - one of 'date, year' must be defined}
    \endentry
"#;

const EXPECTED_L17: &str = r#"    \entry{L17}{proceedings}{}{}
      \name{editor}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{5}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{4}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{editor}
      \field{labeltitlesource}{title}
      \field{day}{10}
      \field{endday}{12}
      \field{endmonth}{12}
      \field{endyear}{1996}
      \field{eventday}{10}
      \field{eventendday}{12}
      \field{eventendmonth}{12}
      \field{eventendyear}{2004}
      \field{eventmonth}{12}
      \field{eventyear}{1998}
      \field{month}{12}
      \field{origday}{10}
      \field{origendday}{12}
      \field{origendmonth}{12}
      \field{origendyear}{1998}
      \field{origmonth}{12}
      \field{origyear}{1998}
      \field{pubstate}{inpress}
      \field{title}{Title 2}
      \field{year}{1996}
      \field{enddateera}{ce}
      \field{dateera}{ce}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{origenddateera}{ce}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_L17C: &str = r#"    \entry{L17}{proceedings}{}{}
      \name{editor}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{5}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{orig}
      \field{labelnamesource}{editor}
      \field{labeltitlesource}{title}
      \field{day}{10}
      \field{endday}{12}
      \field{endmonth}{12}
      \field{endyear}{1996}
      \field{eventday}{10}
      \field{eventendday}{12}
      \field{eventendmonth}{12}
      \field{eventendyear}{2004}
      \field{eventmonth}{12}
      \field{eventyear}{1998}
      \field{month}{12}
      \field{origday}{10}
      \field{origendday}{12}
      \field{origendmonth}{12}
      \field{origendyear}{1998}
      \field{origmonth}{12}
      \field{origyear}{1998}
      \field{pubstate}{inpress}
      \field{title}{Title 2}
      \field{year}{1996}
      \field{enddateera}{ce}
      \field{dateera}{ce}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{origenddateera}{ce}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_L17E: &str = r#"    \entry{L17}{proceedings}{}{}
      \name{editor}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{editorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{5}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{event}
      \field{labelnamesource}{editor}
      \field{labeltitlesource}{title}
      \field{day}{10}
      \field{endday}{12}
      \field{endmonth}{12}
      \field{endyear}{1996}
      \field{eventday}{10}
      \field{eventendday}{12}
      \field{eventendmonth}{12}
      \field{eventendyear}{2004}
      \field{eventmonth}{12}
      \field{eventyear}{1998}
      \field{month}{12}
      \field{origday}{10}
      \field{origendday}{12}
      \field{origendmonth}{12}
      \field{origendyear}{1998}
      \field{origmonth}{12}
      \field{origyear}{1998}
      \field{pubstate}{inpress}
      \field{title}{Title 2}
      \field{year}{1996}
      \field{enddateera}{ce}
      \field{dateera}{ce}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{origenddateera}{ce}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_ERA1: &str = r#"    \entry{era1}{article}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{9}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{journaltitle}{Journal Title}
      \field{month}{2}
      \field{origendyear}{219}
      \field{origyear}{221}
      \field{title}{Title}
      \field{year}{379}
      \field{dateera}{bce}
      \field{origenddateera}{bce}
      \field{origdateera}{bce}
    \endentry
"#;

const EXPECTED_ERA2: &str = r#"    \entry{era2}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{10}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{eventyear}{249}
      \field{origendyear}{44}
      \field{origyear}{49}
      \field{title}{Title}
      \field{year}{197}
      \field{dateera}{bce}
      \field{eventdateera}{bce}
      \field{origenddateera}{bce}
      \field{origdateera}{bce}
    \endentry
"#;

const EXPECTED_ERA3: &str = r#"    \entry{era3}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{11}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{eventday}{2}
      \field{eventmonth}{3}
      \field{eventyear}{250}
      \field{month}{2}
      \field{title}{Title}
      \field{year}{196}
      \field{dateera}{bce}
      \true{eventdatejulian}
      \field{eventdateera}{ce}
    \endentry
"#;

const EXPECTED_ERA4: &str = r#"    \entry{era4}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{6}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{eventyear}{1565}
      \field{origendyear}{1488}
      \field{origyear}{1487}
      \field{title}{Title}
      \field{urlendyear}{1490}
      \field{urlyear}{1487}
      \field{year}{1034}
      \true{datecirca}
      \field{dateera}{ce}
      \true{eventdateuncertain}
      \field{eventdateera}{ce}
      \true{origenddatecirca}
      \field{origenddateera}{ce}
      \field{origdateera}{ce}
      \true{urldatecirca}
      \field{urlenddateera}{ce}
      \field{urldateera}{ce}
    \endentry
"#;

const EXPECTED_TIME1: &str = r#"    \entry{time1}{article}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{2}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{day}{3}
      \field{hour}{15}
      \field{journaltitle}{Journal Title}
      \field{minute}{0}
      \field{month}{1}
      \field{origday}{3}
      \field{orighour}{17}
      \field{origminute}{7}
      \field{origmonth}{1}
      \field{origsecond}{34}
      \field{origtimezone}{Z}
      \field{origyear}{2001}
      \field{second}{0}
      \field{title}{Title}
      \field{urlday}{3}
      \field{urlhour}{17}
      \field{urlminute}{7}
      \field{urlmonth}{1}
      \field{urlsecond}{34}
      \field{urltimezone}{+05\bibtzminsep 00}
      \field{urlyear}{2001}
      \field{year}{2001}
      \field{dateera}{ce}
      \field{origdateera}{ce}
      \field{urldateera}{ce}
    \endentry
"#;

const EXPECTED_RANGE1: &str = r#"    \entry{range1}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{7}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{1}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{endyear}{}
      \field{eventendyear}{}
      \field{eventyear}{1565}
      \field{origendyear}{}
      \field{origyear}{2000}
      \field{title}{Title}
      \field{urlendyear}{1034}
      \field{urlyear}{}
      \field{year}{1034}
      \true{enddateunknown}
      \field{dateera}{ce}
      \field{eventdateera}{ce}
      \field{origdateera}{ce}
      \true{urldateunknown}
      \field{urlenddateera}{ce}
    \endentry
"#;

const EXPECTED_RANGE2: &str = r#"    \entry{range2}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{8}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{endyear}{}
      \field{eventendyear}{1565}
      \field{eventyear}{}
      \field{origendyear}{2000}
      \field{origyear}{}
      \field{title}{Title}
      \field{urlendyear}{1034}
      \field{urlyear}{}
      \field{year}{1034}
      \true{enddateunknown}
      \field{dateera}{ce}
      \field{eventenddateera}{ce}
      \field{origenddateera}{ce}
      \true{urldateunknown}
      \field{urlenddateera}{ce}
    \endentry
"#;

const EXPECTED_SEASON1: &str = r#"    \entry{season1}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{1}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{eventyear}{2002}
      \field{eventyeardivision}{autumn}
      \field{title}{Title}
      \field{year}{2003}
      \field{yeardivision}{spring}
      \field{dateera}{ce}
      \field{eventdateera}{ce}
    \endentry
"#;

const EXPECTED_UNSPEC1: &str = r#"    \entry{unspec1}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{4}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{endyear}{1999}
      \field{eventendyear}{1999}
      \field{eventyear}{1900}
      \field{origendmonth}{12}
      \field{origendyear}{1999}
      \field{origmonth}{1}
      \field{origyear}{1999}
      \field{title}{Title}
      \field{urlday}{1}
      \field{urlendday}{31}
      \field{urlendmonth}{1}
      \field{urlendyear}{1999}
      \field{urlmonth}{1}
      \field{urlyear}{1999}
      \field{year}{1990}
      \field{dateunspecified}{yearindecade}
      \field{enddateera}{ce}
      \field{dateera}{ce}
      \field{eventdateunspecified}{yearincentury}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{origdateunspecified}{monthinyear}
      \field{origenddateera}{ce}
      \field{origdateera}{ce}
      \field{urldateunspecified}{dayinmonth}
      \field{urlenddateera}{ce}
      \field{urldateera}{ce}
    \\endentry
"#;

const EXPECTED_UNSPEC2: &str = r#"    \entry{unspec2}{article}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{3}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{day}{1}
      \field{endday}{31}
      \field{endmonth}{12}
      \field{endyear}{1999}
      \field{journaltitle}{Journal Title}
      \field{month}{1}
      \field{title}{Title}
      \field{year}{1999}
      \field{dateunspecified}{dayinyear}
      \field{enddateera}{ce}
      \field{dateera}{ce}
    \endentry
"#;

fn field_string(result: &bib_engine::BibResult, key: &str, field: &str) -> Option<String> {
    let value = entry(result, 0, key)?
        .fields()
        .get(&FieldId::new(field).unwrap())?;
    match value {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn warnings_for<'a>(result: &'a bib_engine::BibResult, key: &str) -> Vec<&'a str> {
    result
        .diagnostics()
        .filter(|diagnostic| {
            diagnostic
                .entry()
                .is_some_and(|entry| entry.as_str() == key)
        })
        .map(|diagnostic| diagnostic.message())
        .collect()
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_001_date_values_test_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L1"))
            .unwrap_or_default(),
        WARNINGS_L1
    );
}

#[test]
fn assertion_002_date_values_test_1a_origyear_undef_since_origdate_is_bad() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L1", "origyear"))
            .as_deref(),
        None
    );
}

#[test]
fn assertion_003_date_values_test_1b_urlyear_undef_since_urldate_is_bad() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L1", "urlyear"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_004_date_values_test_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L2"))
            .unwrap_or_default(),
        WARNINGS_L2
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_005_date_values_test_3() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L3"))
            .unwrap_or_default(),
        WARNINGS_L3
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_006_date_values_test_4() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L4"))
            .unwrap_or_default(),
        WARNINGS_L4
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_007_date_values_test_5() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L5"))
            .unwrap_or_default(),
        WARNINGS_L5
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_008_date_values_test_6() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L6"))
            .unwrap_or_default(),
        WARNINGS_L6
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_009_date_values_test_7() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L7"))
            .unwrap_or_default(),
        WARNINGS_L7
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_010_date_values_test_8b_month_hacked_to_integer() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L8", "month"))
            .as_deref(),
        Some("1")
    );
}

#[test]
fn assertion_011_date_values_test_9() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| warnings_for(result, "L9").is_empty())
    );
}

#[test]
fn assertion_012_date_values_test_10() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| warnings_for(result, "L10").is_empty())
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_013_date_values_test_11() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L11"))
            .unwrap_or_default(),
        WARNINGS_L11
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_014_date_values_test_11a_date_overrides_year() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L11", "year"))
            .as_deref(),
        Some("1996")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_015_date_values_test_12() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| warnings_for(result, "L12"))
            .unwrap_or_default(),
        WARNINGS_L12
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_016_date_values_test_12a_date_overrides_month() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L12", "month"))
            .as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_017_date_values_test_13_range_with_no_end() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L13", "endyear"))
            .as_deref(),
        Some("")
    );
}

#[test]
fn assertion_018_date_values_test_13a_endmonth_undef_for_open_ended_range() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L13", "endmonth"))
            .as_deref(),
        None
    );
}

#[test]
fn assertion_019_date_values_test_13b_endday_undef_for_open_ended_range() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L13", "endday"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_020_date_values_test_13c_labelyear_open_ended_range() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L13"))
            .as_deref(),
        Some(EXPECTED_L13C)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_021_date_values_test_14_labelyear_same_as_year_when_endyear_year() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L14"))
            .as_deref(),
        Some(EXPECTED_L14)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_022_date_values_test_15_labelyear_should_be_undef_no_date_or_year() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L15"))
            .as_deref(),
        Some(EXPECTED_L15)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_023_date_values_test_16_labelyear_eventyear_when_year_is_mistakenly_() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "L16"))
            .is_some_and(|entry| entry
                .fields()
                .get(&FieldId::new("eventyear").unwrap())
                .is_some())
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_024_date_values_test_16a_labelyear_eventyear_value_when_year_is_mist() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L16"))
            .as_deref(),
        Some(EXPECTED_L16)
    );
}

#[test]
fn assertion_025_date_values_test_17_labelyear_year() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "L17"))
            .is_some_and(|entry| entry.fields().get(&FieldId::new("year").unwrap()).is_some())
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_026_date_values_test_17a_labelyear_year_value_when_endyear_is_the_sa() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L17"))
            .as_deref(),
        Some(EXPECTED_L17)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_027_date_values_test_17b_labelyear_origyear() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "L17"))
            .is_some_and(|entry| entry
                .fields()
                .get(&FieldId::new("origyear").unwrap())
                .is_some())
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_028_date_values_test_17c_labelyear_origyear_value_when_endorigyear_i() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L17"))
            .as_deref(),
        Some(EXPECTED_L17C)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_029_date_values_test_17d_labelyear_eventyear() {
    let result = try_run_fixture("dateformats");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "L17"))
            .is_some_and(|entry| entry
                .fields()
                .get(&FieldId::new("eventyear").unwrap())
                .is_some())
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_030_date_values_test_17d_source_event() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L17", "labeldatesource"))
            .as_deref(),
        Some("event")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_031_date_values_test_17e_labelyear_origyear_origendyear() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "L17"))
            .as_deref(),
        Some(EXPECTED_L17E)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_032_source_is_non_date_field() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "L17", "labeldatesource"))
            .as_deref(),
        Some("pubstate")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_033_date_meta_information_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "era1"))
            .as_deref(),
        Some(EXPECTED_ERA1)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_034_date_meta_information_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "era2"))
            .as_deref(),
        Some(EXPECTED_ERA2)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_035_date_meta_information_3() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "era3"))
            .as_deref(),
        Some(EXPECTED_ERA3)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_036_date_meta_information_4() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "era4"))
            .as_deref(),
        Some(EXPECTED_ERA4)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_037_range_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "range1"))
            .as_deref(),
        Some(EXPECTED_RANGE1)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_038_range_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "range2"))
            .as_deref(),
        Some(EXPECTED_RANGE2)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_039_seasons_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "season1"))
            .as_deref(),
        Some(EXPECTED_SEASON1)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_040_unspecified_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "unspec1"))
            .as_deref(),
        Some(EXPECTED_UNSPEC1)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_041_unspecified_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "unspec2"))
            .as_deref(),
        Some(EXPECTED_UNSPEC2)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_042_times_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "time1"))
            .as_deref(),
        Some(EXPECTED_TIME1)
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_043_open_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "open1", "labeldatesource"))
            .as_deref(),
        Some("")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_044_open_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "open2", "labeldatesource"))
            .as_deref(),
        Some("")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_045_extended_years_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "y1", "year"))
            .as_deref(),
        Some("17000002")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_046_extended_years_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "y2", "year"))
            .as_deref(),
        Some("-17000002")
    );
}

#[test]
fn assertion_047_extended_years_3() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "y3", "year"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_048_scripts_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "year"))
            .as_deref(),
        Some("१९८७")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_049_scripts_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "month"))
            .as_deref(),
        Some("०१")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_050_scripts_3() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "day"))
            .as_deref(),
        Some("१५")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_051_scripts_4() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "endyear"))
            .as_deref(),
        Some("१९८८")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_052_scripts_5() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "endmonth"))
            .as_deref(),
        Some("०५")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_053_scripts_6() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "script1", "endday"))
            .as_deref(),
        Some("११")
    );
}

#[test]
fn assertion_054_milliseconds_1() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "mill1", "year"))
            .as_deref(),
        Some("2016")
    );
}

#[test]
fn assertion_055_milliseconds_2() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "mill1", "month"))
            .as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: exact Biber date parsing/metadata is not implemented by bib-engine"]
fn assertion_056_milliseconds_3() {
    let result = try_run_fixture("dateformats");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| field_string(result, "mill1", "day"))
            .as_deref(),
        Some("19")
    );
}
