use std::fmt;
use std::path::{Path, PathBuf};

use bib_engine::{BibCommand, BibExitStatus, FileProvisioner, ResolvedFile, VfsLimits};
use tex_state::World;

pub fn run(args: impl Iterator<Item = String>) -> Result<(), BibCliError> {
    let args = args.collect::<Vec<_>>();
    let command = BibCommand::parse(&args).map_err(|error| {
        let output = error.output();
        BibCliError::command(output.status(), output.terminal())
    })?;
    let physical_input = physical_input(&args)?;
    let base = physical_input.parent().unwrap_or_else(|| Path::new("."));
    let mut world = World::real();
    let input_bytes = world
        .read_file(&physical_input)
        .map_err(BibCliError::external)?
        .bytes()
        .to_vec();
    let mut files = FileProvisioner::new(VfsLimits::default())
        .map_err(|error| BibCliError::message(1, error.to_string()))?;
    files
        .register_user(command.input().clone(), input_bytes)
        .map_err(|error| BibCliError::message(1, error.to_string()))?;
    if let Some(control) = command.tool_control() {
        files
            .register_user(command.job().control_path().clone(), control)
            .map_err(|error| BibCliError::message(1, error.to_string()))?;
    }

    let output = command.execute_provisioned(&mut files, |request| {
        let logical = request.original_name();
        if logical.contains("://") {
            return None;
        }
        let logical_host = logical.strip_prefix("/job/").unwrap_or(logical);
        let source = if Path::new(logical_host).is_absolute() {
            PathBuf::from(logical_host)
        } else {
            base.join(logical_host)
        };
        let bytes = world.read_file(source).ok()?.bytes().to_vec();
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/bib/{}", request.key().name()),
            bytes,
            expected_digest: None,
        })
    });
    if output.status() != BibExitStatus::Success {
        return Err(BibCliError::command(output.status(), output.terminal()));
    }

    let explicit_output = option_value(&args, "--output-file");
    let mut published = Vec::new();
    if let Some(result) = output.result() {
        for generated in result.files() {
            let destination = explicit_output.clone().unwrap_or_else(|| {
                base.join(
                    generated
                        .path()
                        .as_path()
                        .file_name()
                        .expect("generated paths name a file"),
                )
            });
            published.push((destination, generated.bytes().to_vec()));
        }
    }
    if !output.log().is_empty() {
        let log = base.join(
            physical_input
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map_or_else(|| "bib.blg".to_owned(), |stem| format!("{stem}.blg")),
        );
        published.push((log, output.log().to_vec()));
    }
    world
        .publish_files(published)
        .map_err(BibCliError::external)?;
    print!("{}", String::from_utf8_lossy(output.terminal()));
    Ok(())
}

fn physical_input(args: &[String]) -> Result<PathBuf, BibCliError> {
    let mut input = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let name = argument
            .split_once('=')
            .map_or(argument.as_str(), |pair| pair.0);
        if matches!(
            name,
            "--configfile"
                | "--config-file"
                | "--output-file"
                | "--output-format"
                | "--output-encoding"
                | "--output-newline"
                | "--dot-include"
                | "-dot-include"
        ) && !argument.contains('=')
        {
            index += 2;
            continue;
        }
        if !argument.starts_with('-') {
            input = Some(PathBuf::from(argument));
        }
        index += 1;
    }
    input.ok_or_else(|| BibCliError::message(2, "missing bibliography input path"))
}

fn option_value(args: &[String], option: &str) -> Option<PathBuf> {
    for (index, argument) in args.iter().enumerate() {
        if let Some(value) = argument.strip_prefix(&format!("{option}=")) {
            return Some(PathBuf::from(value));
        }
        if argument == option {
            return args.get(index + 1).map(PathBuf::from);
        }
    }
    None
}

#[derive(Debug)]
pub struct BibCliError {
    status: u8,
    message: String,
}

impl BibCliError {
    fn command(status: BibExitStatus, terminal: &[u8]) -> Self {
        Self::message(
            status.code(),
            String::from_utf8_lossy(terminal).trim_end().to_owned(),
        )
    }

    fn external(error: impl fmt::Display) -> Self {
        Self::message(1, error.to_string())
    }

    fn message(status: u8, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub const fn exit_status(&self) -> u8 {
        self.status
    }
}

impl fmt::Display for BibCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BibCliError {}
