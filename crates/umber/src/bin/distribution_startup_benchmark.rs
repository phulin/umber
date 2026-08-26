#![allow(
    clippy::disallowed_methods,
    reason = "this opt-in native benchmark constructs a hermetic fixture, launches child processes, and measures wall time"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use umber::cli_resource::{
    NativeCompileSession, NativeDistributionOwner, NativeRunOptions, ResolverTelemetry,
};
use umber::{EngineMode, OutputCapabilitySet};
use umber_fetch::{FetchCancellation, ObjectCache};
use umber_hash::{AHash64, HashDomain};

const SAMPLES: usize = 5;

#[derive(Clone, Copy, Debug, Default)]
struct Work {
    manifest_reads: u64,
    manifest_parses: u64,
    manifest_validations: u64,
    verified_manifest_hits: u64,
    shard_loads: u64,
    object_hashes: u64,
    object_cache_hits: u64,
}

impl Work {
    fn add(&mut self, telemetry: ResolverTelemetry) {
        self.manifest_reads = self.manifest_reads.saturating_add(telemetry.manifest_reads);
        self.manifest_parses = self
            .manifest_parses
            .saturating_add(telemetry.manifest_parses);
        self.manifest_validations = self
            .manifest_validations
            .saturating_add(telemetry.manifest_validations);
        self.verified_manifest_hits = self
            .verified_manifest_hits
            .saturating_add(telemetry.verified_manifest_hits);
        self.shard_loads = self.shard_loads.saturating_add(telemetry.shard_loads);
        self.object_hashes = self.object_hashes.saturating_add(telemetry.object_hashes);
        self.object_cache_hits = self
            .object_cache_hits
            .saturating_add(telemetry.object_cache_hits);
    }

    fn add_work(&mut self, other: Self) {
        self.manifest_reads = self.manifest_reads.saturating_add(other.manifest_reads);
        self.manifest_parses = self.manifest_parses.saturating_add(other.manifest_parses);
        self.manifest_validations = self
            .manifest_validations
            .saturating_add(other.manifest_validations);
        self.verified_manifest_hits = self
            .verified_manifest_hits
            .saturating_add(other.verified_manifest_hits);
        self.shard_loads = self.shard_loads.saturating_add(other.shard_loads);
        self.object_hashes = self.object_hashes.saturating_add(other.object_hashes);
        self.object_cache_hits = self
            .object_cache_hits
            .saturating_add(other.object_cache_hits);
    }

