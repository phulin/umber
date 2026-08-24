#![allow(
    clippy::disallowed_methods,
    reason = "format-cache tests deliberately create and corrupt native cache files"
)]

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use tempfile::TempDir;

use super::*;

const BLOB_DIRECTORY: &str = "blobs-v1";

fn identity(mode: FormatEngineMode) -> FormatCacheIdentity {
    FormatCacheIdentity::current(
        mode,
        FormatFingerprint::sha256(b"texlive-2026-r79639 root"),
        FormatFingerprint::sha256(b"sorted closure identity"),
        FormatFingerprint::sha256(b"latex-source.lock"),
        FormatCacheClock {
            time: 720,
            second: 0,
            day: 14,
            month: 7,
            year: 2026,
        },
        FormatFingerprint::sha256(b"release;features=default"),
    )
}

fn compound_identity(mode: FormatEngineMode) -> FormatCacheIdentity {
    FormatCacheIdentity {
        entry_kind: FormatCacheEntryKind::CompoundEvidence,
        ..identity(mode)
    }
}

fn format() -> Vec<u8> {
    crate::format_fixture::construct_format_in_worker(&crate::FormatRecipe::raw_tex82())
        .expect("schema-11 format")
        .image
}

#[test]
fn canonical_key_covers_every_identity_component() {
    let original = identity(FormatEngineMode::Latex);
    assert_eq!(original.key(), original.clone().key());
    assert_ne!(
        original.key(),
        compound_identity(FormatEngineMode::Latex).key()
    );
    assert_eq!(
        original.key().hex(),
        "a1003771df30812d0be73236a014ca7d1627f885445c8e94675c142a5cf9cd8a"
    );

    let mutations = [
        FormatCacheIdentity {
            engine_mode: FormatEngineMode::PdfLatex,
            ..original.clone()
        },
        FormatCacheIdentity {
            format_schema: original.format_schema + 1,
            ..original.clone()
        },
        FormatCacheIdentity {
            format_abi_fingerprint: original.format_abi_fingerprint + 1,
            ..original.clone()
        },
        FormatCacheIdentity {
            lookup_configuration_fingerprint: original.lookup_configuration_fingerprint + 1,
            ..original.clone()
        },
        FormatCacheIdentity {
            distribution_snapshot: FormatFingerprint::sha256(b"other snapshot"),
            ..original.clone()
        },
        FormatCacheIdentity {
            format_closure: FormatFingerprint::sha256(b"other closure"),
            ..original.clone()
        },
        FormatCacheIdentity {
            source_lock: FormatFingerprint::sha256(b"other lock"),
            ..original.clone()
        },
        FormatCacheIdentity {
            build_configuration: FormatFingerprint::sha256(b"debug"),
            ..original.clone()
        },
        FormatCacheIdentity {
            semantic_contract: FormatFingerprint::sha256(b"other semantic contract"),
            ..original.clone()
        },
        FormatCacheIdentity {
            producer_contract: FormatFingerprint::sha256(b"other producer"),
            ..original.clone()
        },
        FormatCacheIdentity {
            resource_closure: FormatFingerprint::sha256(b"other resources"),
            ..original.clone()
        },
        FormatCacheIdentity {
            generation_guards: FormatFingerprint::sha256(b"other guards"),
            ..original.clone()
        },
        FormatCacheIdentity {
            job_clock: FormatCacheClock {
                second: 1,
                ..original.job_clock
            },
            ..original.clone()
        },
        FormatCacheIdentity {
            job_clock: FormatCacheClock {
                time: original.job_clock.time + 1,
                ..original.job_clock
            },
            ..original.clone()
        },
        FormatCacheIdentity {
            job_clock: FormatCacheClock {
                day: original.job_clock.day + 1,
                ..original.job_clock
            },
            ..original.clone()
        },
        FormatCacheIdentity {
            job_clock: FormatCacheClock {
                month: original.job_clock.month + 1,
                ..original.job_clock
            },
            ..original.clone()
        },
        FormatCacheIdentity {
            job_clock: FormatCacheClock {
                year: original.job_clock.year + 1,
                ..original.job_clock
            },
            ..original.clone()
        },
    ];
    for mutation in mutations {
        assert_ne!(mutation.key(), original.key());
    }
}

