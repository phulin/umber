use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::cohort_transaction::CohortCase;
use crate::layout_migration::{Mode, run_staged_cohort};

const CASES: &[&str] = &[
    "annotations_running",
    "embedded_subset_controls_negative",
    "embedded_subset_omit",
    "embedded_subset_truetype",
    "embedded_subset_type1",
    "embedded_tagged_spacing",
    "embedded_truetype",
    "embedded_type1",
    "external_pdf_page",
    "form_xobjects",
    "minimal_rule",
    "navigation_structures",
    "object_dictionaries",
    "pk_bitmap_300",
    "pk_bitmap_600",
];

pub(crate) fn run_cli(args: Vec<String>) -> Result<()> {
    let mode = match args.as_slice() {
        [flag] if flag == "--plan" => Mode::Plan,
        [flag] if flag == "--apply" => Mode::Apply,
        _ => bail!("--migrate-pdf-layout requires exactly --plan or --apply"),
    };
    let repository = test_support::repository_root();
    let inventories = inventories(&repository)?;
    print_report(&inventories);
    if mode == Mode::Plan {
        return Ok(());
    }

    let candidates = TempDir::new_in(&repository)
        .context("create repository-local PDF migration candidate cohort")?;
    for (case, inventory) in &inventories {
        let root = candidates.path().join(case);
        fs::create_dir(&root)?;
        for (name, bytes) in inventory {
            fs::write(root.join(name), bytes)?;
        }
    }
    let staged_root = candidates
        .path()
        .strip_prefix(&repository)
        .context("candidate cohort escaped repository")?;
    let cases = CASES
        .iter()
        .map(|case| CohortCase {
            staged: staged_root.join(case).to_string_lossy().into_owned(),
            destination: format!("tests/corpus/pdf/{case}"),
            authorities: authorities(&repository, case),
        })
        .collect::<Vec<_>>();
    run_staged_cohort(&repository, &cases, Mode::Plan)?;
    run_staged_cohort(&repository, &cases, Mode::Apply)?;
    Ok(())
}

