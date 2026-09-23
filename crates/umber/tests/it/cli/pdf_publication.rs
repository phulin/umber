//! PDF mode selection, lowering, and atomic output publication.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_rule_page_is_published_only_to_an_explicit_distinct_pdf_path() {
    let temp_dir = tempfile::tempdir().expect("create PDF output temp dir");
    let source = temp_dir.path().join("rule.tex");
    let pdf = temp_dir.path().join("rule.pdf");
    let dvi = temp_dir.path().join("rule.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write PDF rule fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run pdfTeX PDF fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read published PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
    assert!(fs::metadata(&dvi).expect("published DVI").len() > 0);
    let telemetry = String::from_utf8_lossy(&output.stderr);
    for marker in [
        "RESOURCE_STARTUP_TELEMETRY",
        "RESOURCE_ENGINE_ACCEPTED",
        "RESOURCE_HOST_TELEMETRY",
        "PDF_TELEMETRY",
        "PDF_DRIVER_BUILD",
        "PDF_DRIVER_TELEMETRY",
    ] {
        assert!(telemetry.contains(marker), "missing {marker}:\n{telemetry}");
    }

    let rejected = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdf")
        .arg(temp_dir.path().join("wrong-mode.pdf"))
        .arg(&source)
        .output()
        .expect("reject PDF without pdfTeX mode");
    assert!(!rejected.status.success());
    assert_eq!(
        String::from_utf8(rejected.stderr).expect("stderr is utf-8"),
        "umber: --pdf requires --pdftex or --pdflatex\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_cli_keeps_deferred_effect_pages_in_the_published_page_tree() {
    let temp_dir = tempfile::tempdir().expect("create prepared PDF output temp dir");
    let source = temp_dir.path().join("prepared-pages.tex");
    let pdf = temp_dir.path().join("prepared-pages.pdf");
    fs::write(
        &source,
        concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}",
            "\\shipout\\vbox{\\openout0=side-effect.txt",
            "\\write0{page-two}\\hrule width2pt height2pt}\\end\n",
        ),
    )
    .expect("write prepared PDF fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(temp_dir.path())
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run prepared PDF fixture");
    assert!(
        output.status.success(),
        "prepared PDF run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(temp_dir.path().join("side-effect.txt"))
            .expect("deferred page effect committed"),
        "page-two\n"
    );
    let pdf = fs::read(pdf).expect("read prepared PDF");
    let parsed = test_support::pdf_query::PdfQuery::new(
        &pdf,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent parser accepts prepared PDF");
    assert_eq!(parsed.pages().expect("prepared page tree").len(), 2);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn pdflatex_mode_composes_latex_compatibility_with_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create pdfLaTeX output temp dir");
    let source = temp_dir.path().join("composed.tex");
    let pdf = temp_dir.path().join("composed.pdf");
    fs::write(
        &source,
        "\\catcode123=1\\catcode125=2\\pdfoutput=1\\ifnum\\strcmp{same}{same}=0\\shipout\\vbox{\\hrule width10pt height5pt}\\fi\\end\n",
    )
    .expect("write composed pdfLaTeX fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdflatex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run composed pdfLaTeX fixture");

    assert!(
        output.status.success(),
        "pdfLaTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read composed pdfLaTeX PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdfdraftmode_does_not_replace_the_requested_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create draft-mode output temp dir");
    let source = temp_dir.path().join("draft.tex");
    let pdf = temp_dir.path().join("draft.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfdraftmode=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write draft-mode fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run draft-mode fixture");

    assert!(
        output.status.success(),
        "draft-mode run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is utf-8"),
        "pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read unchanged output"),
        b"existing output\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn fatal_pdf_finalization_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create fatal-finalization output temp dir");
    let source = temp_dir.path().join("fatal-finalization.tex");
    let pdf = temp_dir.path().join("fatal-finalization.pdf");
    let closure = temp_dir.path().join("fatal-finalization.font-closure");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfobj reserveobjnum\\pdfrefobj 1\\end\n",
    )
    .expect("write fatal-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg("--pdf-font-closure-out")
            .arg(&closure)
            .arg(&source)
            .output()
            .expect("run fatal-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: referenced PDF object 1 was reserved but never initialized\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal detached finalization must publish no partial artifact"
    );
    assert_eq!(
        fs::read(&closure).expect("read accepted font closure"),
        b"umber-pdf-font-closure-v1\n",
        "accepted resource evidence survives a later detached-driver failure"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn unfinished_pdf_thread_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create thread-finalization output temp dir");
    let source = temp_dir.path().join("thread-finalization.tex");
    let pdf = temp_dir.path().join("thread-finalization.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\pdfstartthread name{open}}\\end\n",
    )
    .expect("write thread-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run thread-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: page 1 ends with PDF thread object 1 still running\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal thread finalization must not replace the requested PDF"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn unpublished_default_distribution_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create missing-image output temp dir");
    let source = temp_dir.path().join("missing-image.tex");
    let pdf = temp_dir.path().join("missing-image.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfximage{missing.png}\\shipout\\hbox{\\pdfrefximage1}\\end\n",
    )
    .expect("write missing-image fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run missing-image fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        concat!(
            "umber: the default deterministic aHash64 distribution has not been published; ",
            "pass --distribution and --distribution-ahash64 for a migrated local or hosted root\n",
        )
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "failed delayed acquisition must publish no replacement artifact"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn fatal_annotation_action_finalization_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create annotation-finalization output temp dir");
    let source = temp_dir.path().join("annotation-finalization.tex");
    let pdf = temp_dir.path().join("annotation-finalization.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcatalog{} openaction goto page 2 {/Fit}\\shipout\\hbox{}\\end\n",
    )
    .expect("write annotation-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run annotation-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: PDF open action references missing page 2\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal detached finalization must publish no partial annotation artifact"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdf_lowering_omits_dvi_special_and_publishes_all_driver_output() {
    let temp_dir = tempfile::tempdir().expect("create DVI-special temp dir");
    let source = temp_dir.path().join("text.tex");
    let pdf = temp_dir.path().join("text.pdf");
    let dvi = temp_dir.path().join("text.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\special{dvi-only-payload}}\\end\n",
    )
    .expect("write DVI-special fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run DVI-special PDF fixture");

    assert!(
        output.status.success(),
        "DVI-special PDF run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RESOURCE_ENGINE_ACCEPTED"),
        "accepted-engine telemetry must precede detached finalization"
    );
    let pdf_bytes = fs::read(&pdf).expect("PDF output was published");
    assert!(
        !pdf_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI-only special leaked into PDF output"
    );
    let dvi_bytes = fs::read(&dvi).expect("DVI peer output was published");
    assert!(
        dvi_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI peer output lost its special payload"
    );
}