#[test]
fn legacy_format_layout_is_validated_and_migrated() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let key = identity(FormatEngineMode::Latex);
    let image = format();
    let legacy = temp.path().join(DIRECTORY).join(cache.name(&key));
    fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy namespace");
    fs::write(&legacy, encode_entry(&key, &image)).expect("legacy format entry");

    assert_eq!(
        cache
            .load(&key)
            .expect("legacy format load")
            .expect("legacy format hit")
            .as_bytes(),
        image
    );
    assert!(cache.path(&key).is_file(), "entry migrated to blob store");
}

#[test]
fn hit_miss_and_identity_mismatch_are_safe() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let latex = identity(FormatEngineMode::Latex);
    let pdf_latex = identity(FormatEngineMode::PdfLatex);
    assert!(cache.load(&latex).expect("cold load").is_none());

    let bytes = format();
    cache.store(&latex, &bytes).expect("store");
    assert_eq!(
        cache
            .load(&latex)
            .expect("hit load")
            .expect("cache hit")
            .as_bytes(),
        bytes
    );
    assert!(cache.load(&pdf_latex).expect("other identity").is_none());

    fs::copy(cache.path(&latex), cache.path(&pdf_latex)).expect("forge mismatched metadata");
    assert!(cache.load(&pdf_latex).expect("reject mismatch").is_none());
    assert!(!cache.path(&pdf_latex).exists());
}

#[test]
fn schema_transition_uses_a_disjoint_namespace_and_cannot_relabel_an_entry() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let current = identity(FormatEngineMode::Latex);
    let previous = FormatCacheIdentity {
        format_schema: current.format_schema - 1,
        ..current.clone()
    };
    let bytes = format();
    cache.store(&current, &bytes).expect("store current schema");

    assert_ne!(current.key(), previous.key());
    assert!(cache.load(&previous).expect("old namespace miss").is_none());
    fs::copy(cache.path(&current), cache.path(&previous)).expect("forge old-schema path");
    assert!(cache.load(&previous).expect("reject relabeling").is_none());
    assert!(!cache.path(&previous).exists());
    assert_eq!(
        cache
            .load(&current)
            .expect("current schema load")
            .expect("current schema hit")
            .as_bytes(),
        bytes
    );
}

#[test]
fn entry_encoding_is_deterministic_and_preserves_exact_format_bytes() {
    let key = identity(FormatEngineMode::Latex);
    let bytes = format();
    let first = encode_entry(&key, &bytes);
    let second = encode_entry(&key, &bytes);

    assert_eq!(first, second);
    assert_eq!(decode_entry(&first, &key), Some(bytes.as_slice()));
}

#[test]
fn compound_entry_is_atomic_and_rejects_missing_corrupt_and_cross_key_evidence() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let key = compound_identity(FormatEngineMode::Latex);
    let other = compound_identity(FormatEngineMode::PdfLatex);
    let image = format();
    let evidence = b"bounded-evidence-v1";
    let validator = |bytes: &[u8]| {
        (bytes == evidence)
            .then_some(())
            .ok_or_else(|| "invalid evidence".into())
    };
    cache
        .store_entry(&key, &image, evidence, validator)
        .expect("compound store");
    let hit = cache
        .load_entry(&key, validator)
        .expect("load")
        .expect("hit");
    assert_eq!(hit.image().as_bytes(), image);
    assert_eq!(hit.evidence(), evidence);

    fs::copy(cache.path(&key), cache.path(&other)).expect("cross-key copy");
    assert!(
        cache
            .load_entry(&other, validator)
            .expect("cross-key rejection")
            .is_none()
    );
    assert!(!cache.path(&other).exists());

    let path = cache.path(&key);
    let mut forged = fs::read(&path).expect("entry");
    forged[64] ^= 0x80;
    fs::write(&path, forged).expect("forge evidence digest");
    assert!(
        cache
            .load_entry(&key, validator)
            .expect("digest rejection")
            .is_none()
    );

    assert!(
        matches!(
            cache.store(&key, &image),
            Err(FormatCacheError::WrongEntryKind)
        ),
        "legacy store must not publish an evidence-aware key"
    );
    assert!(matches!(
        cache.load(&key),
        Err(FormatCacheError::WrongEntryKind)
    ));
}

