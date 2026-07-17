#![allow(clippy::disallowed_methods)] // host-side audit of committed fixtures

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const PINNED_COMMIT: &str = "74252e608e5f8115375c532eb25416430a9f52eb";
const IMPORTED_FILE_COUNT: usize = 113;
const UPSTREAM_MODULE_COUNT: usize = 51;
const UPSTREAM_ASSERTION_COUNT: usize = 1_275;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema: u32,
    upstream_repository: String,
    upstream_commit: String,
    compatibility_version: String,
    license: String,
    files: Vec<FixtureFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureFile {
    path: String,
    upstream_path: String,
    bytes: u64,
    sha256: String,
}

#[test]
fn fixture_manifest_is_complete_and_pinned() {
    let root = fixture_root();
    let manifest_bytes = fs::read(root.join("manifest.json"))
        .unwrap_or_else(|error| panic!("failed to read bibliography fixture manifest: {error}"));
    let manifest: FixtureManifest = serde_json::from_slice(&manifest_bytes)
        .unwrap_or_else(|error| panic!("invalid bibliography fixture manifest: {error}"));

    assert_eq!(manifest.schema, 1);
    assert_eq!(
        manifest.upstream_repository,
        "https://github.com/plk/biber.git"
    );
    assert_eq!(manifest.upstream_commit, PINNED_COMMIT);
    assert_eq!(manifest.compatibility_version, "2.22 beta");
    assert_eq!(manifest.license, "Artistic-2.0");
    assert_eq!(manifest.files.len(), IMPORTED_FILE_COUNT);

    let mut declared = BTreeSet::new();
    for fixture in &manifest.files {
        assert!(
            declared.insert(fixture.path.clone()),
            "duplicate manifest path: {}",
            fixture.path
        );
        let expected_upstream_path = if fixture.path == "LICENSE.Artistic-2.0" {
            "LICENSE".to_owned()
        } else {
            format!("t/{}", fixture.path)
        };
        assert_eq!(fixture.upstream_path, expected_upstream_path);
        let path = root.join(&fixture.path);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("failed to read pinned fixture {}: {error}", path.display())
        });
        assert_eq!(
            bytes.len() as u64,
            fixture.bytes,
            "byte length drift for {}",
            fixture.path
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "SHA-256 drift for {}",
            fixture.path
        );
    }

    let present = imported_paths(&root);
    assert_eq!(
        declared, present,
        "manifest must name every imported file and no absent files"
    );
}

