use std::fs;
use std::path::Path;
use std::process::Command;

use super::*;

fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("run git")
            .success()
    );
}

fn fixture() -> (tempfile::TempDir, ClosedCase) {
    let temp = tempfile::tempdir().expect("tempdir");
    git(temp.path(), &["init", "-q"]);
    let root = temp.path().join("tests/corpus/example/only");
    fs::create_dir_all(&root).expect("case root");
    fs::write(
        root.join("case.inventory"),
        "closed-case-v1\nsource.tex\nexpected.log\nmeta.json\n",
    )
    .expect("inventory");
    fs::write(root.join("source.tex"), b"source\n").expect("source");
    fs::write(root.join("expected.log"), b"expected\n").expect("expected");
    fs::write(root.join("meta.json"), b"{}\n").expect("metadata");
    git(temp.path(), &["add", "tests/corpus/example/only"]);
    let case = ClosedCase::discover_at(temp.path(), "tests/corpus/example/only").expect("case");
    (temp, case)
}

fn contract() -> Contract {
    Contract {
        identity: CaseIdentity::new("tests/corpus/example", "only").expect("identity"),
        files: vec![
            TrackedFile {
                name: PayloadName::new("source.tex").expect("source"),
                role: FileRole::Input,
                sha256: Some(hex_digest(b"source\n")),
            },
            TrackedFile {
                name: PayloadName::new("expected.log").expect("expected"),
                role: FileRole::ExpectedOutput,
                sha256: None,
            },
            TrackedFile {
                name: PayloadName::new("meta.json").expect("metadata"),
                role: FileRole::Metadata,
                sha256: None,
            },
        ],
        status: CaseStatus::Xfail(Xfail {
            issue: "umber2-example".into(),
            reason: "pinned semantic difference".into(),
        }),
        profile: CaseProfile("raw-tex82-loaded".into()),
        source_closure: SourceClosure {
            primary: PayloadName::new("source.tex").expect("primary"),
            inputs: Vec::new(),
        },
        publication: PublicationMetadata {
            destination: RepositoryPath::new("tests/corpus/example/only").expect("destination"),
            authorities: vec![RepositoryPath::new("tests/corpus/example/only").expect("authority")],
        },
    }
}

#[test]
fn typed_contract_validates_and_stages_without_publication() {
    let (temp, case) = fixture();
    let contract = contract();
    let validated = contract.validate(&case).expect("typed contract");
    let destination = temp.path().join("candidate");
    let staged = validated.stage_into(&destination).expect("stage candidate");
    assert_eq!(
        staged
            .inventory()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["case.inventory", "expected.log", "meta.json", "source.tex"]
    );
    assert_eq!(
        fs::read_to_string(destination.join("case.inventory")).expect("staged inventory"),
        "closed-case-v1\nsource.tex\nexpected.log\nmeta.json\n"
    );
    assert_eq!(
        case.read("source.tex").expect("authority unchanged"),
        b"source\n"
    );
}

#[test]
fn contract_rejects_membership_hash_closure_and_publication_drift() {
    let (_temp, case) = fixture();

    let mut value = contract();
    value.files.pop();
    assert!(
        format!("{:#}", value.validate(&case).expect_err("missing accepted"))
            .contains("inventory mismatch")
    );

    let mut value = contract();
    value.files.swap(1, 2);
    assert!(
        format!("{:#}", value.validate(&case).expect_err("reorder accepted"))
            .contains("order mismatch")
    );

    let mut value = contract();
    value.files[0].sha256 = Some("0".repeat(64));
    assert!(
        format!("{:#}", value.validate(&case).expect_err("hash accepted"))
            .contains("SHA-256 mismatch")
    );

    let mut value = contract();
    value
        .source_closure
        .inputs
        .push(PayloadName::new("expected.log").expect("name"));
    assert!(
        format!(
            "{:#}",
            value.validate(&case).expect_err("output closure accepted")
        )
        .contains("not a tracked input")
    );

    let mut value = contract();
    value.publication.destination =
        RepositoryPath::new("tests/corpus/example/elsewhere").expect("destination");
    assert!(
        format!(
            "{:#}",
            value
                .validate(&case)
                .expect_err("wrong destination accepted")
        )
        .contains("publication destination")
    );
}

#[test]
fn typed_paths_reject_traversal_absolute_nested_payload_and_target_authority() {
    for path in ["../input", "/tmp/input", "./input"] {
        assert!(
            RepositoryPath::new(path).is_err(),
            "unsafe path accepted: {path}"
        );
    }
    assert!(PayloadName::new("nested/input").is_err());
    assert!(RepositoryPath::new("target/generated/case").is_err());
}

#[test]
fn local_edits_remain_valid_when_a_family_does_not_pin_hashes() {
    let (temp, case) = fixture();
    fs::write(
        temp.path().join("tests/corpus/example/only/expected.log"),
        b"locally regenerated\n",
    )
    .expect("local edit");
    let mut value = contract();
    value.files[0].sha256 = None;
    value.validate(&case).expect("local edit workflow");
}

#[test]
fn staged_inventory_preserves_old_unmanifested_and_closed_membership_rules() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("payload"), b"bytes").expect("payload");
    assert_eq!(
        StagedCase::validate(temp.path())
            .expect("legacy unmanifested case")
            .inventory()
            .len(),
        1
    );
    fs::write(
        temp.path().join("case.inventory"),
        "closed-case-v1\npayload\nmissing\n",
    )
    .expect("inventory");
    assert!(
        format!(
            "{:#}",
            StagedCase::validate(temp.path()).expect_err("missing accepted")
        )
        .contains("staged closed inventory mismatch")
    );
}