#[test]
fn invalid_compound_entry_is_regenerated_once_under_the_key_lock() {
    let temp = TempDir::new().expect("tempdir");
    let cache = Arc::new(FormatCacheStore::new(temp.path()));
    let key = compound_identity(FormatEngineMode::Latex);
    let image = format();
    cache
        .store_entry(&key, &image, b"syntactically-opaque", |_| Ok(()))
        .expect("seed semantically invalid evidence");

    let constructions = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let cache = Arc::clone(&cache);
        let key = key.clone();
        let image = image.clone();
        let constructions = Arc::clone(&constructions);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            cache
                .ensure_entry::<FormatCacheError>(
                    &key,
                    |bytes| {
                        (bytes == b"canonical-evidence")
                            .then_some(())
                            .ok_or_else(|| "invalid semantic evidence".into())
                    },
                    || {
                        constructions.fetch_add(1, Ordering::SeqCst);
                        Ok((image, b"canonical-evidence".to_vec()))
                    },
                )
                .expect("locked regeneration")
        }));
    }
    barrier.wait();
    for thread in threads {
        let entry = thread.join().expect("join");
        assert_eq!(entry.evidence(), b"canonical-evidence");
    }
    assert_eq!(constructions.load(Ordering::SeqCst), 1);
}

#[test]
fn corrupt_truncated_and_decoder_invalid_entries_recover_as_misses() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let key = identity(FormatEngineMode::Latex);
    let bytes = format();

    for mutation in [0_usize, 1, 2] {
        cache.store(&key, &bytes).expect("store");
        let path = cache.path(&key);
        let mut entry = fs::read(&path).expect("entry");
        match mutation {
            0 => entry[28] ^= 0x80,
            1 => entry.truncate(entry.len() - 1),
            _ => {
                let namespace_len = u16::from_le_bytes(
                    entry[12..14]
                        .try_into()
                        .expect("blob namespace length field"),
                ) as usize;
                let key_len =
                    u16::from_le_bytes(entry[14..16].try_into().expect("blob key length field"))
                        as usize;
                let outer_payload = 64 + namespace_len + key_len;
                let metadata_len =
                    read_u32(&entry[outer_payload..], 12).expect("metadata length") as usize;
                let image = outer_payload + ENTRY_HEADER_LEN + metadata_len;
                entry[image] ^= 0x01;
                let image_digest = Sha256::digest(&entry[image..]);
                entry[outer_payload + 24..outer_payload + 56].copy_from_slice(&image_digest);
                let blob_digest = Sha256::digest(&entry[outer_payload..]);
                entry[28..60].copy_from_slice(&blob_digest);
            }
        }
        fs::write(&path, entry).expect("corrupt entry");
        assert!(cache.load(&key).expect("corruption is a miss").is_none());
        assert!(!path.exists());
    }
}

#[test]
fn interrupted_temporary_file_is_ignored() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let directory = temp.path().join(BLOB_DIRECTORY);
    fs::create_dir_all(&directory).expect("directory");
    fs::write(directory.join(".tmp-interrupted"), b"partial").expect("partial temp");
    assert!(
        cache
            .load(&identity(FormatEngineMode::Latex))
            .expect("load")
            .is_none()
    );
}