#[test]
fn classic_fixture_manifest_and_inventory_are_complete_and_pinned() {
    let root = classic_fixture_root();
    let manifest: Value = read_json(&root.join("manifest.json"));
    assert_eq!(manifest["schema"], 1);
    assert_eq!(
        manifest["compatibility"],
        "classic-bibtex-0.99d-texlive-2025-web2c"
    );
    assert_eq!(
        manifest["source_archive"]["url"],
        "https://ftp.math.utah.edu/pub/tex/historic/systems/texlive/2025/texlive-20250308-source.tar.xz"
    );
    assert_eq!(
        manifest["source_archive"]["sha512"],
        "0837c935488b96cfc8dd79f1298f283b467ab68b4163cee9cb04b79e80195982fdc5ae8a80058dc7d3e99206bfda8b3bdd11340425b08f60cbef70d5a0e22702"
    );
    assert_eq!(
        manifest["merged_program"]["web_path"],
        "texk/web2c/bibtex.web"
    );
    assert_eq!(
        manifest["merged_program"]["web_sha256"],
        "38b9ba09fce5abb6f7ec135a2474b26c0d8c3a8b883df2d1c07072d33bc331ed"
    );
    assert_eq!(
        manifest["merged_program"]["change_path"],
        "texk/web2c/bibtex.ch"
    );
    assert_eq!(
        manifest["merged_program"]["change_sha256"],
        "9bffb931a113278d3c9304248a70b47f2576f7ee86fe6c1ae2160865ed0ea716"
    );
    assert_eq!(
        manifest["merged_program"]["merged_sha256"],
        "a0362ee3ca112207a5a666a5bb89484c4bb8c1a44d99c1ea824767b2eaafec79"
    );
    assert_eq!(
        manifest["merged_program"]["tangle_command"],
        "tangle bibtex bibtex"
    );
    assert_eq!(
        manifest["merged_program"]["merged_path"],
        "build/texk/web2c/bibtex.p"
    );
    assert_eq!(
        manifest["merged_program"]["web2c_c_sha256"],
        "848e79f7b29e5a2ad2388ffcfc486399c176f0a2dd2d6e83d55188de532bbc3d"
    );
    assert_eq!(
        manifest["merged_program"]["web2c_h_sha256"],
        "2ffa94f92b6c15b16aad99cc39b587f9e34e98731c911148921a6295b273157a"
    );
    assert_eq!(
        manifest["configuration"]["configure_arguments"],
        "--without-x --disable-shared --disable-all-pkgs --enable-tex --disable-synctex --disable-xetex --enable-missing -C CFLAGS=-O2 CXXFLAGS=-O2"
    );
    assert_eq!(
        manifest["configuration"]["texmf_cnf_path"],
        "texk/kpathsea/texmf.cnf"
    );
    assert_eq!(
        manifest["configuration"]["texmf_cnf_sha256"],
        "75cc5499ea9d15d1cf68722e75c846155fac55f1bbc2f0ca102ff5d423f49b29"
    );
    assert_eq!(
        manifest["configuration"]["c_auto_sha256"],
        "20553e51994937db88c411bd5aa39d1e34965309a184f14aae02c19ebded1c1d"
    );
    assert_eq!(manifest["configuration"]["environment"]["LC_ALL"], "C");
    assert_eq!(manifest["configuration"]["environment"]["LANGUAGE"], "C");
    assert_eq!(manifest["configuration"]["environment"]["BIBINPUTS"], ".");
    assert_eq!(manifest["configuration"]["environment"]["BSTINPUTS"], ".");
    assert_eq!(
        manifest["reference_executable"]["path"],
        "build/texk/web2c/bibtex"
    );
    assert_eq!(
        manifest["reference_executable"]["sha256"],
        "fcd33ae491e1adfc84a636015d3840ba49556649c65f3bf2db2fa7d2f948dc7e"
    );
    assert_eq!(manifest["reference_executable"]["platform"], "Darwin-arm64");
    assert_eq!(
        manifest["reference_executable"]["compiler"],
        "Apple clang 17.0.0"
    );
    assert_eq!(
        manifest["reference_executable"]["banner"],
        "BibTeX 0.99d (TeX Live 2025); kpathsea 6.4.1"
    );
    assert_eq!(manifest["normalizations"].as_array().map(Vec::len), Some(0));

    let standard_styles = manifest["standard_styles"]
        .as_array()
        .expect("standard styles must be an array");
    assert_eq!(standard_styles.len(), 2);
    for (style, name, source, path) in [
        (
            &standard_styles[0],
            "plain",
            "TeX Live 2025 texmf-dist/bibtex/bst/base/plain.bst",
            "styles/plain.bst",
        ),
        (
            &standard_styles[1],
            "apalike",
            "TeX Live 2025 texmf-dist/bibtex/bst/base/apalike.bst",
            "styles/apalike.bst",
        ),
    ] {
        assert_eq!(style["name"], name);
        assert_eq!(style["source"], source);
        assert_eq!(style["path"], path);
        assert_file_identity(
            &root.join(path),
            style["bytes"]
                .as_u64()
                .expect("style bytes must be unsigned"),
            style["sha256"]
                .as_str()
                .expect("style SHA-256 must be text"),
        );
    }
    let standard_cases = manifest["standard_style_execution_cases"]
        .as_array()
        .expect("standard-style execution cases must be an array");
    assert_eq!(standard_cases.len(), 3);
    for (case, name) in standard_cases.iter().zip(["plain", "apalike", "xampl"]) {
        assert_eq!(case["name"], name);
        let files = case["files"]
            .as_array()
            .expect("execution files must be an array");
        assert_eq!(files.len(), 3);
        for file in files {
            let path = file["path"]
                .as_str()
                .expect("execution fixture path must be text");
            assert_file_identity(
                &root.join(path),
                file["bytes"]
                    .as_u64()
                    .expect("execution bytes must be unsigned"),
                file["sha256"]
                    .as_str()
                    .expect("execution SHA-256 must be text"),
            );
        }
    }

    let real_world_styles = manifest["real_world_styles"]
        .as_array()
        .expect("real-world styles must be an array");
    assert_eq!(real_world_styles.len(), 2);
    let elsarticle = &real_world_styles[0];
    assert_eq!(elsarticle["name"], "elsarticle-num");
    assert_eq!(
        elsarticle["source"],
        "TeX Live 2025 texmf-dist/bibtex/bst/elsarticle/elsarticle-num.bst"
    );
    assert_eq!(
        elsarticle["upstream_url"],
        "https://ctan.org/pkg/elsarticle"
    );
    assert_eq!(elsarticle["version"], "2.1");
    assert_eq!(elsarticle["revision"], "272 (2025-01-09)");
    assert_eq!(elsarticle["license"], "LPPL-1.3-or-later");
    assert_file_identity(
        &root.join(
            elsarticle["path"]
                .as_str()
                .expect("real-world style path must be text"),
        ),
        elsarticle["bytes"]
            .as_u64()
            .expect("real-world style bytes must be unsigned"),
        elsarticle["sha256"]
            .as_str()
            .expect("real-world style SHA-256 must be text"),
    );
    let ieeetran = &real_world_styles[1];
    assert_eq!(ieeetran["name"], "IEEEtran");
    assert_eq!(
        ieeetran["source"],
        "TeX Live 2025 texmf-dist/bibtex/bst/ieeetran/IEEEtran.bst"
    );
    assert_eq!(ieeetran["upstream_url"], "https://ctan.org/pkg/ieeetran");
    assert_eq!(ieeetran["version"], "1.14");
    assert_eq!(ieeetran["revision"], "2015-08-26");
    assert_eq!(ieeetran["license"], "LPPL-1.3");
    assert_file_identity(
        &root.join(
            ieeetran["path"]
                .as_str()
                .expect("real-world style path must be text"),
        ),
        ieeetran["bytes"]
            .as_u64()
            .expect("real-world style bytes must be unsigned"),
        ieeetran["sha256"]
            .as_str()
            .expect("real-world style SHA-256 must be text"),
    );

    let real_world_cases = manifest["real_world_execution_cases"]
        .as_array()
        .expect("real-world execution cases must be an array");
    assert_eq!(real_world_cases.len(), 5);
    for ((case, name), style) in real_world_cases
        .iter()
        .zip([
            "elsarticle-book",
            "elsarticle-article",
            "elsarticle-names",
            "elsarticle-month",
            "ieeetran",
        ])
        .zip([
            "elsarticle-num",
            "elsarticle-num",
            "elsarticle-num",
            "elsarticle-num",
            "IEEEtran",
        ])
    {
        assert_eq!(case["name"], name);
        assert_eq!(case["style"], style);
        assert_eq!(case["command"], serde_json::json!(["bibtex", name]));
        assert_eq!(case["status"], 0);
        assert_eq!(case["history"], "spotless");
        let coverage = &case["coverage"];
        assert!(
            !coverage["paths"]
                .as_array()
                .expect("coverage paths must be an array")
                .is_empty(),
            "{name} must document the style paths it adds"
        );
        let files = case["files"]
            .as_array()
            .expect("real-world execution files must be an array");
        let mut roles = BTreeSet::new();
        let mut log_path = None;
        for file in files {
            let path = file["path"]
                .as_str()
                .expect("real-world fixture path must be text");
            roles.insert(file["role"].as_str().expect("fixture role must be text"));
            if file["role"] == "blg-output" {
                log_path = Some(root.join(path));
            }
            assert_file_identity(
                &root.join(path),
                file["bytes"].as_u64().expect("bytes must be unsigned"),
                file["sha256"].as_str().expect("SHA-256 must be text"),
            );
        }
        assert_eq!(
            roles,
            BTreeSet::from([
                "aux-input",
                "bbl-output",
                "bib-input",
                "blg-output",
                "terminal-output",
            ])
        );
        let log = fs::read_to_string(log_path.expect("BLG fixture"))
            .expect("real-world BLG fixture must be UTF-8");
        for (builtin, expected) in coverage["reference_builtin_calls"]
            .as_object()
            .expect("reference builtin calls must be an object")
        {
            let actual = log
                .lines()
                .find_map(|line| {
                    line.split_once(" -- ")
                        .filter(|(found, _)| *found == builtin)
                })
                .and_then(|(_, calls)| calls.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("missing builtin count for {builtin} in {name}"));
            assert_eq!(
                actual,
                expected.as_u64().expect("call count must be unsigned")
            );
        }
    }
    assert_eq!(
        real_world_cases[1]["coverage"]["adds_builtins_beyond"],
        "elsarticle-book"
    );
    assert_eq!(
        real_world_cases[2]["coverage"]["format_name_pattern"],
        "{f.~}{vv~}{ll}{, jj}"
    );
    assert_eq!(
        real_world_cases[3]["coverage"]["style_macro"],
        serde_json::json!({ "jan": "Jan." })
    );
    assert_eq!(
        real_world_cases[4]["coverage"]["control"],
        serde_json::json!({ "forced_et_al": true, "repeated_name_dash": true })
    );

    let cases = manifest["cases"]
        .as_array()
        .expect("cases must be an array");
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0]["name"], "smoke");
    assert_eq!(cases[0]["command"], serde_json::json!(["bibtex", "smoke"]));
    assert_eq!(cases[0]["status"], 0);
    assert_eq!(cases[0]["history"], "warning");
    let mut roles = BTreeSet::new();
    let mut declared = BTreeSet::new();
    for file in cases[0]["files"]
        .as_array()
        .expect("case files must be an array")
    {
        let path = file["path"].as_str().expect("fixture path must be text");
        let case_path = path
            .strip_prefix("cases/")
            .expect("case fixture must live below cases/");
        assert!(
            declared.insert(case_path.to_owned()),
            "duplicate path: {path}"
        );
        roles.insert(file["role"].as_str().expect("role must be text"));
        assert_file_identity(
            &root.join(path),
            file["bytes"].as_u64().expect("bytes must be unsigned"),
            file["sha256"].as_str().expect("sha256 must be text"),
        );
    }
    assert_eq!(
        roles,
        BTreeSet::from([
            "aux-input",
            "bbl-output",
            "bib-input",
            "blg-output",
            "bst-input",
            "terminal-output",
        ])
    );
    for case in standard_cases {
        for file in case["files"]
            .as_array()
            .expect("execution files must be an array")
        {
            let path = file["path"]
                .as_str()
                .expect("execution fixture path must be text");
            let case_path = path
                .strip_prefix("cases/")
                .expect("execution fixture must live below cases/");
            assert!(
                declared.insert(case_path.to_owned()),
                "duplicate path: {path}"
            );
        }
    }
    for case in real_world_cases {
        for file in case["files"]
            .as_array()
            .expect("execution files must be an array")
        {
            let path = file["path"]
                .as_str()
                .expect("execution fixture path must be text");
            let case_path = path
                .strip_prefix("cases/")
                .expect("case fixture must live below cases/");
            assert!(
                declared.insert(case_path.to_owned()),
                "duplicate path: {path}"
            );
        }
    }
    assert_eq!(declared, imported_paths(&root.join("cases")));

    let inventory: Value = read_json(&root.join("inventory.json"));
    assert_eq!(inventory["schema"], 1);
    assert_inventory_names(
        &inventory["aux_commands"],
        &["\\citation", "\\bibdata", "\\bibstyle", "\\@input"],
    );
    assert_inventory_names(
        &inventory["bst_commands"],
        &[
            "ENTRY", "EXECUTE", "FUNCTION", "INTEGERS", "ITERATE", "MACRO", "READ", "REVERSE",
            "SORT", "STRINGS",
        ],
    );
    assert_inventory_names(
        &inventory["bib_commands"],
        &["@comment", "@preamble", "@string"],
    );
    assert_inventory_names(
        &inventory["builtins"],
        &[
            "=",
            ">",
            "<",
            "+",
            "-",
            "*",
            ":=",
            "add.period$",
            "call.type$",
            "change.case$",
            "chr.to.int$",
            "cite$",
            "duplicate$",
            "empty$",
            "format.name$",
            "if$",
            "int.to.chr$",
            "int.to.str$",
            "missing$",
            "newline$",
            "num.names$",
            "pop$",
            "preamble$",
            "purify$",
            "quote$",
            "skip$",
            "stack$",
            "substring$",
            "swap$",
            "text.length$",
            "text.prefix$",
            "top$",
            "type$",
            "warning$",
            "while$",
            "width$",
            "write$",
        ],
    );
    assert_inventory_names(
        &inventory["predefined_symbols"],
        &["crossref", "sort.key$", "entry.max$", "global.max$"],
    );
    for family in [
        "diagnostic_families",
        "reference_limits",
        "branch_families",
        "upstream_tests",
    ] {
        let entries = inventory[family]
            .as_array()
            .unwrap_or_else(|| panic!("{family} must be an array"));
        assert!(!entries.is_empty(), "{family} must not be empty");
        assert_owned(entries);
    }
    assert_eq!(
        inventory["diagnostic_families"].as_array().map(Vec::len),
        Some(15)
    );
    assert_eq!(
        inventory["reference_limits"].as_array().map(Vec::len),
        Some(18)
    );
    assert_eq!(
        inventory["branch_families"].as_array().map(Vec::len),
        Some(15)
    );
    assert_eq!(
        inventory["upstream_tests"].as_array().map(Vec::len),
        Some(17)
    );
}

