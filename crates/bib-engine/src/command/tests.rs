use super::*;
use crate::{FileProvisioner, VfsLimits};

const CONTROL: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<bcf:controlfile xmlns:bcf="https://sourceforge.net/projects/biblatex" version="3.11" bltxversion="3.21">
  <bcf:bibdata section="0"><bcf:datasource type="file" datatype="bibtex">refs.bib</bcf:datasource></bcf:bibdata>
  <bcf:section number="0"><bcf:citekey>*</bcf:citekey></bcf:section>
</bcf:controlfile>"#;
const DATA: &[u8] = b"@book{key, author={Ada Lovelace}, title={Notes}, year={1843}}\n";

#[test]
fn exact_invocation_defaults_and_validation() {
    let command = BibCommand::parse(["paper.bcf"]).expect("default command");
    assert_eq!(command.mode(), BibCommandMode::Process);
    let output = command.job().options().outputs().next().expect("output");
    assert_eq!(output.path().as_str(), "/job/paper.bbl");
    assert_eq!(output.format(), OutputFormat::Bbl);

    let tool =
        BibCommand::parse(["--tool", "--output-format=dot", "refs.bib"]).expect("tool command");
    assert_eq!(tool.mode(), BibCommandMode::Tool);
    assert_eq!(
        tool.job()
            .options()
            .outputs()
            .next()
            .expect("output")
            .path()
            .as_str(),
        "/job/refs_bibertool.dot"
    );

    let pinned_output = BibCommand::parse([
        "--output-align",
        "-dot-include=section,field,xdata,crossref,xref,related",
        "--output-format=dot",
        "paper.bcf",
    ])
    .expect("pinned output options");
    assert!(
        pinned_output
            .job()
            .options()
            .output_options()
            .bibtex()
            .alignment()
    );
    assert_eq!(
        pinned_output
            .job()
            .options()
            .output_options()
            .dot()
            .include(),
        DotInclude::default()
    );

    let invalid =
        BibCommand::parse(["--output-format=pdf", "paper.bcf"]).expect_err("unknown format");
    assert_eq!(invalid.output().status().code(), 2);
    assert_eq!(
        invalid.output().terminal(),
        b"ERROR - unknown output format `pdf`\n"
    );

    for args in [
        ["--configfile=settings.conf", "--noconf", "paper.bcf"],
        ["--noconf", "--configfile=settings.conf", "paper.bcf"],
    ] {
        assert!(
            BibCommand::parse(args)
                .expect("noconf command")
                .job()
                .options()
                .configuration()
                .is_none(),
            "--noconf has invocation precedence independent of argument order"
        );
    }
}

#[test]
fn exact_success_status_terminal_log_and_output_bytes() {
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(
            VirtualPath::user("paper.bcf").expect("path"),
            CONTROL.to_vec(),
        )
        .expect("control");
    files
        .register_user(VirtualPath::user("refs.bib").expect("path"), DATA.to_vec())
        .expect("data");
    let command = BibCommand::parse(["--nolog", "paper.bcf"]).expect("command");
    let output = command.execute(&files.snapshot());
    assert_eq!(output.status(), BibExitStatus::Success);
    assert_eq!(
        output.terminal(),
        b"INFO - Bibliography complete: 1 section(s), 1 entries, 1 file(s)\n"
    );
    assert!(output.log().is_empty());
    let generated = output
        .result()
        .expect("result")
        .files()
        .next()
        .expect("generated file");
    assert_eq!(generated.path().as_str(), "/job/paper.bbl");
    assert!(
        generated
            .bytes()
            .starts_with(b"% $ biblatex auxiliary file $\n")
    );
}

#[test]
fn exact_missing_resource_failure() {
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(
            VirtualPath::user("paper.bcf").expect("path"),
            CONTROL.to_vec(),
        )
        .expect("control");
    let output = BibCommand::parse(["paper.bcf"])
        .expect("command")
        .execute(&files.snapshot());
    assert_eq!(output.status(), BibExitStatus::OperationalFailure);
    assert_eq!(
        output.terminal(),
        b"ERROR - Missing required resource(s): refs.bib\n"
    );
    assert_eq!(output.log(), output.terminal());
}
