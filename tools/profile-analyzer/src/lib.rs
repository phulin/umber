mod analyze;
mod model;
mod symbols;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

pub use analyze::{AnalysisOptions, Entry, Report};

use analyze::analyze;
use model::{Profile, SymbolSidecar};
use symbols::Symbolizer;

pub fn analyze_profile(
    profile_path: &Path,
    symbols_path: Option<&Path>,
    options: &AnalysisOptions,
) -> Result<(Report, Option<PathBuf>), String> {
    let profile: Profile = read_json(profile_path)?;
    let symbols_path = symbols_path
        .map(Path::to_owned)
        .or_else(|| discover_symbols(profile_path));
    let sidecar = symbols_path
        .as_deref()
        .map(read_json::<SymbolSidecar>)
        .transpose()?;
    let symbolizer = sidecar.map(Symbolizer::new);
    let report = analyze(&profile, symbolizer.as_ref(), options)?;
    Ok((report, symbols_path))
}

#[allow(clippy::disallowed_methods)] // Read-only host tooling; no engine state observes this I/O.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut reader: Box<dyn Read> =
        if path.extension().and_then(|value| value.to_str()) == Some("gz") {
            Box::new(GzDecoder::new(file))
        } else {
            Box::new(file)
        };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn discover_symbols(profile: &Path) -> Option<PathBuf> {
    let mut candidate = profile.to_owned();
    candidate.set_extension("syms.json");
    candidate.is_file().then_some(candidate)
}