#[test]
fn translated_suite_has_no_compatibility_allowances() {
    let upstream = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/upstream");
    let mut modules = 0;
    let mut assertions = 0;
    for entry in fs::read_dir(&upstream)
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", upstream.display()))
    {
        let path = entry.expect("valid upstream directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs")
            || path.file_name().is_some_and(|name| name == "mod.rs")
        {
            continue;
        }
        modules += 1;
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assertions += source.matches("#[test]").count();
        let expected_failure_marker = ["x", "fail"].concat();
        let unexpected_pass_marker = ["X", "PASS"].concat();
        let ignored_test_marker = ["#[", "ignore", "]"].concat();
        let expected_panic_marker = ["#[", "should_panic", "]"].concat();
        for forbidden in [
            ignored_test_marker.as_str(),
            expected_panic_marker.as_str(),
            expected_failure_marker.as_str(),
            unexpected_pass_marker.as_str(),
        ] {
            assert!(
                !source.contains(forbidden),
                "compatibility allowance `{forbidden}` remains in {}",
                path.display()
            );
        }
    }
    assert_eq!(modules, UPSTREAM_MODULE_COUNT);
    assert_eq!(assertions, UPSTREAM_ASSERTION_COUNT);
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/bib/upstream-2.22")
}

fn classic_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/bibtex")
}

