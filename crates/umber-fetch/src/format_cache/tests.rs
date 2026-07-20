#![allow(
    clippy::disallowed_methods,
    reason = "format-cache tests deliberately create and corrupt native cache files"
)]

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use tempfile::TempDir;
use tex_state::Universe;

use super::*;

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

fn format() -> Vec<u8> {
    Universe::new().dump_format().expect("schema-10 format")
}

#[test]
fn canonical_key_covers_every_identity_component() {
    let original = identity(FormatEngineMode::Latex);
    assert_eq!(original.key(), original.clone().key());
    assert_eq!(
        original.key().hex(),
        "bc00f33d67890f252f6c5ac6ec748271ac1a19bf490af1dbdb37f6e86528962c"
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
            0 => entry[24] ^= 0x80,
            1 => entry.truncate(entry.len() - 1),
            _ => {
                let metadata_len = read_u32(&entry, 12).expect("metadata length") as usize;
                let payload = ENTRY_HEADER_LEN + metadata_len;
                entry[payload] ^= 0x01;
                let digest = Sha256::digest(&entry[payload..]);
                entry[24..56].copy_from_slice(&digest);
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
    let directory = temp.path().join(DIRECTORY);
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
