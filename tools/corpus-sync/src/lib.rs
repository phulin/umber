#![allow(clippy::disallowed_methods)] // host-side acquisition tool

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use corpus_manifest::{Manifest, parse_manifest_file};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub manifest_path: PathBuf,
    pub dest_dir: PathBuf,
    pub offline: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            manifest_path: PathBuf::from("tests/corpus-manifest.txt"),
            dest_dir: PathBuf::from("third_party/corpus"),
            offline: false,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SyncReport {
    pub documents: Vec<DocumentStatus>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DocumentStatus {
    Verified { name: String, path: PathBuf },
    Fetched { name: String, path: PathBuf },
}

impl fmt::Display for DocumentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified { name, path } => {
                write!(f, "verified {name}: {}", path.display())
            }
            Self::Fetched { name, path } => {
                write!(f, "fetched {name}: {}", path.display())
            }
        }
    }
}

pub fn sync_corpus(options: &SyncOptions) -> Result<SyncReport> {
    let manifest = read_manifest(&options.manifest_path)?;
    fs::create_dir_all(&options.dest_dir)
        .with_context(|| format!("failed to create {}", options.dest_dir.display()))?;

    let mut documents = Vec::with_capacity(manifest.support.len() + manifest.doc.len());
    for file in manifest.support {
        documents.push(sync_file(&file.name, &file.urls, &file.sha256, options)?);
    }
    for doc in manifest.doc {
        documents.push(sync_file(&doc.name, &doc.urls, &doc.sha256, options)?);
    }

    Ok(SyncReport { documents })
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let parsed =
        parse_manifest_file(path).with_context(|| format!("failed to parse {}", path.display()))?;
    if parsed.doc.is_empty() {
        bail!(
            "manifest {} does not contain any doc entries",
            path.display()
        );
    }
    Ok(parsed)
}

fn sync_file(
    name: &str,
    urls: &[String],
    expected_sha256: &str,
    options: &SyncOptions,
) -> Result<DocumentStatus> {
    let path = options.dest_dir.join(name);
    if path.exists() {
        verify_existing(name, expected_sha256, &path)?;
        return Ok(DocumentStatus::Verified {
            name: name.to_string(),
            path,
        });
    }

    if options.offline {
        bail!(
            "missing corpus document {} at {} while running --offline",
            name,
            path.display()
        );
    }

    let bytes = fetch_verified(name, urls, expected_sha256, &path)?;

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, &bytes)
        .with_context(|| format!("failed to write temporary file {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to move {} into place", path.display()))?;

    Ok(DocumentStatus::Fetched {
        name: name.to_string(),
        path,
    })
}

fn fetch_verified(name: &str, urls: &[String], expected_sha256: &str, path: &Path) -> Result<Vec<u8>> {
    let mut failures = Vec::with_capacity(urls.len());
    for url in urls {
        match fetch_url(url) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                if actual == expected_sha256 {
                    return Ok(bytes);
                }
                failures.push(format!(
                    "{url}: sha256 mismatch (expected {expected_sha256}, got {actual})"
                ));
            }
            Err(error) => failures.push(format!("{url}: {error:#}")),
        }
    }
    bail!(
        "all {} locators failed for corpus document {}: {}; not writing {}",
        urls.len(),
        name,
        failures.join("; "),
        path.display()
    )
}

fn verify_existing(name: &str, expected_sha256: &str, path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let actual = sha256_hex(&bytes);
    if actual != expected_sha256 {
        bail!(
            "sha256 mismatch for cached {} at {}: expected {}, got {}; remove the file and rerun to refetch",
            name,
            path.display(),
            expected_sha256,
            actual
        );
    }
    Ok(())
}

fn fetch_url(url: &str) -> Result<Vec<u8>> {
    let mut response = reqwest::blocking::get(url)?.error_for_status()?;
    let mut bytes = Vec::new();
    response.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}


