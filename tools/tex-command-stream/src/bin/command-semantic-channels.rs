//! Derives the per-channel contract for every committed minifixture.
//!
//! This is the regeneration path for the `command-semantic` corpus, invoked by
//! `scripts/regen-fixtures.sh --area command-semantic`. It runs exactly the
//! code the gate runs -- `tex_command_stream::semantic` -- so a regenerated
//! contract cannot describe a run the gate would not reproduce.
//!
//! It writes each case's `channels` block into its domain manifest and each
//! nonempty stream channel into `expected/<case-id>.<channel>`. Bytes it
//! derives from this implementation are recorded with the `umber-baseline`
//! authority: they pin the channel against silent drift, and they are still
//! owed an oracle adjudication.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only regeneration entry point rewrites its committed corpus"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use tex_command_stream::semantic::{
    CapturedChannels, ChannelPolicy, DeclaredCase, STREAM_CHANNELS, channel_file, execute,
    load_suite_with, repository_root,
};

fn main() -> ExitCode {
    match run() {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("command-semantic-channels: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let cases = load_suite_with(ChannelPolicy::Deriving)?;
    let mut derived: BTreeMap<String, BTreeMap<String, Derived>> = BTreeMap::new();
    let mut unrunnable = Vec::new();

    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let source = fs::read(declared.domain_dir.join(&declared.case.source))
            .map_err(|error| format!("{label}: source read: {error}"))?;
        match execute(&source, &declared.case) {
            Ok(run) => {
                let captured = CapturedChannels::capture(&run);
                write_channel_files(declared, &captured)?;
                derived
                    .entry(declared.domain.clone())
                    .or_default()
                    .insert(declared.case.id.clone(), Derived::new(declared, &captured));
            }
            // A case whose run fails has no channels to record. It is already
            // an `xfail` on its projection; recording an invented contract
            // here would let the failure read as covered.
            Err(error) => unrunnable.push(format!("{label}: {error}")),
        }
    }

    let mut rewritten = 0;
    for (domain, cases) in &derived {
        let manifest = repository_root()
            .join("tests/corpus/command-semantic")
            .join(domain)
            .join("manifest.json");
        rewrite_manifest(&manifest, cases)?;
        rewritten += 1;
    }

    let mut summary = format!(
        "derived channel contracts for {} cases across {rewritten} domains",
        derived.values().map(BTreeMap::len).sum::<usize>()
    );
    if !unrunnable.is_empty() {
        summary.push_str(&format!(
            "\n{} case(s) do not run and carry no channel contract:\n  {}",
            unrunnable.len(),
            unrunnable.join("\n  ")
        ));
    }
    Ok(summary)
}

struct Derived {
    events: usize,
    status: String,
    streams: [(String, bool); 4],
}

impl Derived {
    fn new(declared: &DeclaredCase, captured: &CapturedChannels) -> Self {
        let mut streams = [const { (String::new(), false) }; 4];
        for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
            let existing = declared
                .case
                .channels
                .as_ref()
                .map(|channels| channels.stream(channel));
            let bug = match existing {
                Some(tex_command_stream::semantic::StreamDisposition::Xfail { bug, .. }) => {
                    Some(bug.clone())
                }
                _ => None,
            };
            streams[index] = (
                bug.unwrap_or_default(),
                !captured.stream(channel).is_empty(),
            );
        }
        Self {
            events: captured.events,
            status: captured.status.clone(),
            streams,
        }
    }

    /// Renders the block in the exact shape `dprint` produces, so a
    /// regeneration run leaves the formatter with nothing to do. Emitting a
    /// single line instead would make every regeneration dirty the tree twice
    /// -- once here and once under `scripts/check.sh` -- and the second
    /// rewrite would silently defeat this tool's own block replacement.
    fn json(&self, indent: &str) -> String {
        let inner = format!("{indent}  ");
        let mut fields = vec![
            format!("{inner}\"events\": {}", self.events),
            format!("{inner}\"status\": {}", json_string(&self.status)),
        ];
        for (index, channel) in STREAM_CHANNELS.into_iter().enumerate() {
            let (bug, present) = &self.streams[index];
            let disposition = if !*present {
                "{ \"kind\": \"empty\" }".to_owned()
            } else if bug.is_empty() {
                "{ \"kind\": \"file\", \"authority\": \"umber-baseline\" }".to_owned()
            } else {
                format!(
                    "{{ \"kind\": \"xfail\", \"bug\": {}, \"authority\": \"umber-baseline\" }}",
                    json_string(bug)
                )
            };
            fields.push(format!("{inner}\"{}\": {disposition}", channel.name()));
        }
        format!("{{\n{}\n{indent}}}", fields.join(",\n"))
    }
}

fn write_channel_files(declared: &DeclaredCase, captured: &CapturedChannels) -> Result<(), String> {
    for channel in STREAM_CHANNELS {
        let path = channel_file(&declared.domain_dir, &declared.case.id, channel);
        let content = captured.stream(channel);
        if content.is_empty() {
            if path.exists() {
                fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            }
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&path, content).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

/// Inserts each derived `channels` block into the manifest immediately *before*
/// the case's `expectation` key, preserving every other byte of the file.
///
/// A structural rewrite through `serde_json` would reorder keys and drop the
/// hand-authored formatting the corpus is reviewed in, so the edit is textual.
/// It anchors before `expectation` rather than after because an `xfail`
/// expectation spans several lines: inserting before a key needs no knowledge
/// of where its value ends, while inserting after would have to parse it.
fn rewrite_manifest(path: &Path, cases: &BTreeMap<String, Derived>) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut output = String::with_capacity(text.len());
    let mut current: Option<&Derived> = None;
    let mut applied = 0;

    let mut skipping_channels = 0i32;
    for line in text.lines() {
        let trimmed = line.trim_start();

        // Drop any previously derived block, which the formatter may have
        // expanded across several lines. Counting braces rather than matching
        // one line is what makes the tool re-runnable on its own output.
        if skipping_channels > 0 {
            skipping_channels += braces(line);
            continue;
        }
        if trimmed.starts_with("\"channels\":") {
            skipping_channels = braces(line);
            continue;
        }

        if let Some(id) = trimmed
            .strip_prefix("\"id\": \"")
            .and_then(|rest| rest.split('"').next())
        {
            current = cases.get(id);
        }
        if let Some(derived) = current
            && trimmed.starts_with("\"expectation\":")
        {
            let indent = &line[..line.len() - trimmed.len()];
            output.push_str(&format!(
                "{indent}\"channels\": {},\n",
                derived.json(indent)
            ));
            applied += 1;
            current = None;
        }
        output.push_str(line);
        output.push('\n');
    }
    if skipping_channels != 0 {
        return Err(format!(
            "{}: an existing \"channels\" block does not close",
            path.display()
        ));
    }

    if applied != cases.len() {
        return Err(format!(
            "{}: rewrote {applied} of {} cases; a case's \"expectation\" anchor is missing",
            path.display(),
            cases.len()
        ));
    }
    fs::write(path, output).map_err(|error| format!("{}: {error}", path.display()))
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// Net brace depth a line contributes, ignoring braces inside JSON strings.
fn braces(line: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for character in line.chars() {
        match character {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => depth -= 1,
            _ => {}
        }
    }
    depth
}
