use crate::{
    CompilationCache, CompileLimits, CompiledCommand, DiagnosticKind, Instruction, compile,
};

const VALID: &[u8] = br#"
ENTRY { author title } { seen } { label }
INTEGERS { count }
STRINGS { output }
MACRO { hello } { "world" }
FUNCTION { emit } { "x" write$ }
READ
EXECUTE { emit }
ITERATE { emit }
REVERSE { emit }
SORT
"#;

#[test]
fn compiles_all_top_level_commands() {
    let result = compile(VALID, CompileLimits::default());
    let style = result.program().expect("valid style");
    assert_eq!(
        style.commands(),
        &[
            CompiledCommand::Read,
            CompiledCommand::Execute(crate::FunctionId(0)),
            CompiledCommand::Iterate(crate::FunctionId(0)),
            CompiledCommand::Reverse(crate::FunctionId(0)),
            CompiledCommand::Sort
        ]
    );
    assert!(matches!(
        style.functions()[0].instructions(),
        [
            Instruction::PushString(_),
            Instruction::Builtin(crate::Builtin::Write)
        ]
    ));
}

#[test]
fn compiles_committed_smoke_style() {
    let source = include_bytes!("../../../tests/corpus/bibtex/cases/smoke/smoke.bst");
    let result = compile(source, CompileLimits::default());
    assert!(result.is_success(), "{:?}", result.diagnostics());
}

#[test]
fn compiles_imported_standard_styles() {
    for source in [
        include_bytes!("../../../tests/corpus/bibtex/styles/plain.bst").as_slice(),
        include_bytes!("../../../tests/corpus/bibtex/styles/apalike.bst").as_slice(),
    ] {
        let result = compile(source, CompileLimits::default());
        assert!(result.is_success(), "{:?}", result.diagnostics());
    }
}

#[test]
fn recovery_reaches_later_command() {
    let source = b"ENTRY { title } { } { }\nBOGUS { broken }\nREAD\nSORT\n";
    let result = compile(source, CompileLimits::default());
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::UnknownCommand)
    );
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.kind() != DiagnosticKind::Phase)
    );
}

#[test]
fn phase_recovery_preserves_the_next_command() {
    let source = b"READ\nENTRY { title } { } { }\nREAD\nSORT\n";
    let result = compile(source, CompileLimits::default());
    assert_eq!(
        result
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.kind() == DiagnosticKind::Phase)
            .count(),
        1
    );
}

#[test]
fn declaration_before_use_and_self_recursion_are_diagnostics() {
    let source =
        b"ENTRY { } { } { }\nFUNCTION { first } { later first }\nFUNCTION { later } { }\nREAD\n";
    let result = compile(source, CompileLimits::default());
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::UnknownSymbol)
    );
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::IllegalRecursion)
    );
}

#[test]
fn compiles_signed_literals_and_quoted_builtins() {
    let source =
        b"ENTRY { } { } { }\nFUNCTION { control } { #-1 'skip$ if$ }\nREAD\nEXECUTE { control }\n";
    let result = compile(source, CompileLimits::default());
    let style = result.program().expect("standard BST syntax");
    assert!(matches!(
        style.functions()[0].instructions(),
        [
            Instruction::PushInteger(-1),
            Instruction::PushFunction(_),
            Instruction::Builtin(crate::Builtin::If)
        ]
    ));
}

#[test]
fn cache_hit_revalidates_active_limits() {
    let mut cache = CompilationCache::new(2, 1024 * 1024);
    assert!(cache.compile(VALID, CompileLimits::default()).is_success());
    assert!(
        cache
            .compile(VALID, CompileLimits::default())
            .stats()
            .cache_hit
    );
    let limits = CompileLimits {
        instructions: 1,
        ..CompileLimits::default()
    };
    let result = cache.compile(VALID, limits);
    assert!(!result.is_success());
    assert!(!result.stats().cache_hit);
}

#[test]
fn arbitrary_bytes_terminate_under_limits() {
    let limits = CompileLimits {
        bytes: 4096,
        tokens: 128,
        nesting: 8,
        work: 512,
        ..CompileLimits::default()
    };
    for seed in 0_u8..=255 {
        let bytes = vec![seed; 1024];
        let _ = compile(&bytes, limits);
    }
}
