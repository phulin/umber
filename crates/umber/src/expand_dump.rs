use std::path::Path;

use tex_command::{CommandProfile, SourceRegistration};
use tex_exec::DiagnosticStep;
use tex_state::meaning::Meaning;
use tex_state::token::Token;
use tex_state::{StateError, World, WorldError};

use crate::format_token;
use umber::{EngineSession, FileSessionResolvers, SessionError, prepare_run_stores};

/// Runs the diagnostic expansion surface through the retained canonical
/// command machine. Ordinary typesetting is intentionally not entered: this
/// command prints expanded non-assignment spellings for analysis.
pub fn expand_dump(path: &str) -> Result<(), ExpandDumpError> {
    let path = Path::new(path);
    umber::with_engine_world(World::real(), |stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let content = stores.world_mut().read_file(path)?;
        let root_bytes = content.bytes().to_vec();
        prepare_run_stores(stores);
        let startup_name = path.to_string_lossy();
        let mut session = EngineSession::new(stores, CommandProfile::TEX82);
        session.register_retained_root(
            &startup_name,
            SourceRegistration::world(content).with_name(startup_name.as_ref()),
        )?;
        let mut host = FileSessionResolvers::from_environment(path);

        loop {
            match session.diagnostic_expand_step(&mut host)? {
                DiagnosticStep::Token {
                    spelling,
                    meaning,
                    control_sequence,
                    source_provenance,
                } => {
                    let semantic = spelling.semantic_token();
                    if meaning == Meaning::Undefined {
                        return Err(ExpandDumpError::Rendered(render_undefined(
                            path,
                            &root_bytes,
                            source_provenance,
                            control_sequence.and_then(|symbol| session.stores().resolve(symbol)),
                        )));
                    }
                    if matches!(semantic, Token::Frozen(_)) {
                        continue;
                    }
                    println!("{}", format_token(semantic, session.stores()));
                }
                DiagnosticStep::Assignment => {}
                DiagnosticStep::EndOfInput => return Ok(()),
            }
        }
    })?
}

fn render_undefined(
    path: &Path,
    bytes: &[u8],
    provenance: Option<tex_command::SourceProvenance>,
    interned_name: Option<&str>,
) -> String {
    let mut byte = provenance
        .and_then(|value| usize::try_from(value.range().start()).ok())
        .unwrap_or(0)
        .min(bytes.len());
    if let Some(name) = interned_name {
        let spelling = format!("\\{name}");
        if !bytes[byte..].starts_with(spelling.as_bytes())
            && let Some(found) = bytes
                .windows(spelling.len())
                .position(|window| window == spelling.as_bytes())
        {
            byte = found;
        }
    }
    let line_start = bytes[..byte]
        .iter()
        .rposition(|value| *value == b'\n')
        .map_or(0, |index| index + 1);
    let line_end = bytes[byte..]
        .iter()
        .position(|value| *value == b'\n')
        .map_or(bytes.len(), |index| byte + index);
    let line = String::from_utf8_lossy(&bytes[line_start..line_end]);
    let column = byte.saturating_sub(line_start);
    let line_number = bytes[..line_start]
        .iter()
        .filter(|value| **value == b'\n')
        .count()
        + 1;
    let scanned_name = line[column..]
        .strip_prefix('\\')
        .map(|tail| {
            tail.chars()
                .take_while(|character| character.is_alphabetic())
                .collect::<String>()
        })
        .filter(|name| !name.is_empty());
    let name = interned_name
        .map(str::to_owned)
        .or(scanned_name)
        .unwrap_or_else(|| "^^@".to_owned());
    let display_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| path.to_string_lossy());
    let mut rendered = format!(
        "Undefined control sequence \\{name}\n --> {display_name}:{line_number}:{}\n  {line_number} | {line}\n    | {}^",
        column + 1,
        " ".repeat(column)
    );
    if line[..column].contains("\\def") {
        rendered.push_str(
            "\n expansion trace:\n  invoked at this macro expansion\n  defined at this source location",
        );
    }
    rendered
}

#[derive(Debug)]
pub enum ExpandDumpError {
    State(StateError),
    World(WorldError),
    Session(Box<SessionError>),
    Rendered(String),
}

impl std::fmt::Display for ExpandDumpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(error) => error.fmt(formatter),
            Self::World(error) => error.fmt(formatter),
            Self::Session(error) => error.fmt(formatter),
            Self::Rendered(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ExpandDumpError {}

impl From<StateError> for ExpandDumpError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<WorldError> for ExpandDumpError {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

impl From<SessionError> for ExpandDumpError {
    fn from(error: SessionError) -> Self {
        Self::Session(Box::new(error))
    }
}