#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use anyhow::Result;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn verifies_cached_document_without_fetching() -> Result<()> {
        let temp = TempDir::new()?;
        let dest = temp.path().join("corpus");
        fs::create_dir(&dest)?;
        fs::write(dest.join("sample.tex"), b"cached")?;
        let manifest = write_manifest(temp.path(), "http://127.0.0.1:9/sample.tex", "cached")?;

        let report = sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest.clone(),
            offline: true,
        })?;

        assert_eq!(
            report.documents,
            vec![
                DocumentStatus::Verified {
                    name: "plain.tex".to_string(),
                    path: dest.join("plain.tex")
                },
                DocumentStatus::Verified {
                    name: "sample.tex".to_string(),
                    path: dest.join("sample.tex")
                }
            ]
        );
        Ok(())
    }

    #[test]
    fn reports_cached_hash_drift() -> Result<()> {
        let temp = TempDir::new()?;
        let dest = temp.path().join("corpus");
        fs::create_dir(&dest)?;
        fs::write(dest.join("sample.tex"), b"changed")?;
        let manifest = write_manifest(temp.path(), "http://127.0.0.1:9/sample.tex", "cached")?;

        let error = sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest,
            offline: true,
        })
        .expect_err("cached drift should fail");

        assert!(
            error.to_string().contains("sha256 mismatch for cached"),
            "{error:#}"
        );
        Ok(())
    }

    #[test]
    fn fetches_missing_document() -> Result<()> {
        let temp = TempDir::new()?;
        let body = b"from server";
        let url = serve_once(body)?;
        let manifest = write_manifest(temp.path(), &url, std::str::from_utf8(body)?)?;
        let dest = temp.path().join("corpus");

        let report = sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest.clone(),
            offline: false,
        })?;

        assert_eq!(fs::read(dest.join("sample.tex"))?, body);
        assert_eq!(
            report.documents,
            vec![
                DocumentStatus::Verified {
                    name: "plain.tex".to_string(),
                    path: dest.join("plain.tex")
                },
                DocumentStatus::Fetched {
                    name: "sample.tex".to_string(),
                    path: dest.join("sample.tex")
                }
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_fetched_hash_drift_without_writing_file() -> Result<()> {
        let temp = TempDir::new()?;
        let url = serve_once(b"from server")?;
        let manifest = write_manifest(temp.path(), &url, "different");
        let dest = temp.path().join("corpus");

        let error = sync_corpus(&SyncOptions {
            manifest_path: manifest?,
            dest_dir: dest.clone(),
            offline: false,
        })
        .expect_err("fetched drift should fail");

        let message = error.to_string();
        assert!(message.contains("all 1 locators failed"), "{error:#}");
        assert!(message.contains("sha256 mismatch"), "{error:#}");
        assert!(!dest.join("sample.tex").exists());
        Ok(())
    }

    #[test]
    fn falls_back_after_transport_failure() -> Result<()> {
        let temp = TempDir::new()?;
        let good = serve_once(b"from fallback")?;
        let urls = [
            "http://127.0.0.1:9/unavailable".to_string(),
            good,
        ];
        let manifest = write_manifest_with_urls(temp.path(), &urls, b"from fallback")?;
        let dest = temp.path().join("corpus");

        sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest.clone(),
            offline: false,
        })?;
        assert_eq!(fs::read(dest.join("sample.tex"))?, b"from fallback");
        Ok(())
    }

    #[test]
    fn falls_back_after_digest_mismatch() -> Result<()> {
        let temp = TempDir::new()?;
        let urls = [serve_once(b"wrong")?, serve_once(b"right")?];
        let manifest = write_manifest_with_urls(temp.path(), &urls, b"right")?;
        let dest = temp.path().join("corpus");

        sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest.clone(),
            offline: false,
        })?;
        assert_eq!(fs::read(dest.join("sample.tex"))?, b"right");
        Ok(())
    }

    #[test]
    fn reports_every_locator_failure_without_writing() -> Result<()> {
        let temp = TempDir::new()?;
        let urls = [serve_once(b"wrong-one")?, serve_once(b"wrong-two")?];
        let manifest = write_manifest_with_urls(temp.path(), &urls, b"expected")?;
        let dest = temp.path().join("corpus");
        let error = sync_corpus(&SyncOptions {
            manifest_path: manifest,
            dest_dir: dest.clone(),
            offline: false,
        })
        .expect_err("all digest mismatches must fail");

        let expected = format!(
            "all 2 locators failed for corpus document sample.tex: {}: sha256 mismatch (expected {}, got {}); {}: sha256 mismatch (expected {}, got {}); not writing {}",
            urls[0],
            sha256_hex(b"expected"),
            sha256_hex(b"wrong-one"),
            urls[1],
            sha256_hex(b"expected"),
            sha256_hex(b"wrong-two"),
            dest.join("sample.tex").display()
        );
        assert_eq!(error.to_string(), expected);
        assert!(!dest.join("sample.tex").exists());
        Ok(())
    }

    fn write_manifest(dir: &Path, url: &str, content: &str) -> Result<PathBuf> {
        write_manifest_with_urls(dir, &[url.to_string()], content.as_bytes())
    }

    fn write_manifest_with_urls(dir: &Path, urls: &[String], content: &[u8]) -> Result<PathBuf> {
        let sha = sha256_hex(content);
        let support = b"format source";
        let support_sha = sha256_hex(support);
        let corpus = dir.join("corpus");
        fs::create_dir_all(&corpus)?;
        fs::write(corpus.join("plain.tex"), support)?;
        let manifest = dir.join("manifest.txt");
        let locator_lines = urls
            .iter()
            .map(|url| format!("url {url}\n"))
            .collect::<String>();
        fs::write(
            &manifest,
            format!(
                "\
support plain.tex
url https://example.com/plain.tex
sha256 {support_sha}
license MIT
redistributable true
notes test format source

doc sample.tex
{locator_lines}\
sha256 {sha}
license MIT
redistributable true
format_source plain.tex
expected_ref_dvi_sha256 {sha}
notes test fixture
"
            ),
        )?;
        Ok(manifest)
    }

    fn serve_once(body: &'static [u8]) -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });
        Ok(format!("http://{addr}/sample.tex"))
    }
}
