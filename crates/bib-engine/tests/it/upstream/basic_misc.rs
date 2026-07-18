//! Native translations of upstream `t/basic-misc.t` at commit 74252e6.

use bib_engine::{Entry, FieldId, FieldValue, RangeEndpoint, SectionId};

use super::maps::{
    entry, list_keys, output_entry, output_text, section_entry_keys, try_run_fixture,
};

const EXPECTED_U1: &str = r#"    \entry{u1}{misc}{}{}
      \name{author}{4}{ul=4}{%
        {{un=0,uniquepart=base,hash=e1faffb3e614e6c2fba74296962386b7}{%
           family={AAA},
           familyi={A\bibinitperiod}}}%
        {{un=0,uniquepart=base,hash=2bb225f0ba9a58930757a868ed57d9a3}{%
           family={BBB},
           familyi={B\bibinitperiod}}}%
        {{un=0,uniquepart=base,hash=defb99e69a9f1f6e06f15006b1f166ae}{%
           family={CCC},
           familyi={C\bibinitperiod}}}%
        {{un=0,uniquepart=base,hash=45054f47ac3305a2a33e9bcceadff712}{%
           family={DDD},
           familyi={D\bibinitperiod}}}%
      }
      \strng{namehash}{b78abdc838d79b6576f2ed0021642766}
      \strng{fullhash}{b78abdc838d79b6576f2ed0021642766}
      \strng{fullhashraw}{b78abdc838d79b6576f2ed0021642766}
      \strng{bibnamehash}{b78abdc838d79b6576f2ed0021642766}
      \strng{authorbibnamehash}{b78abdc838d79b6576f2ed0021642766}
      \strng{authornamehash}{b78abdc838d79b6576f2ed0021642766}
      \strng{authorfullhash}{b78abdc838d79b6576f2ed0021642766}
      \strng{authorfullhashraw}{b78abdc838d79b6576f2ed0021642766}
      \field{labelalpha}{AAA\textbf{+}00}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{singletitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{note}{0}
      \field{title}{A title}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_MURRAY1: &str = r#"    \entry{murray}{article}{}{}
      \name{author}{14}{}{%
        {{un=0,uniquepart=base,hash=1c180cd8a2042c60a0f1dda22e34794a}{%
           family={Hostetler},
           familyi={H\bibinitperiod},
           given={Michael\bibnamedelima J.},
           giveni={M\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=2030d395cebb15e0da06bd52f115049e}{%
           family={Wingate},
           familyi={W\bibinitperiod},
           given={Julia\bibnamedelima E.},
           giveni={J\bibinitperiod\bibinitdelim E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=76100791c221471771c6bf1dbbc0975d}{%
           family={Zhong},
           familyi={Z\bibinitperiod},
           given={Chuan-Jian},
           giveni={C\bibinithyphendelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=06d5aa45c5c29069024ba0cdd1d32ead}{%
           family={Harris},
           familyi={H\bibinitperiod},
           given={Jay\bibnamedelima E.},
           giveni={J\bibinitperiod\bibinitdelim E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=e9020055de8cefbc4a22aa3232d3fbab}{%
           family={Vachet},
           familyi={V\bibinitperiod},
           given={Richard\bibnamedelima W.},
           giveni={R\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=7e4f8edb775a146bcbbeeafed50a0360}{%
           family={Clark},
           familyi={C\bibinitperiod},
           given={Michael\bibnamedelima R.},
           giveni={M\bibinitperiod\bibinitdelim R\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=835e54469f856c3fcb684033251ee209}{%
           family={Londono},
           familyi={L\bibinitperiod},
           given={J.\bibnamedelimi David},
           giveni={J\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=c36de3411f9881a05ab58a3db9a5b945}{%
           family={Green},
           familyi={G\bibinitperiod},
           given={Stephen\bibnamedelima J.},
           giveni={S\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=aed4e73d98e6c7b39ce8dd8daeec0702}{%
           family={Stokes},
           familyi={S\bibinitperiod},
           given={Jennifer\bibnamedelima J.},
           giveni={J\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=13e766e053b44242f97acf0b776df72b}{%
           family={Wignall},
           familyi={W\bibinitperiod},
           given={George\bibnamedelima D.},
           giveni={G\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=9a1aea8c8c5706554440d568ccbc0850}{%
           family={Glish},
           familyi={G\bibinitperiod},
           given={Gary\bibnamedelima L.},
           giveni={G\bibinitperiod\bibinitdelim L\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=ab2f6cef185e0b8cc0360f2fd84bcb3f}{%
           family={Porter},
           familyi={P\bibinitperiod},
           given={Marc\bibnamedelima D.},
           giveni={M\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=84f346653233f1532a42873769dd6553}{%
           family={Evans},
           familyi={E\bibinitperiod},
           given={Neal\bibnamedelima D.},
           giveni={N\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=ea266d9882f073db41b2567f03c3da5c}{%
           family={Murray},
           familyi={M\bibinitperiod},
           given={Royce\bibnamedelima W.},
           giveni={R\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{0c2086c92b65b24b0fb04b9462cf6c00}
      \strng{fullhash}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{fullhashraw}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{bibnamehash}{132c55db0f03fae26126bc20d94cd834}
      \strng{authorbibnamehash}{132c55db0f03fae26126bc20d94cd834}
      \strng{authornamehash}{0c2086c92b65b24b0fb04b9462cf6c00}
      \strng{authorfullhash}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{authorfullhashraw}{1572cc3fd324f560e5e71d041a6bd764}
      \field{labelalpha}{Hos\textbf{+}98}
      \field{sortinit}{H}
      \field{sortinithash}{23a3aa7c24e56cfa16945d55545109b5}
      \true{singletitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{shorttitle}
      \field{annotation}{An \texttt{article} entry with \arabic{author} authors. By default, long author and editor lists are automatically truncated. This is configurable}
      \field{indextitle}{Alkanethiolate gold cluster molecules}
      \field{journaltitle}{Langmuir}
      \field{langid}{english}
      \field{langidopts}{variant=american}
      \field{number}{1}
      \field{shorttitle}{Alkanethiolate gold cluster molecules}
      \field{subtitle}{Core and monolayer properties as a function of core size}
      \field{title}{Alkanethiolate gold cluster molecules with core diameters from 1.5 to 5.2~nm}
      \field{volume}{14}
      \field{year}{1998}
      \field{pages}{17\bibrangedash 30}
      \range{pages}{14}
      \keyw{keyw1,keyw2}
    \endentry
"#;

const EXPECTED_MURRAY2: &str = r#"    \entry{murray}{article}{}{}
      \name{author}{14}{}{%
        {{un=0,uniquepart=base,hash=1c180cd8a2042c60a0f1dda22e34794a}{%
           family={Hostetler},
           familyi={H\bibinitperiod},
           given={Michael\bibnamedelima J.},
           giveni={M\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=2030d395cebb15e0da06bd52f115049e}{%
           family={Wingate},
           familyi={W\bibinitperiod},
           given={Julia\bibnamedelima E.},
           giveni={J\bibinitperiod\bibinitdelim E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=76100791c221471771c6bf1dbbc0975d}{%
           family={Zhong},
           familyi={Z\bibinitperiod},
           given={Chuan-Jian},
           giveni={C\bibinithyphendelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=06d5aa45c5c29069024ba0cdd1d32ead}{%
           family={Harris},
           familyi={H\bibinitperiod},
           given={Jay\bibnamedelima E.},
           giveni={J\bibinitperiod\bibinitdelim E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=e9020055de8cefbc4a22aa3232d3fbab}{%
           family={Vachet},
           familyi={V\bibinitperiod},
           given={Richard\bibnamedelima W.},
           giveni={R\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=7e4f8edb775a146bcbbeeafed50a0360}{%
           family={Clark},
           familyi={C\bibinitperiod},
           given={Michael\bibnamedelima R.},
           giveni={M\bibinitperiod\bibinitdelim R\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=835e54469f856c3fcb684033251ee209}{%
           family={Londono},
           familyi={L\bibinitperiod},
           given={J.\bibnamedelimi David},
           giveni={J\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=c36de3411f9881a05ab58a3db9a5b945}{%
           family={Green},
           familyi={G\bibinitperiod},
           given={Stephen\bibnamedelima J.},
           giveni={S\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=aed4e73d98e6c7b39ce8dd8daeec0702}{%
           family={Stokes},
           familyi={S\bibinitperiod},
           given={Jennifer\bibnamedelima J.},
           giveni={J\bibinitperiod\bibinitdelim J\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=13e766e053b44242f97acf0b776df72b}{%
           family={Wignall},
           familyi={W\bibinitperiod},
           given={George\bibnamedelima D.},
           giveni={G\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=9a1aea8c8c5706554440d568ccbc0850}{%
           family={Glish},
           familyi={G\bibinitperiod},
           given={Gary\bibnamedelima L.},
           giveni={G\bibinitperiod\bibinitdelim L\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=ab2f6cef185e0b8cc0360f2fd84bcb3f}{%
           family={Porter},
           familyi={P\bibinitperiod},
           given={Marc\bibnamedelima D.},
           giveni={M\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=84f346653233f1532a42873769dd6553}{%
           family={Evans},
           familyi={E\bibinitperiod},
           given={Neal\bibnamedelima D.},
           giveni={N\bibinitperiod\bibinitdelim D\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=ea266d9882f073db41b2567f03c3da5c}{%
           family={Murray},
           familyi={M\bibinitperiod},
           given={Royce\bibnamedelima W.},
           giveni={R\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{0c2086c92b65b24b0fb04b9462cf6c00}
      \strng{fullhash}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{fullhashraw}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{bibnamehash}{132c55db0f03fae26126bc20d94cd834}
      \strng{authorbibnamehash}{132c55db0f03fae26126bc20d94cd834}
      \strng{authornamehash}{0c2086c92b65b24b0fb04b9462cf6c00}
      \strng{authorfullhash}{1572cc3fd324f560e5e71d041a6bd764}
      \strng{authorfullhashraw}{1572cc3fd324f560e5e71d041a6bd764}
      \field{labelalpha}{Hos98}
      \field{sortinit}{H}
      \field{sortinithash}{23a3aa7c24e56cfa16945d55545109b5}
      \true{singletitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{shorttitle}
      \field{annotation}{An \texttt{article} entry with \arabic{author} authors. By default, long author and editor lists are automatically truncated. This is configurable}
      \field{indextitle}{Alkanethiolate gold cluster molecules}
      \field{journaltitle}{Langmuir}
      \field{langid}{english}
      \field{langidopts}{variant=american}
      \field{number}{1}
      \field{shorttitle}{Alkanethiolate gold cluster molecules}
      \field{subtitle}{Core and monolayer properties as a function of core size}
      \field{title}{Alkanethiolate gold cluster molecules with core diameters from 1.5 to 5.2~nm}
      \field{volume}{14}
      \field{year}{1998}
      \field{pages}{17\bibrangedash 30}
      \range{pages}{14}
      \keyw{keyw1,keyw2}
    \endentry
"#;

const EXPECTED_T1: &str = r#"    \entry{t1}{misc}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=858fcf9483ec29b7707a7dda2dde7a6f}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={Bill},
           giveni={B\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{fullhash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{fullhashraw}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{bibnamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorbibnamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authornamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorfullhash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorfullhashraw}{858fcf9483ec29b7707a7dda2dde7a6f}
      \field{extraname}{1}
      \field{labelalpha}{Bro92}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{10\% of [100] and 90% of $Normal_2$ | \& # things {$^{3}$}}
      \field{year}{1992}
      \field{pages}{100\bibrangedash}
      \range{pages}{-1}
      \keyw{primary,something,somethingelse}
    \endentry
"#;

const EXPECTED_T2: &str = r#"    \entry{t2}{misc}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=858fcf9483ec29b7707a7dda2dde7a6f}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={Bill},
           giveni={B\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{fullhash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{fullhashraw}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{bibnamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorbibnamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authornamehash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorfullhash}{858fcf9483ec29b7707a7dda2dde7a6f}
      \strng{authorfullhashraw}{858fcf9483ec29b7707a7dda2dde7a6f}
      \field{extraname}{2}
      \field{labelalpha}{Bro94}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Signs of W$\frac{o}{a}$nder}
      \field{year}{1994}
      \field{pages}{100\bibrangedash 108}
      \range{pages}{9}
    \endentry
"#;

const EXPECTED_ANON1: &str = r#"    \entry{anon1}{unpublished}{}{}
      \name{author}{1}{}{%
        {{hash=a66f357fe2fd356fe49959173522a651}{%
           family={AnonymousX},
           familyi={A\bibinitperiod}}}%
      }
      \name{shortauthor}{1}{}{%
        {{un=0,uniquepart=base,hash=9873a6cc65c553faa2b21aaad626fe4b}{%
           family={XAnony},
           familyi={X\bibinitperiod}}}%
      }
      \strng{namehash}{9873a6cc65c553faa2b21aaad626fe4b}
      \strng{fullhash}{a66f357fe2fd356fe49959173522a651}
      \strng{fullhashraw}{a66f357fe2fd356fe49959173522a651}
      \strng{bibnamehash}{9873a6cc65c553faa2b21aaad626fe4b}
      \strng{authorbibnamehash}{a66f357fe2fd356fe49959173522a651}
      \strng{authornamehash}{a66f357fe2fd356fe49959173522a651}
      \strng{authorfullhash}{a66f357fe2fd356fe49959173522a651}
      \strng{authorfullhashraw}{a66f357fe2fd356fe49959173522a651}
      \strng{shortauthorbibnamehash}{9873a6cc65c553faa2b21aaad626fe4b}
      \strng{shortauthornamehash}{9873a6cc65c553faa2b21aaad626fe4b}
      \strng{shortauthorfullhash}{9873a6cc65c553faa2b21aaad626fe4b}
      \strng{shortauthorfullhashraw}{9873a6cc65c553faa2b21aaad626fe4b}
      \field{labelalpha}{XAn35}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{singletitle}
      \true{uniquework}
      \field{labelnamesource}{shortauthor}
      \field{labeltitlesource}{shorttitle}
      \field{langid}{english}
      \field{langidopts}{variant=american}
      \field{note}{anon1}
      \field{shorttitle}{Shorttitle}
      \field{title}{Title1}
      \field{year}{1835}
      \field{pages}{111\bibrangedash 118}
      \range{pages}{8}
      \keyw{arc}
    \endentry
"#;

const EXPECTED_ANON2: &str = r#"    \entry{anon2}{unpublished}{}{}
      \name{author}{1}{}{%
        {{hash=a0bccee4041bc840e14c06e5ba7f083c}{%
           family={AnonymousY},
           familyi={A\bibinitperiod}}}%
      }
      \name{shortauthor}{1}{}{%
        {{un=0,uniquepart=base,hash=f64c29e89ea49402b997956610b58ef6}{%
           family={YAnony},
           familyi={Y\bibinitperiod}}}%
      }
      \strng{namehash}{f64c29e89ea49402b997956610b58ef6}
      \strng{fullhash}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{fullhashraw}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{bibnamehash}{f64c29e89ea49402b997956610b58ef6}
      \strng{authorbibnamehash}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{authornamehash}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{authorfullhash}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{authorfullhashraw}{a0bccee4041bc840e14c06e5ba7f083c}
      \strng{shortauthorbibnamehash}{f64c29e89ea49402b997956610b58ef6}
      \strng{shortauthornamehash}{f64c29e89ea49402b997956610b58ef6}
      \strng{shortauthorfullhash}{f64c29e89ea49402b997956610b58ef6}
      \strng{shortauthorfullhashraw}{f64c29e89ea49402b997956610b58ef6}
      \field{labelalpha}{YAn39}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{singletitle}
      \true{uniquework}
      \field{labelnamesource}{shortauthor}
      \field{labeltitlesource}{shorttitle}
      \field{langid}{english}
      \field{langidopts}{variant=american}
      \field{note}{anon2}
      \field{shorttitle}{Shorttitle}
      \field{title}{Title2}
      \field{year}{1839}
      \field{pages}{1176\bibrangedash 1276}
      \range{pages}{101}
      \keyw{arc}
    \endentry
"#;

const EXPECTED_URL1: &str = r#"    \entry{url1}{misc}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=b2106a3dda6c5a4879a0cab37e9cca55}{%
           family={Alias},
           familyi={A\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{fullhash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{fullhashraw}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{bibnamehash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{authorbibnamehash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{authornamehash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{authorfullhash}{b2106a3dda6c5a4879a0cab37e9cca55}
      \strng{authorfullhashraw}{b2106a3dda6c5a4879a0cab37e9cca55}
      \field{extraname}{4}
      \field{labelalpha}{Ali05}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{extraalpha}{4}
      \field{labelnamesource}{author}
      \field{year}{2005}
      \field{dateera}{ce}
      \verb{urlraw}
      \verb http://www.something.com/q=áŠ
      \endverb
      \verb{url}
      \verb http://www.something.com/q=%C3%A1%C5%A0
      \endverb
      \lverb{urlsraw}{2}
      \lverb http://www.something.com/q=áŠ
      \lverb http://www.sun.com
      \endlverb
      \lverb{urls}{2}
      \lverb http://www.something.com/q=%C3%A1%C5%A0
      \lverb http://www.sun.com
      \endlverb
    \endentry
"#;

const EXPECTED_LIST1: &str = r#"    \entry{list1}{book}{}{}
      \true{morelocation}
      \list{location}{2}{%
        {ÁAA}%
        {BBB}%
      }
      \field{sortinit}{}
      \field{sortinithash}{495dc9894017a8b12cafa9c619d10c0c}
    \endentry
"#;

const EXPECTED_OVER1: &str = r#"    \entry{over1}{book}{}{}
      \field{sortinit}{}
      \field{sortinithash}{495dc9894017a8b12cafa9c619d10c0c}
      \field{userd}{thing}
    \endentry
"#;

const EXPECTED_ISBN1: &str = r#"    \entry{isbn1}{misc}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=f6595ccb9db5f634e7bb242a3f78e5f9}{%
           family={Flummox},
           familyi={F\bibinitperiod},
           given={Fred},
           giveni={F\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{fullhash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{fullhashraw}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{bibnamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorbibnamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authornamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorfullhash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorfullhashraw}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \field{extraname}{1}
      \field{labelalpha}{Flu}
      \field{sortinit}{F}
      \field{sortinithash}{2638baaa20439f1b5a8f80c6c08a13b4}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{isbn}{978-0-8165-2066-4}
    \endentry
"#;

const EXPECTED_ISBN2: &str = r#"    \entry{isbn2}{misc}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=f6595ccb9db5f634e7bb242a3f78e5f9}{%
           family={Flummox},
           familyi={F\bibinitperiod},
           given={Fred},
           giveni={F\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{fullhash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{fullhashraw}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{bibnamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorbibnamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authornamehash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorfullhash}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \strng{authorfullhashraw}{f6595ccb9db5f634e7bb242a3f78e5f9}
      \field{extraname}{2}
      \field{labelalpha}{Flu}
      \field{sortinit}{F}
      \field{sortinithash}{2638baaa20439f1b5a8f80c6c08a13b4}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
      \field{isbn}{978-0-8165-2066-4}
    \endentry
"#;

const EXPECTED_NEW1: &str = r#"    \entry{newtestkey}{book}{}{}
      \field{sortinit}{}
      \field{sortinithash}{495dc9894017a8b12cafa9c619d10c0c}
      \field{note}{note}
      \field{usera}{RC-6947}
      \field{userb}{RC}
    \endentry
"#;

const EXPECTED_CLONE1: &str = r#"    \entry{snk1}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=83330b0520b5d4ea57529a23b404d43d}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           prefix={von},
           prefixi={v\bibinitperiod},
           suffix={Jr},
           suffixi={J\bibinitperiod},
           suffixun=0}}%
      }
      \strng{namehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{fullhash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{fullhashraw}{83330b0520b5d4ea57529a23b404d43d}
      \strng{bibnamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorbibnamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authornamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorfullhash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorfullhashraw}{83330b0520b5d4ea57529a23b404d43d}
      \field{extraname}{2}
      \field{labelalpha}{vDoe}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
    \endentry
"#;

const EXPECTED_CLONE2: &str = r#"    \entry{clone-snk1}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=83330b0520b5d4ea57529a23b404d43d}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           prefix={von},
           prefixi={v\bibinitperiod},
           suffix={Jr},
           suffixi={J\bibinitperiod},
           suffixun=0}}%
      }
      \strng{namehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{fullhash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{fullhashraw}{83330b0520b5d4ea57529a23b404d43d}
      \strng{bibnamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorbibnamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authornamehash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorfullhash}{83330b0520b5d4ea57529a23b404d43d}
      \strng{authorfullhashraw}{83330b0520b5d4ea57529a23b404d43d}
      \field{extraname}{1}
      \field{labelalpha}{vDoe}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{addendum}{add}
    \endentry
"#;

const EXPECTED_ENT1: &str = r#"    \entry{ent1}{book}{}{}
      \name{author}{2}{sortingnamekeytemplatename=snks1}{%
        {{un=0,uniquepart=base,hash=6b3653417f9aa97391c37cff5dfda7fa}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Simon},
           giveni={S\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,sortingnamekeytemplatename=snks2,hash=878a51e6f69e95562d15cb8a3ead5c95}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod},
           givenun=0,
           prefix={de},
           prefixi={d\bibinitperiod}}}%
      }
      \strng{namehash}{b2536a425d549b46de5f21c4d468050a}
      \strng{fullhash}{b2536a425d549b46de5f21c4d468050a}
      \strng{fullhashraw}{b2536a425d549b46de5f21c4d468050a}
      \strng{bibnamehash}{b2536a425d549b46de5f21c4d468050a}
      \strng{authorbibnamehash}{b2536a425d549b46de5f21c4d468050a}
      \strng{authornamehash}{b2536a425d549b46de5f21c4d468050a}
      \strng{authorfullhash}{b2536a425d549b46de5f21c4d468050a}
      \strng{authorfullhashraw}{b2536a425d549b46de5f21c4d468050a}
      \field{labelalpha}{SdB}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{singletitle}
      \field{labelnamesource}{author}
    \endentry
"#;

const EXPECTED_VERB1: &str = r#"    \entry{verb1}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=cac5a25f503e71f5ef28f474e14007b6}{%
           family={Allright},
           familyi={A\\bibinitperiod},
           given={Arthur},
           giveni={A\\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{fullhash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{fullhashraw}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{bibnamehash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{authorbibnamehash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{authornamehash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{authorfullhash}{cac5a25f503e71f5ef28f474e14007b6}
      \strng{authorfullhashraw}{cac5a25f503e71f5ef28f474e14007b6}
      \field{labelalpha}{All}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{singletitle}
      \field{labelnamesource}{author}
      \verb{verba}
      \verb \=y.\"a
      \endverb
    \endentry
"#;

const EXPECTED_CITEDKEYS: &[&str] = &[
    "alias1", "alias2", "alias5", "anon1", "anon2", "ent1", "isbn1", "isbn2", "kant:kpv",
    "kant:ku", "list1", "markey", "matches1", "matches2", "matches3", "murray", "over1",
    "recurse1", "shore", "t1", "t2", "u1", "u2", "us1", "verb1",
];

const EXPECTED_ALLKEYS: &[&str] = &[
    "alias1",
    "alias2",
    "alias5",
    "almendro",
    "angenendt",
    "angenendtsa",
    "angenendtsk",
    "anon1",
    "anon2",
    "aristotle:anima",
    "aristotle:physics",
    "aristotle:poetics",
    "aristotle:rhetoric",
    "augustine",
    "averroes/bland",
    "averroes/hannes",
    "averroes/hercz",
    "avona",
    "baez/article",
    "baez/online",
    "bertram",
    "brandt",
    "britannica",
    "chiu",
    "cicero",
    "clone-snk1",
    "cms",
    "coleridge",
    "companion",
    "cotton",
    "ctan",
    "ent1",
    "final",
    "gaonkar",
    "geer",
    "gerhardt",
    "gillies",
    "gonzalez",
    "hammond",
    "hasan",
    "hyman",
    "i1",
    "i2",
    "iliad",
    "isbn1",
    "isbn2",
    "itzhaki",
    "jaffe",
    "kant:kpv",
    "kant:ku",
    "kastenholz",
    "knuth:ct",
    "knuth:ct:a",
    "knuth:ct:b",
    "knuth:ct:c",
    "knuth:ct:d",
    "knuth:ct:e",
    "kowalik",
    "labelstest",
    "laufenberg",
    "list1",
    "lne1",
    "luzzatto",
    "malinowski",
    "markey",
    "maron",
    "massa",
    "matches1",
    "matches2",
    "matches3",
    "moraux",
    "murray",
    "newtestkey",
    "nietzsche:historie",
    "nietzsche:ksa",
    "nietzsche:ksa1",
    "nussbaum",
    "ol1",
    "others1",
    "others2",
    "over1",
    "padhye",
    "pages1",
    "pages2",
    "pages3",
    "pages4",
    "pages5",
    "pages6",
    "pages7",
    "pages8",
    "pages9",
    "piccato",
    "pimentel00",
    "pines",
    "recurse1",
    "reese",
    "rvonr",
    "set",
    "set:aksin",
    "set:herrmann",
    "set:yoon",
    "shore",
    "sigfridsson",
    "sn1",
    "snk1",
    "sorace",
    "spiegelberg",
    "springer",
    "stdmodel",
    "stdmodel:glashow",
    "stdmodel:ps_sc",
    "stdmodel:salam",
    "stdmodel:weinberg",
    "t1",
    "t2",
    "tmn1",
    "tmn2",
    "tmn3",
    "tmn4",
    "tvonb",
    "u1",
    "u2",
    "url1",
    "us1",
    "vangennep",
    "vangennepx",
    "vazques-de-parga",
    "verb1",
    "westfahl:frontier",
    "westfahl:space",
    "worman",
    "wormanx",
];

fn field_string(entry: &Entry, field: &str) -> Option<String> {
    if field == "entrytype" {
        return Some(entry.entry_type().as_str().to_owned());
    }
    match entry.fields().get(&FieldId::new(field).unwrap())? {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::Boolean(value) => Some(value.to_string()),
        FieldValue::UriList(values) => values.first().map(|value| value.as_str().to_owned()),
        _ => None,
    }
}
fn list_values<'a>(entry: &'a Entry, field: &str) -> Vec<&'a str> {
    match entry.fields().get(&FieldId::new(field).unwrap()) {
        Some(FieldValue::LiteralList(values)) => {
            values.iter().map(|value| value.as_str()).collect()
        }
        _ => Vec::new(),
    }
}
fn name_count(entry: &Entry) -> usize {
    entry
        .fields()
        .iter()
        .find_map(|field| match field.value() {
            FieldValue::NameList(names) => Some(names.len()),
            _ => None,
        })
        .unwrap_or_default()
}
fn uniquename_projection(entry: &Entry) -> Vec<String> {
    entry
        .fields()
        .iter()
        .find_map(|field| match field.value() {
            FieldValue::NameList(names) => Some(
                names
                    .iter()
                    .map(|name| {
                        let mut key = String::new();
                        if let Some(prefix) = name.prefix() {
                            key.push_str(prefix.value().as_str());
                        }
                        if let Some(family) = name.family() {
                            key.push_str(family.value().as_str());
                        }
                        key.push('\u{10fffd}');
                        key.push_str(name.hash_id().unwrap_or_default());
                        key
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}
fn ranges(entry: &Entry) -> Vec<(String, Option<String>)> {
    fn endpoint(value: &RangeEndpoint) -> Option<String> {
        match value {
            RangeEndpoint::Integer(value) => Some(value.to_string()),
            RangeEndpoint::Literal(value) => Some(value.as_str().to_owned()),
            RangeEndpoint::Open => None,
        }
    }
    match entry.fields().get(&FieldId::new("pages").unwrap()) {
        Some(FieldValue::RangeList(values)) => values
            .iter()
            .map(|value| {
                (
                    endpoint(value.start()).unwrap_or_default(),
                    endpoint(value.end()),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}
fn alias<'a>(result: &'a bib_engine::BibResult, key: &str) -> Option<&'a str> {
    result
        .document()
        .section(SectionId::new(0))?
        .aliases()
        .find(|(alias, _)| alias.as_str() == key)
        .map(|(_, target)| target.as_str())
}
#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_001_uniquelist_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "u1"))
            .as_deref(),
        Some(EXPECTED_U1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_002_citekeys_1() {
    let result = try_run_fixture("basic-misc");
    let mut keys = result
        .as_ref()
        .ok()
        .map(|result| {
            section_entry_keys(result, 0)
                .into_iter()
                .filter(|key| !key.starts_with("loopkey"))
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    keys.sort_unstable();
    assert_eq!(keys, EXPECTED_CITEDKEYS);
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_003_shorthands() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| list_keys(result, 0, "shorthand/global//global/global/global"))
            .unwrap_or_default(),
        ["kant:kpv", "kant:ku"]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_004_citekeys_2() {
    let result = try_run_fixture("basic-misc");
    let mut keys = result
        .as_ref()
        .ok()
        .map(|result| {
            section_entry_keys(result, 0)
                .into_iter()
                .filter(|key| !key.starts_with("loopkey"))
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    keys.sort_unstable();
    assert_eq!(keys, EXPECTED_ALLKEYS);
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_005_bbl_entry_with_maths_in_title_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "t1"))
            .as_deref(),
        Some(EXPECTED_T1)
    );
}

#[test]
fn assertion_006_default_bib_month_macros() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "shore"))
            .and_then(|entry| field_string(entry, "month"))
            .as_deref(),
        Some(r#"3"#)
    );
}

#[test]
fn assertion_007_keywords_test_1() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "t1"))
            .is_some_and(|entry| list_values(entry, "keywords").contains(&"primary"))
    );
}

#[test]
fn assertion_008_keywords_test_2() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "t1"))
            .is_some_and(|entry| list_values(entry, "keywords").contains(&"something"))
    );
}

#[test]
fn assertion_009_keywords_test_3() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "t1"))
            .is_some_and(|entry| list_values(entry, "keywords").contains(&"somethingelse"))
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_010_bbl_entry_with_maths_in_title_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "t2"))
            .as_deref(),
        Some(EXPECTED_T2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_011_uniquename_count_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "WormanN"))
            .map(uniquename_projection)
            .unwrap_or_default(),
        ["Worman􏿽WormanN"]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_012_uniquename_count_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "vanGennep"))
            .map(uniquename_projection)
            .unwrap_or_default(),
        ["vanGennep􏿽vanGennepA", "vanGennep􏿽vanGennepJ"]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_013_bbl_with_maxcitenames() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "murray"))
            .as_deref(),
        Some(EXPECTED_MURRAY1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_014_missing_citekey_1() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| output_text(result).contains("  \\missing{missing1}\n"))
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_015_missing_citekey_2() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| output_text(result).contains("  \\missing{missing2}\n"))
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_016_bbl_with_maxcitenames_empty_alphaothers() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "murray"))
            .as_deref(),
        Some(EXPECTED_MURRAY2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_017_namehash_fullhash_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "anon1"))
            .as_deref(),
        Some(EXPECTED_ANON1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_018_namehash_fullhash_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "anon2"))
            .as_deref(),
        Some(EXPECTED_ANON2)
    );
}

#[test]
fn assertion_019_map_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .and_then(|entry| field_string(entry, "abstract"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_020_map_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .and_then(|entry| field_string(entry, "userd"))
            .as_deref(),
        Some(r#"test"#)
    );
}

#[test]
fn assertion_021_map_3() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i2"))
            .and_then(|entry| field_string(entry, "userb"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_022_map_4() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i2"))
            .and_then(|entry| field_string(entry, "usere"))
            .as_deref(),
        Some(r#"a Štring"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_023_map_5() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .map(|entry| list_values(entry, "listd").join("!"))
            .as_deref(),
        Some("abc")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_024_map_6() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .map(|entry| list_values(entry, "listb").join("!"))
            .as_deref(),
        Some("REPlacedte!early")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_025_map_7() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .map(|entry| list_values(entry, "institution").join("!"))
            .as_deref(),
        Some("REPlaCEDte!early")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_026_map_8() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i1"))
            .and_then(|entry| field_string(entry, "note"))
            .as_deref(),
        Some(r#"i1"#)
    );
}

#[test]
fn assertion_027_map_9() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i2"))
            .and_then(|entry| field_string(entry, "userf"))
            .as_deref(),
        None
    );
}

#[test]
fn assertion_028_map_10() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i2"))
            .and_then(|entry| field_string(entry, "userc"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_029_bib_visibility_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "i2"))
            .map(name_count),
        Some(3)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_030_per_type_maxcitenames_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn1"))
            .map(name_count),
        Some(1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_031_per_type_maxcitenames_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn2"))
            .map(name_count),
        Some(3)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_032_per_type_bibnames_3() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn3"))
            .map(name_count),
        Some(2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_033_per_type_bibnames_4() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn4"))
            .map(name_count),
        Some(3)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_034_per_type_entry_alphanames_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn1"))
            .map(name_count),
        Some(3)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_035_per_type_entry_alphanames_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn2"))
            .map(name_count),
        Some(2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_036_per_type_entry_items_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn1"))
            .map(|entry| list_values(entry, "institution").join("!"))
            .as_deref(),
        Some("A!B!C")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_037_per_type_entry_items_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "tmn3"))
            .map(|entry| list_values(entry, "institution").join("!"))
            .as_deref(),
        Some("A!B􏿽")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_038_citekey_aliases_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| alias(result, "alias3")),
        Some("alias1")
    );
}

#[test]
fn assertion_039_citekey_aliases_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| alias(result, "alias2")),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_040_citekey_aliases_3() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| alias(result, "alias4")),
        Some("alias2")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_041_citekey_aliases_4() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| alias(result, "alias6")),
        Some("alias5")
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_042_citekey_aliases_5() {
    let result = try_run_fixture("basic-misc");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "alias5"))
            .is_some()
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_043_url_encoding_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "url1"))
            .and_then(|entry| field_string(entry, "url"))
            .as_deref(),
        Some(r#"http://www.something.com/q=áŠ"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_044_url_encoding_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "url1"))
            .as_deref(),
        Some(EXPECTED_URL1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_045_map_final_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "ol1"))
            .and_then(|entry| field_string(entry, "note"))
            .as_deref(),
        Some(r#"A note"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_046_map_final_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "ol1"))
            .and_then(|entry| field_string(entry, "title"))
            .as_deref(),
        Some(r#"Online1"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_047_pages_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages1"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"23"#.to_owned(), Some(r#"24"#.to_owned()))]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_048_pages_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages2"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"23"#.to_owned(), None)]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_049_pages_3() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages3"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"I-II"#.to_owned(), Some(r#"III-IV"#.to_owned()))]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_050_pages_4() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages4"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"3"#.to_owned(), Some(r#"5"#.to_owned()))]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_051_pages_5() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages5"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"42"#.to_owned(), Some(r#""#.to_owned()))]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_052_pages_6() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages6"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"\bibstring{number} 42"#.to_owned(), None)]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_053_pages_7() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages7"))
            .map(ranges)
            .unwrap_or_default(),
        [
            (r#"\bibstring{number} 42"#.to_owned(), None),
            (r#"3"#.to_owned(), Some(r#"6"#.to_owned())),
            (r#"I-II"#.to_owned(), Some(r#"5"#.to_owned()))
        ]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_054_pages_8() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages8"))
            .map(ranges)
            .unwrap_or_default(),
        [
            (r#"10"#.to_owned(), Some(r#"15"#.to_owned())),
            (r#"ⅥⅠ"#.to_owned(), Some(r#"ⅻ"#.to_owned()))
        ]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_055_pages_9() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "pages9"))
            .map(ranges)
            .unwrap_or_default(),
        [(r#"M-1"#.to_owned(), Some(r#"M-4"#.to_owned()))]
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_056_map_levels_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "us1"))
            .and_then(|entry| field_string(entry, "entrytype"))
            .as_deref(),
        Some(r#"customa"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_057_entry_with_others_list() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "list1"))
            .as_deref(),
        Some(EXPECTED_LIST1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_058_overwrite_test_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "over1"))
            .as_deref(),
        Some(EXPECTED_OVER1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_059_isbn_options_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "isbn1"))
            .as_deref(),
        Some(EXPECTED_ISBN1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_060_isbn_options_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "isbn2"))
            .as_deref(),
        Some(EXPECTED_ISBN2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_061_clone_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "snk1"))
            .as_deref(),
        Some(EXPECTED_CLONE1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_062_clone_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "clone-snk1"))
            .as_deref(),
        Some(EXPECTED_CLONE2)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_063_new_key_mapping_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "newtestkey"))
            .as_deref(),
        Some(EXPECTED_NEW1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_064_new_key_loop_mapping_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| section_entry_keys(result, 0)
                .into_iter()
                .filter(|key| key.starts_with("loopkey:"))
                .count()),
        Some(3)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_065_new_key_loop_mapping_2() {
    let result = try_run_fixture("basic-misc");
    let note = result
        .as_ref()
        .ok()
        .and_then(|result| result.document().section(SectionId::new(0)))
        .and_then(|section| {
            section
                .entries()
                .find(|entry| entry.id().as_str().starts_with("loopkey:"))
        })
        .and_then(|entry| field_string(entry, "note"));
    assert_eq!(note.as_deref(), Some("NOTEreplaced"));
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_066_notfield_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "markey"))
            .and_then(|entry| field_string(entry, "addendum"))
            .as_deref(),
        Some(r#"NF1"#)
    );
}

#[test]
fn assertion_067_notfield_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "markey"))
            .and_then(|entry| field_string(entry, "userb"))
            .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_068_extended_name_test_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "ent1"))
            .as_deref(),
        Some(EXPECTED_ENT1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_069_decoding_verbatim_fields_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "verb1"))
            .as_deref(),
        Some(EXPECTED_VERB1)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_070_static_match_list_1() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "matches1"))
            .and_then(|entry| field_string(entry, "note"))
            .as_deref(),
        Some(r#"1"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_071_static_match_list_2() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "matches2"))
            .and_then(|entry| field_string(entry, "note"))
            .as_deref(),
        Some(r#"3"#)
    );
}

#[test]
#[ignore = "xfail: exact Biber mixed-stage behavior is not implemented by bib-engine"]
fn assertion_072_static_match_list_3() {
    let result = try_run_fixture("basic-misc");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "matches3"))
            .and_then(|entry| field_string(entry, "note"))
            .as_deref(),
        Some(r#"2"#)
    );
}
