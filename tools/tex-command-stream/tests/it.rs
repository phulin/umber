#[cfg(target_os = "linux")]
umber::register_format_worker_test_bootstrap!();

#[path = "it/command_semantic.rs"]
mod command_semantic;
#[path = "it/explicit_repository.rs"]
mod explicit_repository;
#[path = "it/repository_comparison.rs"]
mod repository_comparison;
