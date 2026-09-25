//! Compare existing PDF files through the bounded Hayro semantic projection.

#![allow(clippy::disallowed_methods)] // Host-side corpus tool reads explicit paths.

use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_support::pdf_compare::{MAX_PDF_BYTES, PdfProjection, first_difference, hex, project_pdf};

const SCHEMA: &str = "umber-pdf-compare-v2";
const CRITERION: &str = "hayro-corpus-graph-content-v2";

struct Side {
    bytes: usize,
    sha256: String,
    projection: PdfProjection,
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        println!("{SCHEMA}");
        return ExitCode::SUCCESS;
    }
    let result = if args.len() == 2 {
        compare(Path::new(&args[0]), Path::new(&args[1]))
    } else {
        Err(anyhow::anyhow!(
            "usage: pdf-compare REFERENCE.pdf UMBER.pdf"
        ))
    };
    match result {
        Ok((receipt, exit)) => {
            println!("{receipt}");
            ExitCode::from(exit)
        }
        Err(error) => {
            println!(
                "{}",
                json!({
                    "schema": SCHEMA,
                    "criterion": CRITERION,
                    "status": "error",
                    "error": format!("{error:#}"),
                })
            );
            ExitCode::from(2)
        }
    }
}

fn compare(reference_path: &Path, umber_path: &Path) -> Result<(Value, u8)> {
    let reference = read_side(reference_path).context("reference PDF")?;
    let umber = read_side(umber_path).context("Umber PDF")?;
    let first = first_difference(&reference.projection.text, &umber.projection.text);
    let equal = first.is_none();
    let difference = first.map(
        |(line, reference, umber)| json!({"line": line, "reference": reference, "umber": umber}),
    );
    Ok((
        json!({
            "schema": SCHEMA,
            "criterion": CRITERION,
            "status": if equal { "equal" } else { "different" },
            "reference": side_receipt(&reference),
            "umber": side_receipt(&umber),
            "decoded_content_bytes_equal": reference.projection.decoded_content_sha256
                == umber.projection.decoded_content_sha256,
            "first_difference": difference,
        }),
        if equal { 0 } else { 1 },
    ))
}

fn side_receipt(side: &Side) -> Value {
    json!({
        "bytes": side.bytes,
        "sha256": side.sha256,
        "pages": side.projection.pages,
        "projection_sha256": side.projection.sha256,
        "decoded_content_sha256": side.projection.decoded_content_sha256,
    })
}

fn read_side(path: &Path) -> Result<Side> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if metadata.len() > MAX_PDF_BYTES as u64 {
        bail!(
            "{} exceeds {MAX_PDF_BYTES} byte input limit",
            path.display()
        );
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_PDF_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    if bytes.len() > MAX_PDF_BYTES {
        bail!(
            "{} exceeds {MAX_PDF_BYTES} byte input limit",
            path.display()
        );
    }
    let sha256 = hex(&Sha256::digest(&bytes));
    let projection = project_pdf(&bytes).with_context(|| format!("project {}", path.display()))?;
    Ok(Side {
        bytes: bytes.len(),
        sha256,
        projection,
    })
}
