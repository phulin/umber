//! Native file staging for the in-process classic BibTeX command.

use std::path::{Path, PathBuf};

use bib_engine::{ClassicBibCommand, FileProvisioner, ResolvedFile, VfsLimits};
use tex_state::World;

use crate::bib::BibCliError;

pub fn run(args: impl Iterator<Item = String>) -> Result<(), BibCliError> {
    let args = args.collect::<Vec<_>>();
    let command = ClassicBibCommand::parse(&args).map_err(|error| {
        let output = error.output();
        BibCliError::command(output.status(), output.terminal())
    })?;
    let physical_input = physical_input(&args)?;
    let base = physical_input.parent().unwrap_or_else(|| Path::new("."));
    let mut world = World::real();
    let mut files = FileProvisioner::new(VfsLimits::default())
        .map_err(|error| BibCliError::message(1, error.to_string()))?;
    files
        .register_user(
            command.aux_path().clone(),
            world
                .read_file(&physical_input)
                .map_err(BibCliError::external)?
                .bytes()
                .to_vec(),
        )
        .map_err(|error| BibCliError::message(1, error.to_string()))?;
    let output = command.execute_provisioned(&mut files, |request| {
        let source = base.join(request.key().name());
        let bytes = world.read_file(source).ok()?.bytes().to_vec();
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/bibtex/{}", request.key().name()),
            bytes,
            expected_digest: None,
        })
    });
    if let Some(result) = output.result() {
        let artifacts = result
            .files()
            .chain(result.partial_files())
            .map(|file| {
                (
                    base.join(
                        file.path()
                            .as_path()
                            .file_name()
                            .expect("classic artifacts have file names"),
                    ),
                    file.bytes().to_vec(),
                )
            })
            .collect();
        world
            .publish_files(artifacts)
            .map_err(BibCliError::external)?;
    }
    if output.status().code() != 0 {
        return Err(BibCliError::command(output.status(), output.terminal()));
    }
    print!("{}", String::from_utf8_lossy(output.terminal()));
    Ok(())
}

fn physical_input(args: &[String]) -> Result<PathBuf, BibCliError> {
    let input = args
        .first()
        .ok_or_else(|| BibCliError::message(2, "missing AUX job name"))?;
    let path = PathBuf::from(input);
    Ok(
        if path.extension().is_some_and(|extension| extension == "aux") {
            path
        } else {
            path.with_extension("aux")
        },
    )
}