    fn parse(line: &str) -> Result<Self, String> {
        let fields = line
            .split_ascii_whitespace()
            .skip(1)
            .map(|field| {
                let (key, value) = field
                    .split_once('=')
                    .ok_or_else(|| format!("invalid child field {field}"))?;
                let value = value
                    .parse::<u64>()
                    .map_err(|_| format!("invalid child value {field}"))?;
                Ok((key, value))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let get = |key| {
            fields
                .get(key)
                .copied()
                .ok_or_else(|| format!("child result omitted {key}"))
        };
        Ok(Self {
            manifest_reads: get("manifest_reads")?,
            manifest_parses: get("manifest_parses")?,
            manifest_validations: get("manifest_validations")?,
            verified_manifest_hits: get("verified_manifest_hits")?,
            shard_loads: get("shard_loads")?,
            object_hashes: get("object_hashes")?,
            object_cache_hits: get("object_cache_hits")?,
        })
    }
}

struct Fixture {
    root: PathBuf,
    input: PathBuf,
    distribution: PathBuf,
    cache: PathBuf,
    manifest_digest: String,
}

fn main() -> Result<(), String> {
    let args = std::env::args_os().collect::<Vec<_>>();
    if args.get(1).is_some_and(|argument| argument == "--child") {
        return child(&args[2..]);
    }
    parent()
}

fn parent() -> Result<(), String> {
    let fixture = Fixture::create()?;
    let options = fixture.options();

    // Populate only the synthetic benchmark cache, then freeze its exact byte
    // inventory. Both measured routes start from these same verified bytes.
    let warm_owner =
        NativeDistributionOwner::with_cache(&options, ObjectCache::new(fixture.cache.clone()));
    let (expected_dvi, _) = compile_once(&options, &warm_owner)?;
    drop(warm_owner);
    let cache_before = inventory(&fixture.cache)?;

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let cold_started = Instant::now();
    let mut cold_work = Work::default();
    for _ in 0..SAMPLES {
        let output = Command::new(&executable)
            .arg("--child")
            .arg(&fixture.input)
            .arg(&fixture.distribution)
            .arg(&fixture.manifest_digest)
            .arg(&fixture.cache)
            .output()
            .map_err(|error| format!("launch cold child: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "cold child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let stdout = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        let line = stdout
            .lines()
            .find(|line| line.starts_with("CHILD "))
            .ok_or_else(|| format!("cold child emitted no result: {stdout}"))?;
        let (work, dvi_sha256) = line
            .rsplit_once(" dvi_sha256=")
            .ok_or_else(|| "cold child omitted DVI identity".to_owned())?;
        if dvi_sha256 != hex_digest(&expected_dvi) {
            return Err("cold child changed observable DVI bytes".to_owned());
        }
        cold_work.add_work(Work::parse(work)?);
    }
    let cold_elapsed = cold_started.elapsed();

    let shared_owner =
        NativeDistributionOwner::with_cache(&options, ObjectCache::new(fixture.cache.clone()));
    let shared_started = Instant::now();
    let mut shared_work = Work::default();
    for _ in 0..SAMPLES {
        let (dvi, telemetry) = compile_once(&options, &shared_owner)?;
        if dvi != expected_dvi {
            return Err("same-process session changed observable DVI bytes".to_owned());
        }
        shared_work.add(telemetry);
    }
    let shared_elapsed = shared_started.elapsed();
    let cache_after = inventory(&fixture.cache)?;

    if cache_before != cache_after {
        return Err("measured sessions changed the verified cache byte inventory".to_owned());
    }
    if shared_work.manifest_reads >= cold_work.manifest_reads
        || shared_work.manifest_parses >= cold_work.manifest_parses
        || shared_work.manifest_validations >= cold_work.manifest_validations
        || shared_work.shard_loads >= cold_work.shard_loads
    {
        return Err("shared owner did not reduce verified manifest startup work".to_owned());
    }
    if cold_work.object_hashes != SAMPLES as u64 || shared_work.object_hashes != SAMPLES as u64 {
        return Err(
            "live distribution object hashes were not proportional to requested resources"
                .to_owned(),
        );
    }

    println!(
        "distribution_startup schema=1 samples={SAMPLES} cache_files={} cache_bytes={} cache_sha256={} dvi_sha256={}",
        cache_before.files,
        cache_before.bytes,
        cache_before.sha256,
        hex_digest(&expected_dvi),
    );
    print_result("cold_process", cold_elapsed, cold_work);
    print_result("same_process_shared_owner", shared_elapsed, shared_work);
    println!(
        "zero_loss=true cache_bytes_unchanged=true object_hashes_proportional=true manifest_read_reduction={} manifest_parse_reduction={} manifest_validation_reduction={} shard_load_reduction={}",
        cold_work.manifest_reads - shared_work.manifest_reads,
        cold_work.manifest_parses - shared_work.manifest_parses,
        cold_work.manifest_validations - shared_work.manifest_validations,
        cold_work.shard_loads - shared_work.shard_loads,
    );

    fixture.remove()?;
    Ok(())
}

fn child(args: &[std::ffi::OsString]) -> Result<(), String> {
    let [input, distribution, manifest_digest, cache] = args else {
        return Err("child requires INPUT DISTRIBUTION MANIFEST_AHASH64 CACHE".to_owned());
    };
    let options = options(
        PathBuf::from(input),
        PathBuf::from(distribution),
        manifest_digest
            .to_str()
            .ok_or_else(|| "manifest digest is not UTF-8".to_owned())?
            .to_owned(),
    );
    let owner =
        NativeDistributionOwner::with_cache(&options, ObjectCache::new(PathBuf::from(cache)));
    let (dvi, telemetry) = compile_once(&options, &owner)?;
    let work = Work::default_with(telemetry);
    print!("CHILD ");
    print_work(work);
    println!(" dvi_sha256={}", hex_digest(&dvi));
    Ok(())
}

impl Work {
    fn default_with(telemetry: ResolverTelemetry) -> Self {
        let mut work = Self::default();
        work.add(telemetry);
        work
    }
}

fn compile_once(
    options: &NativeRunOptions,
    owner: &NativeDistributionOwner,
) -> Result<(Vec<u8>, ResolverTelemetry), String> {
    let cancellation = FetchCancellation::new();
    let reachability_store = tex_incr::new_reachability_store();
    let mut session =
        NativeCompileSession::new_with_owners(options, &cancellation, owner, &reachability_store)
            .map_err(|error| error.to_string())?;
    let output = session
        .compile(&cancellation)
        .map_err(|error| error.to_string())?;
    Ok((output.dvi, session.host_telemetry().resolver))
}

fn print_result(label: &str, elapsed: Duration, work: Work) {
    print!("{label} elapsed_us={} ", elapsed.as_micros());
    print_work(work);
    println!();
}

fn print_work(work: Work) {
    print!(
        "manifest_reads={} manifest_parses={} manifest_validations={} verified_manifest_hits={} shard_loads={} object_hashes={} object_cache_hits={}",
        work.manifest_reads,
        work.manifest_parses,
        work.manifest_validations,
        work.verified_manifest_hits,
        work.shard_loads,
        work.object_hashes,
        work.object_cache_hits,
    );
}

impl Fixture {
    fn create() -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "umber-distribution-startup-benchmark-{}",
            std::process::id()
        ));
        if root.exists() {
            return Err(format!("refusing to replace {}", root.display()));
        }
        let distribution = root.join("distribution");
        let objects = distribution.join("objects");
        let cache = root.join("cache");
        fs::create_dir_all(&objects).map_err(|error| error.to_string())?;
        let package = b"\\def\\benchmarkheight{1pt}";
        let package_digest = distribution_digest(package);
        fs::write(
            objects.join(format!("ahash64-v1-{package_digest}")),
            package,
        )
        .map_err(|error| error.to_string())?;
        let unrequested = b"unrequested valid distribution object";
        let unrequested_digest = distribution_digest(unrequested);
        fs::write(
            objects.join(format!("ahash64-v1-{unrequested_digest}")),
            unrequested,
        )
        .map_err(|error| error.to_string())?;
        let shard = concat!(
            r#"{"schema":3,"distribution":"startup-benchmark","index":0,"files":{"tex:benchmark.sty":{"virtualPath":"/texlive/tex/benchmark.sty","object":"ahash64-v1-$DIGEST","ahash64":"$DIGEST","bytes":$BYTES,"dependencies":[{"key":"tex:unrequested.sty","virtualPath":"/texlive/tex/unrequested.sty","object":"ahash64-v1-$UNREQUESTED_DIGEST","ahash64":"$UNREQUESTED_DIGEST","bytes":$UNREQUESTED_BYTES}]},"tex:unrequested.sty":{"virtualPath":"/texlive/tex/unrequested.sty","object":"ahash64-v1-$UNREQUESTED_DIGEST","ahash64":"$UNREQUESTED_DIGEST","bytes":$UNREQUESTED_BYTES}}}"#,
            "\n"
        )
        .replace("$DIGEST", &package_digest)
        .replace("$BYTES", &package.len().to_string())
        .replace("$UNREQUESTED_DIGEST", &unrequested_digest)
        .replace("$UNREQUESTED_BYTES", &unrequested.len().to_string());
        let shard_digest = distribution_digest(shard.as_bytes());
        fs::write(objects.join(format!("ahash64-v1-{shard_digest}")), shard)
            .map_err(|error| error.to_string())?;
        let manifest = format!(
            "{{\"schema\":5,\"distribution\":\"startup-benchmark\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
        );
        let manifest_digest = distribution_digest(manifest.as_bytes());
        fs::write(distribution.join("manifest-v5.json"), manifest)
            .map_err(|error| error.to_string())?;
        let input = root.join("main.tex");
        fs::write(
            &input,
            b"\\input benchmark.sty \\shipout\\vbox{\\hrule height\\benchmarkheight}\\end",
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            root,
            input,
            distribution,
            cache,
            manifest_digest,
        })
    }

