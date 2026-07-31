#[path = "it/cli.rs"]
mod cli;
#[path = "it/e2e_conformance.rs"]
mod e2e_conformance;
#[path = "it/effectful_replay.rs"]
mod effectful_replay;
#[path = "it/font_catalog.rs"]
mod font_catalog;
#[cfg(feature = "profiling-runner")]
#[path = "it/gentle_profile_cli.rs"]
mod gentle_profile_cli;
#[path = "it/pdf_parity.rs"]
mod pdf_parity;
#[path = "it/replay_identity.rs"]
mod replay_identity;

umber::register_format_worker_test_bootstrap!();
