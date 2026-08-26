use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use umber::verify_distribution;
use umber_fetch::BlobStore;

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("distribution-verify: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut distribution = None;
    let mut distribution_ahash64 = None;
    let mut cache = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--distribution" => distribution = Some(required_value(&mut args, &argument)?),
            "--distribution-ahash64" => {
                distribution_ahash64 = Some(required_value(&mut args, &argument)?)
            }
            "--cache" => cache = Some(required_value(&mut args, &argument)?),
            "--help" | "-h" => {
                println!(
                    "usage: distribution-verify [--distribution PATH --distribution-ahash64 AHASH64] [--cache PATH]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    if distribution.is_none() && cache.is_none() {
        return Err("at least one of --distribution or --cache is required".to_owned());
    }
    match (distribution, distribution_ahash64) {
        (Some(path), Some(digest)) => {
            let report = verify_distribution(&PathBuf::from(path), &digest)
                .map_err(|error| error.to_string())?;
            println!(
                "distribution roots={} shards={} objects={} hashed_bytes={}",
                report.roots, report.shards, report.objects, report.hashed_bytes
            );
        }
        (Some(_), None) => {
            return Err("--distribution requires --distribution-ahash64".to_owned());
        }
        (None, Some(_)) => {
            return Err("--distribution-ahash64 requires --distribution".to_owned());
        }
        (None, None) => {}
    }
    if let Some(path) = cache {
        let report = BlobStore::new(path)
            .verify_all()
            .map_err(|error| error.to_string())?;
        println!(
            "cache blobs={} objects={} manifests={} other={} payload_bytes={}",
            report.blobs,
            report.object_blobs,
            report.manifest_blobs,
            report.other_blobs,
            report.payload_bytes
        );
    }
    Ok(())
}

fn required_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value after {option}"))
}