#[test]
fn concurrent_publishers_and_readers_observe_only_complete_entries() {
    let temp = TempDir::new().expect("tempdir");
    let cache = Arc::new(FormatCacheStore::new(temp.path()));
    let key = Arc::new(identity(FormatEngineMode::Latex));
    let bytes = Arc::new(format());
    let barrier = Arc::new(Barrier::new(9));
    let mut threads = Vec::new();
    for index in 0..8 {
        let cache = Arc::clone(&cache);
        let key = Arc::clone(&key);
        let bytes = Arc::clone(&bytes);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            if index < 4 {
                cache.store(&key, &bytes).expect("concurrent store");
            }
            for _ in 0..20 {
                if let Some(hit) = cache.load(&key).expect("concurrent load") {
                    assert_eq!(hit.as_bytes(), bytes.as_slice());
                }
            }
        }));
    }
    barrier.wait();
    for handle in threads {
        handle.join().expect("cache thread");
    }
    assert_eq!(
        cache
            .load(&key)
            .expect("final load")
            .expect("final hit")
            .as_bytes(),
        bytes.as_slice()
    );
}

#[test]
fn store_refuses_unvalidated_format_bytes() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    assert!(matches!(
        cache.store(&identity(FormatEngineMode::Latex), b"not a format"),
        Err(FormatCacheError::InvalidFormat(_))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_namespace_and_entry_are_never_followed() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("tempdir");
    let outside = TempDir::new().expect("outside");
    let cache = FormatCacheStore::new(temp.path());
    symlink(outside.path(), temp.path().join(BLOB_DIRECTORY)).expect("namespace symlink");
    assert!(
        cache
            .store(&identity(FormatEngineMode::Latex), &format())
            .is_err()
    );
    assert!(
        fs::read_dir(outside.path())
            .expect("outside readable")
            .next()
            .is_none()
    );

    fs::remove_file(temp.path().join(BLOB_DIRECTORY)).expect("remove namespace symlink");
    fs::create_dir(temp.path().join(BLOB_DIRECTORY)).expect("real namespace");
    let target = outside.path().join("target");
    fs::write(&target, b"untouched").expect("target");
    let key = identity(FormatEngineMode::Latex);
    symlink(&target, cache.path(&key)).expect("entry symlink");
    assert!(cache.load(&key).is_err());
    assert_eq!(fs::read(&target).expect("target remains"), b"untouched");
}

#[test]
fn validation_failure_never_replaces_a_live_entry() {
    let temp = TempDir::new().expect("tempdir");
    let cache = FormatCacheStore::new(temp.path());
    let key = identity(FormatEngineMode::Latex);
    let bytes = format();
    cache.store(&key, &bytes).expect("initial entry");
    let before = fs::read(cache.path(&key)).expect("live entry");

    assert!(matches!(
        cache.store(&key, b"not a format"),
        Err(FormatCacheError::InvalidFormat(_))
    ));
    assert_eq!(fs::read(cache.path(&key)).expect("entry survives"), before);
}

#[cfg(unix)]
#[test]
fn competing_processes_serialize_publication_and_leave_no_temporary_files() {
    let temp = TempDir::new().expect("tempdir");
    let executable = std::env::current_exe().expect("current test executable");
    let mut children = Vec::new();
    for _ in 0..8 {
        children.push(
            Command::new(&executable)
                .args([
                    "--ignored",
                    "--exact",
                    "format_cache::tests::process_cache_worker",
                ])
                .env("UMBER_FORMAT_CACHE_WORKER_ROOT", temp.path())
                .spawn()
                .expect("spawn cache worker"),
        );
    }
    for mut child in children {
        assert!(child.wait().expect("wait for cache worker").success());
    }

    let cache = FormatCacheStore::new(temp.path());
    assert_eq!(
        cache
            .load(&identity(FormatEngineMode::Latex))
            .expect("load process winner")
            .expect("process winner")
            .as_bytes(),
        format()
    );
    let names: Vec<_> = fs::read_dir(temp.path().join(BLOB_DIRECTORY))
        .expect("cache namespace")
        .map(|entry| {
            entry
                .expect("cache namespace entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names
            .iter()
            .filter(|name| name.starts_with("sha256-"))
            .count(),
        1
    );
    assert!(!names.iter().any(|name| name.starts_with(".tmp-")));
}

#[cfg(unix)]
#[test]
fn crashed_lock_owner_is_recovered_and_corrupt_quarantine_is_exact() {
    let temp = TempDir::new().expect("tempdir");
    let executable = std::env::current_exe().expect("current test executable");
    let status = Command::new(executable)
        .args([
            "--ignored",
            "--exact",
            "format_cache::tests::process_cache_worker",
        ])
        .env("UMBER_FORMAT_CACHE_WORKER_ROOT", temp.path())
        .env("UMBER_FORMAT_CACHE_WORKER_ABORT_WITH_LOCK", "1")
        .status()
        .expect("run crashing cache worker");
    assert!(!status.success());

    let cache = FormatCacheStore::new(temp.path());
    let key = identity(FormatEngineMode::Latex);
    cache
        .store(&key, &format())
        .expect("recover abandoned lock");
    fs::write(cache.path(&key), b"corrupt").expect("install corrupt entry");
    assert!(
        cache
            .load(&key)
            .expect("quarantine corrupt entry")
            .is_none()
    );
    let names: Vec<_> = fs::read_dir(temp.path().join(BLOB_DIRECTORY))
        .expect("cache namespace")
        .map(|entry| {
            entry
                .expect("cache namespace entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(!names.iter().any(|name| {
        name.starts_with(".tmp-") || name.starts_with(".corrupt-") || name.starts_with("sha256-")
    }));
}

#[cfg(unix)]
#[test]
fn non_regular_entry_and_symlinked_root_component_fail_closed() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("tempdir");
    let outside = TempDir::new().expect("outside");
    let escaped_root = temp.path().join("escape").join("cache");
    symlink(outside.path(), temp.path().join("escape")).expect("root-component symlink");
    assert!(
        FormatCacheStore::new(&escaped_root)
            .store(&identity(FormatEngineMode::Latex), &format())
            .is_err()
    );
    assert!(
        fs::read_dir(outside.path())
            .expect("outside readable")
            .next()
            .is_none()
    );

    let cache = FormatCacheStore::new(temp.path().join("real"));
    cache
        .store(&identity(FormatEngineMode::Latex), &format())
        .expect("create authority");
    let key = identity(FormatEngineMode::Latex);
    fs::remove_file(cache.path(&key)).expect("remove entry");
    fs::create_dir(cache.path(&key)).expect("replace entry with directory");
    assert!(cache.load(&key).is_err());
}

#[cfg(unix)]
#[test]
#[ignore = "subprocess-only helper"]
fn process_cache_worker() {
    let Some(root) = std::env::var_os("UMBER_FORMAT_CACHE_WORKER_ROOT") else {
        return;
    };
    let cache = FormatCacheStore::new(PathBuf::from(root));
    let key = identity(FormatEngineMode::Latex);
    if std::env::var_os("UMBER_FORMAT_CACHE_WORKER_ABORT_WITH_LOCK").is_some() {
        let spec = cache
            .spec(&key, compound_limit())
            .expect("worker blob specification");
        let _ = cache.blobs.ensure_validated::<FormatCacheError>(
            &spec,
            |_| Ok(()),
            || -> Result<Vec<u8>, FormatCacheError> { std::process::abort() },
        );
        unreachable!("abort terminates the worker");
    }
    cache.store(&key, &format()).expect("worker store");
}