fn read_json(path: &Path) -> Value {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("invalid JSON in {}: {error}", path.display()))
}

fn assert_file_identity(path: &Path, expected_bytes: u64, expected_sha256: &str) {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    assert_eq!(
        bytes.len() as u64,
        expected_bytes,
        "byte length drift for {}",
        path.display()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        expected_sha256,
        "SHA-256 drift for {}",
        path.display()
    );
}

fn assert_inventory_names(value: &Value, expected: &[&str]) {
    let entries = value.as_array().expect("inventory family must be an array");
    assert_owned(entries);
    let actual: Vec<_> = entries
        .iter()
        .map(|entry| entry["name"].as_str().expect("inventory name must be text"))
        .collect();
    assert_eq!(actual, expected);
}

fn assert_owned(entries: &[Value]) {
    for entry in entries {
        for field in ["implementation_owner", "test_owner"] {
            assert!(
                entry[field].as_str().is_some_and(|owner| !owner.is_empty()),
                "{} has no {field}",
                entry["name"]
            );
        }
    }
}

fn imported_paths(root: &Path) -> BTreeSet<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|error| panic!("failed to enumerate fixture: {error}"));
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| name != "manifest.json") {
                let relative = path
                    .strip_prefix(root)
                    .expect("fixture must be below corpus root");
                paths.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    paths
}