    fn options(&self) -> NativeRunOptions {
        options(
            self.input.clone(),
            self.distribution.clone(),
            self.manifest_digest.clone(),
        )
    }

    fn remove(self) -> Result<(), String> {
        fs::remove_dir_all(&self.root)
            .map_err(|error| format!("remove owned fixture {}: {error}", self.root.display()))
    }
}

fn options(input: PathBuf, distribution: PathBuf, manifest_digest: String) -> NativeRunOptions {
    NativeRunOptions {
        input,
        format: None,
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution: Some(distribution.to_string_lossy().into_owned()),
        distribution_ahash64: Some(manifest_digest),
        offline: true,
        expansion_fuel: None,
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Inventory {
    files: u64,
    bytes: u64,
    sha256: String,
}

fn inventory(root: &Path) -> Result<Inventory, String> {
    fn visit(
        base: &Path,
        directory: &Path,
        entries: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), String> {
        let mut children = fs::read_dir(directory)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let file_type = child.file_type().map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                visit(base, &path, entries)?;
            } else if file_type.is_file() {
                entries.push((
                    path.strip_prefix(base)
                        .map_err(|error| error.to_string())?
                        .to_owned(),
                    fs::read(&path).map_err(|error| error.to_string())?,
                ));
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    for (path, content) in &entries {
        let path = path.to_string_lossy();
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((content.len() as u64).to_le_bytes());
        digest.update(content);
        bytes = bytes.saturating_add(content.len() as u64);
    }
    Ok(Inventory {
        files: entries.len() as u64,
        bytes,
        sha256: hex_bytes(&digest.finalize()),
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn distribution_digest(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
