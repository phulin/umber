use crate::{
    ClassicStringPool, CompilationCache, CompileLimits, CompiledCommand, DiagnosticKind,
    Instruction, StringPoolLimit, StringPoolLimits, compile,
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

#[test]
fn classic_pool_deduplicates_empty_strings_and_preserves_identities() {
    let mut pool = ClassicStringPool::new(StringPoolLimits::unlimited());
    let empty = pool.intern("").expect("empty string");
    assert_eq!(pool.intern("").expect("same empty string"), empty);
    let title = pool.intern("title").expect("title");
    assert_eq!(pool.value(title), Some("title"));
    assert_eq!(pool.usage().strings(), 2);
    assert_eq!(pool.usage().characters(), 5);
}

#[test]
fn web2c_bootstrap_owns_the_reference_predefined_pool() {
    let pool = ClassicStringPool::web2c();
    assert_eq!(pool.usage().strings(), 81);
    assert_eq!(pool.usage().characters(), 470);
}

#[test]
fn classic_pool_enforces_charged_limits_after_deduplication() {
    let mut pool = ClassicStringPool::new(StringPoolLimits::new(1, 3));
    pool.intern("abc").expect("fits");
    assert_eq!(pool.intern("abc"), pool.intern("abc"));
    assert_eq!(pool.intern("d"), Err(StringPoolLimit::Strings));
    let mut characters = ClassicStringPool::new(StringPoolLimits::new(2, 3));
    assert_eq!(characters.intern("four"), Err(StringPoolLimit::Characters));
}

#[test]
fn compiler_pool_trace_covers_symbols_and_literals_without_double_charging() {
    let result = compile(
        b"ENTRY { title } {} {} MACRO { titlecase } { \"x\" } FUNCTION { emit } { \"x\" #7 } READ",
        CompileLimits::default(),
    );
    let style = result.program().expect("valid style");
    assert_eq!(style.compiler_pool_usage().strings(), 5);
    assert_eq!(style.compiler_pool_usage().characters(), 20);
}

#[test]
fn compiler_pool_trace_keeps_web2c_integer_and_implicit_function_names() {
    let result = compile(
        b"ENTRY { title } {} {} FUNCTION { emit } { #7 { skip$ } { skip$ } if$ } READ",
        CompileLimits::default(),
    );
    let style = result.program().expect("valid style");
    let mut pool = ClassicStringPool::web2c();
    style.apply_pool_trace(&mut pool);
    assert_eq!(pool.usage().strings(), 86);
    assert_eq!(pool.usage().characters(), 484);
}
