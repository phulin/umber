#![allow(clippy::disallowed_methods)] // Git-backed host-side fixture inventory gate.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const AREAS: &[&str] = &[
    "align",
    "etex_exec",
    "exec",
    "expand",
    "math",
    "tex_exec",
    "tex_exec_io",
    "typeset",
];

#[test]
fn execution_minifixtures_are_closed_tracked_directories() {
    let repository = test_support::repository_root();
    let corpus = repository.join("tests/corpus");
    let primary = repository
        .canonicalize()
        .expect("canonicalize selected checkout");
    let mut actual = BTreeSet::new();

    for area in AREAS {
        let area_path = corpus.join(area);
        let mut case_names = BTreeSet::new();
        for entry in fs::read_dir(&area_path).expect("read fixture area") {
            let entry = entry.expect("read fixture-area entry");
            let file_type = entry.file_type().expect("read fixture-area entry type");
            assert!(
                file_type.is_dir() && !file_type.is_symlink(),
                "{} must contain only regular case directories",
                area_path.display()
            );
            let case = entry.file_name().into_string().expect("case name is UTF-8");
            assert!(
                case_names.insert(case.clone()),
                "duplicate case {area}/{case}"
            );
            let case_path = entry.path();
            assert!(
                case_path.join(format!("{case}.tex")).is_file(),
                "{area}/{case} lacks {case}.tex"
            );
            let canonical = case_path
                .canonicalize()
                .expect("canonicalize case directory");
            assert!(
                canonical.starts_with(&primary),
                "{area}/{case} resolves outside selected checkout"
            );
            assert!(
                !canonical.starts_with(repository.join("target")),
                "{area}/{case} resolves through target"
            );
            collect_case_files(&repository, &case_path, &mut actual);
        }
    }

    let tracked = tracked_area_files(&repository);
    assert_eq!(
        actual, tracked,
        "execution minifixture trees must contain exactly regular Git-tracked files"
    );
}

fn collect_case_files(repository: &Path, case: &Path, files: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(case).expect("read case directory") {
        let entry = entry.expect("read case entry");
        let kind = entry.file_type().expect("read case entry type");
        assert!(
            kind.is_file() && !kind.is_symlink(),
            "{} is not a regular file",
            entry.path().display()
        );
        let relative = entry
            .path()
            .strip_prefix(repository)
            .expect("case stays in repository")
            .to_owned();
        assert!(
            files.insert(relative.clone()),
            "duplicate fixture path {}",
            relative.display()
        );
    }
}

fn tracked_area_files(repository: &Path) -> BTreeSet<PathBuf> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--stage", "--"]);
    for area in AREAS {
        command.arg(format!("tests/corpus/{area}"));
    }
    let output = command.output().expect("run git ls-files");
    assert!(output.status.success(), "git ls-files failed");
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .lines()
        .map(|line| {
            let (metadata, path) = line.split_once('\t').expect("staged Git record");
            let mode = metadata.split_whitespace().next().expect("Git mode");
            assert!(
                matches!(mode, "100644" | "100755"),
                "{path} has forbidden Git mode {mode}"
            );
            PathBuf::from(path)
        })
        .collect()
}
