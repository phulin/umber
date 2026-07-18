// Native Rust translation of upstream t/remote-files.t at commit 74252e6.

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, FileProvisioner, GeneratedFile,
    OutputFormat, OutputRequest, VfsLimits, VirtualPath,
};

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/remote-files.bcf");
const URL: &str =
    "https://raw.githubusercontent.com/twschiller/public-bib/refs/heads/master/schiller.bib";
const DL1: &str = r########"    \entry{SchillerCND2010}{article}{}{}
      \name{author}{4}{}{%
        {{un=0,uniquepart=base,hash=c606849f9ce94faa18c52562c39b6f92}{%
           family={Schiller},
           familyi={S\bibinitperiod},
           given={Todd\bibnamedelima W.},
           giveni={T\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=20154ea5f879b7b3256febae2ee215b6}{%
           family={Chen},
           familyi={C\bibinitperiod},
           given={Yixin},
           giveni={Y\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=47c2d9d369b1f19efa50d61551e7a69b}{%
           family={El\bibnamedelima Naqa},
           familyi={E\bibinitperiod\bibinitdelim N\bibinitperiod},
           given={Issam},
           giveni={I\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=f967f6b51246cf632661681073e1b6d8}{%
           family={Deasy},
           familyi={D\bibinitperiod},
           given={Joseph\bibnamedelima O.},
           giveni={J\bibinitperiod\bibinitdelim O\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{fullhash}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{fullhashraw}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{bibnamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authorbibnamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authornamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authorfullhash}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{authorfullhashraw}{104a7a1c8b6937b5c35dc05b94f446b9}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{journaltitle}{Neurocomputing}
      \field{month}{6}
      \field{number}{10-12}
      \field{title}{Modeling Radiation-induced Lung Injury Risk with an Ensemble of Support Vector Machines}
      \field{volume}{73}
      \field{year}{2010}
      \verb{doi}
      \verb 10.1016/j.neucom.2009.09.023
      \endverb
    \endentry
"########;

fn run() -> (String, Vec<u8>) {
    let mut files = FileProvisioner::new(VfsLimits::default()).unwrap();
    files
        .register_user(
            VirtualPath::user("remote-files.bcf").unwrap(),
            CONTROL.to_vec(),
        )
        .unwrap();
    let output_path = VirtualPath::user("remote-files.bbl").unwrap();
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output_path, OutputFormat::Bbl))
        .unwrap();
    let job = BibJob::new(
        VirtualPath::user("remote-files.bcf").unwrap(),
        options.freeze(),
    );
    match BibSession::default().process(&job, &files.snapshot()) {
        BibAttempt::NeedResources(requests) => {
            assert_eq!(requests.required.len(), 1);
            (requests.required[0].original_name().to_owned(), Vec::new())
        }
        BibAttempt::Complete(result) => {
            let bytes = result
                .files()
                .next()
                .map(GeneratedFile::bytes)
                .unwrap_or_default()
                .to_vec();
            (String::new(), bytes)
        }
        BibAttempt::Failed(failure) => panic!("remote control processing failed: {failure:?}"),
    }
}

#[test]
#[ignore = "xfail: host-neutral engine requests remote bytes instead of fetching them"]
fn assertion_001_fetch_from_plain_bib_download() {
    let (requested, output) = run();
    assert_eq!(requested, URL);
    assert!(
        output
            .windows(DL1.len())
            .any(|window| window == DL1.as_bytes())
    );
}
