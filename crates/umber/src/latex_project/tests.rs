use bib_engine::{BibOptionsBuilder, OutputFormat, OutputRequest};

use super::*;
use crate::EngineMode;

fn options() -> LatexProjectOptions {
    let control = VirtualPath::user("/job/main.bcf").expect("control path");
    let mut bib = BibOptionsBuilder::new();
    bib.output(OutputRequest::new(
        VirtualPath::user("/job/main.bbl").expect("output path"),
        OutputFormat::Bbl,
    ))
    .expect("output");
    LatexProjectOptions {
        tex: SessionOptions {
            engine: EngineMode::Tex82,
            ..SessionOptions::default()
        },
        bibliography: BibJob::new(control, bib.freeze()),
        bib_session: BibSessionOptions::default(),
        limits: LatexProjectLimits::default(),
    }
}

#[test]
fn converges_and_atomically_publishes_tex_and_bibliography_files() {
    let source = br#"\immediate\openout1=main.bcf
\immediate\write1{<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex"><bcf:section number="0"></bcf:section></bcf:controlfile>}
\immediate\closeout1
\shipout\hbox{X}\end
"#;
    let mut session = LatexProjectSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", source.to_vec())
        .expect("source");
    let output = match session.compile_attempt() {
        LatexProjectAttempt::Complete(output) => output,
        attempt => panic!("expected complete project, got {attempt:?}"),
    };
    assert!(output.passes >= 2);
    assert!(!output.tex.dvi.is_empty());
    assert!(
        output
            .generated_files
            .iter()
            .any(|file| file.path == std::path::Path::new("/job/main.bcf"))
    );
    assert!(
        output
            .generated_files
            .iter()
            .any(|file| file.path == std::path::Path::new("/job/main.bbl"))
    );
    assert_eq!(
        session.compile_attempt(),
        LatexProjectAttempt::Complete(output.clone())
    );

    let mut cold = LatexProjectSession::new(options()).expect("cold session");
    cold.add_user_file("/job/main.tex", source.to_vec())
        .expect("cold source");
    assert_eq!(
        cold.compile_attempt(),
        LatexProjectAttempt::Complete(output),
        "cold and retained project runs must agree byte-for-byte"
    );
}

#[test]
fn failed_patch_preserves_the_accepted_project() {
    let source =
        b"\\immediate\\openout1=state.aux \\immediate\\write1{old} \\immediate\\closeout1 \\end\n"
            .to_vec();
    let mut project_options = options();
    project_options.tex.limits.output_bytes = 512;
    let mut session = LatexProjectSession::new(project_options).expect("session");
    session
        .add_user_file("/job/main.tex", source.clone())
        .expect("source");
    let accepted = match session.compile_attempt() {
        LatexProjectAttempt::Complete(output) => output,
        attempt => panic!("expected complete project, got {attempt:?}"),
    };
    let start = source
        .windows(3)
        .position(|window| window == b"old")
        .expect("payload");
    session
        .apply_patch(SourcePatch {
            base_revision: accepted.revision,
            next_revision: tex_incr::RevisionId::new(2),
            expected_hash: accepted.content_hash,
            range: start..start + 3,
            replacement: "x".repeat(1_024),
        })
        .expect("patch");
    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttempt::Error(_)
    ));
    assert_eq!(session.accepted_output(), Some(&accepted));
    assert_eq!(session.revision(), Some(tex_incr::RevisionId::new(1)));
}

#[test]
fn repeated_resource_need_is_a_typed_no_progress_failure() {
    let mut session = LatexProjectSession::new(options()).expect("session");
    session
        .add_user_file("/job/main.tex", b"\\input remote \\end\n".to_vec())
        .expect("source");
    assert!(matches!(
        session.compile_attempt(),
        LatexProjectAttempt::NeedResources(_)
    ));
    assert_eq!(
        session.compile_attempt(),
        LatexProjectAttempt::Error(LatexProjectError::Compile(CompileError::NoProgress))
    );
    assert!(session.accepted_output().is_none());
}
