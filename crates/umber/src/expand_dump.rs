use std::path::Path;

use tex_expand::{ExpandError, semantic_token};
use tex_lex::{InputStack, LexError, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::meaning::Meaning;
use tex_state::{Universe, World, WorldError};

use crate::format_token;
use umber::{EngineSession, FileSessionResolvers};

pub fn expand_dump(path: &str) -> Result<(), ExpandDumpError> {
    let path = Path::new(path);
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    install_dump_primitives(&mut stores);

    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut resolvers = FileSessionResolvers::from_environment(path);
    let mut session = EngineSession::new(&mut input, &mut stores, resolvers.context());
    match dump(&mut session) {
        Ok(()) => Ok(()),
        Err(err) => {
            session.publish_input_summary();
            Err(err.render_with_provenance(session.stores()))
        }
    }
}

fn dump(session: &mut EngineSession<'_, '_>) -> Result<(), ExpandDumpError> {
    while let Some(token) = session.next_expanded_token()? {
        if session.try_execute_assignment(token)? {
            continue;
        }
        println!("{}", format_token(semantic_token(token), session.stores()));
    }
    Ok(())
}

fn install_dump_primitives(stores: &mut Universe) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    stores.intern("par");
    tex_exec::install_unexpandable_primitives(stores);

    tex_expand::install_expandable_primitives(stores);

    for name in [
        "def",
        "edef",
        "gdef",
        "xdef",
        "long",
        "outer",
        "protected",
        "global",
        "globaldefs",
        "let",
        "futurelet",
        "chardef",
        "catcode",
        "count",
        "dimen",
        "toks",
        "endlinechar",
        "escapechar",
    ] {
        stores.intern(name);
    }
}

#[derive(Debug)]
pub enum ExpandDumpError {
    World(WorldError),
    Exec(tex_exec::ExecError),
    Lex(LexError),
    Expand(ExpandError),
    Rendered(String),
}

impl std::fmt::Display for ExpandDumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
            Self::Rendered(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for ExpandDumpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World(err) => Some(err),
            Self::Exec(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
            Self::Rendered(_) => None,
        }
    }
}

impl From<WorldError> for ExpandDumpError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_exec::ExecError> for ExpandDumpError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Exec(value)
    }
}

impl From<LexError> for ExpandDumpError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ExpandError> for ExpandDumpError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl ExpandDumpError {
    fn render_with_provenance(self, stores: &Universe) -> Self {
        match self {
            Self::Exec(err) => {
                Self::Rendered(err.format_with_provenance(stores).trim_end().to_owned())
            }
            Self::Expand(err) => Self::Rendered(
                tex_state::ProvenanceResolver::new(stores)
                    .render_diagnostic_site(&err.to_string(), &err.diagnostic_site())
                    .trim_end()
                    .to_owned(),
            ),
            err => err,
        }
    }
}
