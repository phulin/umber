use tex_command::{CharacterMode, CommandDialect, CommandProfile, CommandState};

fn main() {
    let mut profile = CommandProfile::TEX82;
    profile.dialect = CommandDialect::Pdftex14027;
    profile.characters = CharacterMode::UnicodeExtended;

    let mut state = CommandState::new(CommandProfile::TEX82);
    state.expansion.profile = CommandProfile::unicode_extended(CommandDialect::Tex82);
}
