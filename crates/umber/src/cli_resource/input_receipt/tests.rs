use super::*;
use crate::{FileKind, FileRequest};

fn file(name: &str, bytes: &[u8]) -> (FileRequest, ResolvedFile) {
    let key = FileRequestKey::new(FileKind::TexInput, name).expect("key");
    (
        FileRequest::new(key.clone(), name),
        ResolvedFile {
            request: key,
            virtual_path: format!("/texlive/tex/{name}"),
            expected_digest: None,
            bytes: bytes.to_vec().into(),
        },
    )
}

fn used(path: &str, bytes: &[u8]) -> (PathBuf, ContentIdentity) {
    (PathBuf::from(path), ContentIdentity::from_bytes(bytes))
}

#[test]
fn sorted_identity_rows_deduplicate_repeated_admissions() {
    let mut receipt = InputAdmissions::new(b"main");
    receipt
        .record_files(&[
            file("z.tex", b"z"),
            file("a.tex", b"a"),
            file("z.tex", b"z"),
        ])
        .expect("admissions");
    let dependencies = [used("/texlive/tex/a.tex", b"a")];
    let rendered =
        String::from_utf8(receipt.to_bytes(&dependencies).expect("receipt")).expect("UTF-8");
    assert_eq!(
        rendered,
        format!(
            "umber-input-admissions-v1\nmain\t4\t{}\nfile\tused\ttex:a.tex\t1\t{}\nfile\tadmitted\ttex:z.tex\t1\t{}\n",
            AHash64::for_bytes(HashDomain::DistributionContent, b"main").hex(),
            AHash64::for_bytes(HashDomain::DistributionContent, b"a").hex(),
            AHash64::for_bytes(HashDomain::DistributionContent, b"z").hex()
        )
    );
}

#[test]
fn changed_content_for_same_key_is_rejected() {
    let mut receipt = InputAdmissions::new(b"main");
    let error = receipt
        .record_files(&[file("same.tex", b"one"), file("same.tex", b"two")])
        .expect_err("content conflict");
    assert!(
        error
            .to_string()
            .contains("input admission changed content")
    );
}

#[test]
fn dependency_identity_must_match_admitted_bytes() {
    let mut receipt = InputAdmissions::new(b"main");
    receipt
        .record_files(&[file("same.tex", b"old")])
        .expect("admission");
    let dependencies = [used("/texlive/tex/same.tex", b"new")];
    let error = receipt
        .to_bytes(&dependencies)
        .expect_err("mismatched consumed identity");
    assert!(error.to_string().contains("no matching accepted admission"));
}

#[test]
fn consumed_external_path_requires_admission() {
    let receipt = InputAdmissions::new(b"main");
    let dependencies = [used("/texlive/tex/hidden.tex", b"hidden")];
    let error = receipt
        .to_bytes(&dependencies)
        .expect_err("missing consumed admission");
    assert!(error.to_string().contains("no matching accepted admission"));
}
