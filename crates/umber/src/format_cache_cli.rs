use std::path::{Path, PathBuf};

use tex_state::{World, WorldError};
use umber_fetch::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint,
};

const DISTRIBUTION_DOMAIN: &[u8] = b"umber.pinned-distribution.v1\0";

pub fn run(args: impl Iterator<Item = String>) -> Result<(), FormatCacheCliError> {
    let options = Options::parse(args)?;
    let identity = options.identity()?;
    let key = identity.key().hex();
    let store = options
        .cache_root
        .map_or_else(FormatCacheStore::from_environment, |root| {
            Ok(FormatCacheStore::new(root))
        })?;

    match options.action {
        Action::Restore { output } => match store.load(&identity)? {
            Some(format) => {
                let mut world = World::real();
                world.publish_files(vec![(output, format.into_bytes())])?;
                println!("hit");
                eprintln!("umber: generated format cache hit key={key}");
            }
            None => {
                println!("miss");
                eprintln!("umber: generated format cache miss key={key}");
            }
        },
        Action::Store { format } => {
            let bytes = World::real().read_file(&format)?.into_bytes();
            store.store(&identity, &bytes)?;
            println!("stored");
            eprintln!("umber: published generated format cache entry key={key}");
        }
    }
    Ok(())
}

#[derive(Debug)]
enum Action {
    Restore { output: PathBuf },
    Store { format: PathBuf },
}

#[derive(Debug)]
struct Options {
    action: Action,
    engine: FormatEngineMode,
    distribution: String,
    closure: PathBuf,
    source_lock: PathBuf,
    build_configuration: PathBuf,
    cache_root: Option<PathBuf>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, FormatCacheCliError> {
        let action_name = args.next().ok_or(FormatCacheCliError::Usage(
            "format-cache requires restore or store",
        ))?;
        let mut engine = None;
        let mut distribution = None;
        let mut closure = None;
        let mut source_lock = None;
        let mut build_configuration = None;
        let mut cache_root = None;
        let mut output = None;
        let mut format = None;

        while let Some(argument) = args.next() {
            let value = match argument.as_str() {
                "--engine"
                | "--distribution"
                | "--closure"
                | "--source-lock"
                | "--build-configuration"
                | "--cache-root"
                | "--format-out"
                | "--format" => args.next().ok_or(FormatCacheCliError::Usage(
                    "missing format-cache option value",
                ))?,
                _ => {
                    return Err(FormatCacheCliError::Usage(
                        "unknown option for format-cache",
                    ));
                }
            };
            match argument.as_str() {
                "--engine" => {
                    engine = Some(match value.as_str() {
                        "latex" => FormatEngineMode::Latex,
                        "pdflatex" => FormatEngineMode::PdfLatex,
                        _ => {
                            return Err(FormatCacheCliError::Usage(
                                "format-cache engine must be latex or pdflatex",
                            ));
                        }
                    });
                }
                "--distribution" => distribution = Some(value),
                "--closure" => closure = Some(PathBuf::from(value)),
                "--source-lock" => source_lock = Some(PathBuf::from(value)),
                "--build-configuration" => build_configuration = Some(PathBuf::from(value)),
                "--cache-root" => cache_root = Some(PathBuf::from(value)),
                "--format-out" => output = Some(PathBuf::from(value)),
                "--format" => format = Some(PathBuf::from(value)),
                _ => unreachable!("matched above"),
            }
        }

        let action = match action_name.as_str() {
            "restore" if format.is_none() => Action::Restore {
                output: required(output, "restore requires --format-out PATH")?,
            },
            "store" if output.is_none() => Action::Store {
                format: required(format, "store requires --format PATH")?,
            },
            "restore" => {
                return Err(FormatCacheCliError::Usage(
                    "restore does not accept --format",
                ));
            }
            "store" => {
                return Err(FormatCacheCliError::Usage(
                    "store does not accept --format-out",
                ));
            }
            _ => {
                return Err(FormatCacheCliError::Usage(
                    "format-cache action must be restore or store",
                ));
            }
        };
        Ok(Self {
            action,
            engine: required(engine, "format-cache requires --engine")?,
            distribution: required(distribution, "format-cache requires --distribution")?,
            closure: required(closure, "format-cache requires --closure")?,
            source_lock: required(source_lock, "format-cache requires --source-lock")?,
            build_configuration: required(
                build_configuration,
                "format-cache requires --build-configuration",
            )?,
            cache_root,
        })
    }

    fn identity(&self) -> Result<FormatCacheIdentity, FormatCacheCliError> {
        let mut world = World::real();
        let closure = read_identity_input(&mut world, &self.closure)?;
        let source_lock = read_identity_input(&mut world, &self.source_lock)?;
        let build_configuration = read_identity_input(&mut world, &self.build_configuration)?;
        let mut distribution =
            Vec::with_capacity(DISTRIBUTION_DOMAIN.len() + self.distribution.len());
        distribution.extend_from_slice(DISTRIBUTION_DOMAIN);
        distribution.extend_from_slice(self.distribution.as_bytes());
        let clock = world.job_clock();
        Ok(FormatCacheIdentity::current(
            self.engine,
            FormatFingerprint::sha256(&distribution),
            FormatFingerprint::sha256(&closure),
            FormatFingerprint::sha256(&source_lock),
            FormatCacheClock {
                time: clock.time,
                second: clock.second,
                day: clock.day,
                month: clock.month,
                year: clock.year,
            },
            FormatFingerprint::sha256(&build_configuration),
        ))
    }
}

fn required<T>(value: Option<T>, message: &'static str) -> Result<T, FormatCacheCliError> {
    value.ok_or(FormatCacheCliError::Usage(message))
}

fn read_identity_input(world: &mut World, path: &Path) -> Result<Vec<u8>, WorldError> {
    world.read_file(path).map(|content| content.into_bytes())
}

#[derive(Debug)]
pub enum FormatCacheCliError {
    Usage(&'static str),
    Cache(FormatCacheError),
    World(WorldError),
}

impl std::fmt::Display for FormatCacheCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Cache(error) => error.fmt(f),
            Self::World(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FormatCacheCliError {}

impl From<FormatCacheError> for FormatCacheCliError {
    fn from(value: FormatCacheError) -> Self {
        Self::Cache(value)
    }
}

impl From<WorldError> for FormatCacheCliError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}
