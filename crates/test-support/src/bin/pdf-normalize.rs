//! Host-only command for projecting a PDF through the independent Hayro probe.

#![allow(clippy::disallowed_methods)]

use std::env;
use std::fs;
use std::io::{self, Write};

use anyhow::{Context, Result};
use test_support::pdf::normalize_structure;

fn main() -> Result<()> {
    let path = env::args_os().nth(1).context("usage: pdf-normalize PATH")?;
    anyhow::ensure!(env::args_os().nth(2).is_none(), "usage: pdf-normalize PATH");
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read PDF {}", path.to_string_lossy()))?;
    let normalized = normalize_structure(&bytes)
        .with_context(|| format!("failed to normalize PDF {}", path.to_string_lossy()))?;
    io::stdout()
        .write_all(normalized.as_bytes())
        .context("failed to write normalized PDF projection")
}
