use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use umber_distribution::{ManifestShard, ShardFile, pack_shard};
use umber_hash::{AHash64, HashDomain};

use super::*;

#[test]
fn explicit_distribution_verifier_hashes_the_complete_graph() {
    let fixture = complete_fixture();
    assert_eq!(
        verify_distribution(fixture.root.path(), &fixture.root_digest)
            .expect("complete distribution verification"),
        DistributionVerificationReport {
            roots: 1,
            shards: 1,
            objects: 2,
            hashed_bytes: fixture.hashed_bytes,
        }
    );
}

#[test]
fn explicit_distribution_verifier_rejects_root_and_unrequested_object_mutation() {
    let fixture = complete_fixture();
    let root_path = fixture.root.path().join("manifest-v8.json");
    let root_bytes = fs::read(&root_path).expect("root bytes");
    fs::write(&root_path, b"mutated root").expect("mutate root");
    let root_error = verify_distribution(fixture.root.path(), &fixture.root_digest)
        .expect_err("root mutation must fail");
    assert!(
        root_error
            .to_string()
            .contains("root manifest digest mismatch")
    );

    fs::write(&root_path, root_bytes).expect("restore root");
    fs::write(&fixture.unrequested_object, b"mutated").expect("mutate unrequested object");
    let object_error = verify_distribution(fixture.root.path(), &fixture.root_digest)
        .expect_err("unrequested object mutation must fail the exhaustive verifier");
    assert!(object_error.to_string().contains("length does not match"));
}

struct Fixture {
    root: TempDir,
    root_digest: String,
    unrequested_object: PathBuf,
    hashed_bytes: u64,
}

fn complete_fixture() -> Fixture {
    let root = TempDir::new().expect("distribution tempdir");
    let objects = root.path().join("objects");
    fs::create_dir_all(&objects).expect("objects directory");
    let requested = b"requested";
    let unrequested = b"unrequested";
    let requested_digest = digest(requested);
    let unrequested_digest = digest(unrequested);
    let requested_name = format!("ahash64-v1-{requested_digest}");
    let unrequested_name = format!("ahash64-v1-{unrequested_digest}");
    fs::write(objects.join(&requested_name), requested).expect("requested object");
    let unrequested_object = objects.join(&unrequested_name);
    fs::write(&unrequested_object, unrequested).expect("unrequested object");
    let shard = pack_shard(&ManifestShard {
        schema: umber_distribution::INDEX_SHARD_SCHEMA,
        distribution: "verify".to_owned(),
        index: 0,
        files: BTreeMap::from([
            (
                "tex:requested.tex".to_owned(),
                ShardFile {
                    virtual_path: "/texlive/requested.tex".to_owned(),
                    object: requested_name,
                    ahash64: requested_digest,
                    bytes: requested.len() as u64,
                    dependencies: Vec::new(),
                },
            ),
            (
                "tex:unrequested.tex".to_owned(),
                ShardFile {
                    virtual_path: "/texlive/unrequested.tex".to_owned(),
                    object: unrequested_name,
                    ahash64: unrequested_digest,
                    bytes: unrequested.len() as u64,
                    dependencies: Vec::new(),
                },
            ),
        ]),
        fonts: BTreeMap::new(),
        legacy_mappings: BTreeMap::new(),
    })
    .expect("packed shard");
    let shard_digest = digest(&shard);
    fs::write(objects.join(format!("ahash64-v1-{shard_digest}")), &shard).expect("shard object");
    let root_bytes = ShardedManifestRoot {
        schema: umber_distribution::SHARDED_ROOT_SCHEMA,
        distribution: "verify".to_owned(),
        objects_base_url: "https://example.invalid/objects/".to_owned(),
        shard_bits: 0,
        shard_count: 1,
        shards: vec![shard_digest],
        formats: BTreeMap::new(),
    }
    .to_json();
    let root_digest = digest(root_bytes.as_bytes());
    fs::write(root.path().join("manifest-v8.json"), &root_bytes).expect("root manifest");
    Fixture {
        root,
        root_digest,
        unrequested_object,
        hashed_bytes: (root_bytes.len() + shard.len() + requested.len() + unrequested.len()) as u64,
    }
}

fn digest(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}