fn inventories(repository: &Path) -> Result<BTreeMap<String, BTreeMap<String, Vec<u8>>>> {
    let pdf = repository.join("tests/corpus/pdf");
    if CASES.iter().all(|case| pdf.join(case).is_dir()) {
        return CASES
            .iter()
            .map(|case| {
                let root = pdf.join(case);
                let files = fs::read_dir(&root)?
                    .map(|entry| {
                        let entry = entry?;
                        ensure!(
                            entry.file_type()?.is_file(),
                            "non-file in {}",
                            root.display()
                        );
                        let name = entry.file_name().to_string_lossy().into_owned();
                        Ok((name, fs::read(entry.path())?))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?;
                Ok(((*case).to_owned(), files))
            })
            .collect();
    }
    ensure!(
        CASES
            .iter()
            .all(|case| pdf.join(format!("{case}.tex")).is_file()),
        "PDF layout is neither the complete flat cohort nor the complete directory cohort"
    );
    let mut result = BTreeMap::new();
    let cmr10_tfm = fs::read(repository.join("crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"))?;
    let woff2 = fs::read(repository.join("crates/umber-wasm/assets/cmu-serif-500-roman.woff2"))?;
    let truetype = tex_fonts::PdfTrueTypeProgram::from_woff2(&woff2)
        .context("decode committed CMU Serif WOFF2")?
        .bytes()
        .to_vec();
    let pdftexspace_tfm = provisioned_texlive_bytes(repository, "pdftexspace.tfm")?;
    let pdftexspace_pfb = provisioned_texlive_bytes(repository, "pdftexspace.pfb")?;

    for case in CASES {
        let mut files = BTreeMap::new();
        files.insert(
            "source.tex".to_owned(),
            fs::read(pdf.join(format!("{case}.tex")))?,
        );
        let prefix = format!("{case}.expected.");
        for entry in fs::read_dir(&pdf)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(kind) = name.strip_prefix(&prefix) {
                files.insert(format!("expected.{kind}"), fs::read(entry.path())?);
            }
        }
        if is_font_case(case) {
            files.insert("cmr10.tfm".to_owned(), cmr10_tfm.clone());
        }
        match *case {
            "pk_bitmap_300" => {
                files.insert("cmr10.300pk".to_owned(), fs::read(pdf.join("cmr10.300pk"))?);
            }
            "pk_bitmap_600" => {
                files.insert("cmr10.600pk".to_owned(), fs::read(pdf.join("cmr10.600pk"))?);
            }
            "embedded_type1"
            | "embedded_subset_type1"
            | "embedded_subset_omit"
            | "embedded_subset_controls_negative" => {
                files.insert(
                    "cmr10.pfb".to_owned(),
                    fs::read(pdf.join("embedded_type1.pfb"))?,
                );
            }
            "embedded_tagged_spacing" => {
                files.insert(
                    "cmr10.pfb".to_owned(),
                    fs::read(pdf.join("embedded_type1.pfb"))?,
                );
                files.insert(
                    "tagged_spacing.enc".to_owned(),
                    fs::read(pdf.join("tagged_spacing.enc"))?,
                );
                files.insert("customspace.tfm".to_owned(), pdftexspace_tfm.clone());
                files.insert("pdftexspace.pfb".to_owned(), pdftexspace_pfb.clone());
            }
            "embedded_truetype" => {
                files.insert("cmu-serif.ttf".to_owned(), truetype.clone());
            }
            "embedded_subset_truetype" => {
                files.insert("cmu-serif.ttf".to_owned(), truetype.clone());
                files.insert("fixture.enc".to_owned(), fs::read(pdf.join("fixture.enc"))?);
            }
            "external_pdf_page" => {
                files.insert(
                    "minimal_rule.expected.ref.pdf".to_owned(),
                    fs::read(pdf.join("minimal_rule.expected.ref.pdf"))?,
                );
            }
            _ => {}
        }
        let inventory =
            test_support::closed_case::candidate_inventory_bytes(files.keys().map(String::as_str))?;
        files.insert("case.inventory".to_owned(), inventory);
        result.insert((*case).to_owned(), files);
    }
    Ok(result)
}

fn authorities(repository: &Path, case: &str) -> Vec<String> {
    let pdf = "tests/corpus/pdf";
    let mut paths = vec![format!("{pdf}/{case}.tex")];
    let prefix = format!("{case}.expected.");
    for path in git_files(repository, pdf).expect("query PDF Git inventory") {
        let candidate = Path::new(&path);
        if candidate.parent() == Some(Path::new(pdf))
            && candidate
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        {
            paths.push(path);
        }
    }
    let shared = match case {
        "embedded_subset_truetype" => &["fixture.enc"][..],
        "embedded_tagged_spacing" => &["tagged_spacing.enc"][..],
        "embedded_type1" => &["embedded_type1.pfb"][..],
        "pk_bitmap_300" => &["cmr10.300pk"][..],
        "pk_bitmap_600" => &["cmr10.600pk"][..],
        _ => &[],
    };
    paths.extend(shared.iter().map(|name| format!("{pdf}/{name}")));
    paths.sort();
    paths
}

fn git_files(repository: &Path, root: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--", root])
        .output()
        .expect("query PDF Git inventory");
    ensure!(output.status.success(), "git ls-files failed");
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_owned)
        .collect())
}

fn provisioned_texlive_bytes(repository: &Path, name: &str) -> Result<Vec<u8>> {
    let path = repository.join("third_party/fonts").join(name);
    fs::read(&path).with_context(|| {
        format!(
            "read provisioned TeX Live input {}; run python3 scripts/provision.py worktree .",
            path.display()
        )
    })
}

fn is_font_case(case: &str) -> bool {
    case.starts_with("embedded_") || case.starts_with("pk_bitmap_")
}

fn print_report(inventories: &BTreeMap<String, BTreeMap<String, Vec<u8>>>) {
    for (case, files) in inventories {
        let bytes = files.values().map(Vec::len).sum::<usize>();
        let mut digest = Sha256::new();
        digest.update(b"umber-pdf-closed-case-v1\0");
        for (name, payload) in files {
            digest.update((name.len() as u64).to_be_bytes());
            digest.update(name.as_bytes());
            digest.update((payload.len() as u64).to_be_bytes());
            digest.update(payload);
        }
        println!(
            "pdf/{case}: {} files {bytes} bytes sha256={:x}",
            files.len(),
            digest.finalize()
        );
    }
}
