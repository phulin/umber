//! Registry and single reachability choke point for the byte-exact end-to-end
//! DVI conformance gates.
//!
//! Every gate compares Umber's assembled DVI against an oracle produced by a
//! real reference engine. Those oracles are derived from third-party documents
//! and are deliberately gitignored, so they exist only where someone ran
//! `scripts/setup-conformance-tests.sh`. That is a licensing decision and it
//! stands.
//!
//! What must not stand is a gate whose absence is indistinguishable from its
//! success. The gates previously each ran their own `if !present { eprintln!(
//! "skipping ..."); return; }` check, and libtest swallows a passing test's
//! captured output, so those notices were invisible without `--nocapture`:
//! a fresh worktree reported a clean suite while the epic's flagship byte-exact
//! Story DVI parity result never executed.
//!
//! This module removes the skip branch from the gates entirely. [`with_gate`]
//! is the only way a gate reaches its assets, it has no caller-visible skip
//! path, and every notice it emits is written to the process's real stderr
//! handle rather than through `eprintln!`, so libtest's capture cannot hide it.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Repository-relative directory holding the locally generated, deliberately
/// gitignored `<gate>.expected.dvi` oracles.
pub const ORACLE_DIR: &str = "tests/corpus/e2e";

/// Escape hatch for environments that genuinely cannot host the oracles (no
/// TeX distribution, no network). Setting it to [`OPT_OUT_VALUE`] downgrades
/// every gate from a hard failure to a loud, uncapturable stderr notice.
const OPT_OUT_VAR: &str = "UMBER_CONFORMANCE_ORACLES";

/// The only accepted [`OPT_OUT_VAR`] value; anything else is rejected rather
/// than quietly treated as "required", so a typo can never look deliberate.
const OPT_OUT_VALUE: &str = "optional";

/// One byte-exact end-to-end DVI conformance gate.
///
/// `inputs` lists every repository-relative external file the gate stages
/// besides its oracle, so an absence report names all of them at once instead
/// of failing one staging step at a time.
pub struct ConformanceGate {
    /// Gate name; also the `<name>.expected.dvi` oracle stem and the string
    /// each `#[test]` passes to [`with_gate`].
    pub name: &'static str,
    /// Repository-relative external inputs the gate stages.
    pub inputs: &'static [&'static str],
    /// Exact commands that materialize this gate's assets, in order.
    pub materialize: &'static [&'static str],
}

/// Every byte-exact end-to-end DVI conformance gate in this crate.
///
/// `conformance_gate_registry_matches_gitignore` below holds this list in
/// exact correspondence with the gitignored oracle entries in `.gitignore`,
/// and `conformance_gate_registry_is_reachable` holds every entry in
/// correspondence with a real `with_gate` call site. A new gate therefore
/// cannot be added with a private presence check of its own.
pub const GATES: &[ConformanceGate] = &[
    ConformanceGate {
        name: "story",
        inputs: &[
            "third_party/corpus/story.tex",
            "third_party/corpus/plain.tex",
            "third_party/hyphen/hyphen.tex",
        ],
        materialize: &["scripts/setup-conformance-tests.sh"],
    },
    ConformanceGate {
        name: "gentle",
        inputs: &[
            "third_party/corpus/gentle.tex",
            "third_party/corpus/plain.tex",
            "third_party/hyphen/hyphen.tex",
        ],
        materialize: &["scripts/setup-conformance-tests.sh"],
    },
    ConformanceGate {
        name: "trip",
        inputs: &["third_party/trip/trip.tex", "third_party/trip/trip.tfm"],
        materialize: &[
            "scripts/fetch-conformance-inputs.sh",
            "scripts/regen-fixtures.sh --case e2e/trip",
        ],
    },
    ConformanceGate {
        name: "etrip",
        inputs: &["third_party/trip/etrip.tex", "third_party/trip/trip.tfm"],
        materialize: &[
            "scripts/fetch-conformance-inputs.sh",
            "scripts/regen-fixtures.sh --case e2e/etrip",
        ],
    },
];

/// Assets handed to a gate body once every required file is confirmed present.
pub struct GateAssets {
    /// Registered gate name, also the oracle stem.
    pub name: &'static str,
    /// Absolute repository root.
    pub repo_root: PathBuf,
    /// Absolute path to this gate's locally generated DVI oracle.
    pub oracle: PathBuf,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repository root")
}

/// Writes a line to the process's real stderr handle.
///
/// `eprintln!` routes through libtest's thread-local output capture, which
/// discards everything a passing test prints unless `--nocapture` is given.
/// `std::io::stderr()` writes to file descriptor 2 directly, so a gate's
/// "ran" confirmation and its opt-out notice are visible in every run.
fn note(message: &str) {
    // One `write_all` for the line and its terminator: gates run on parallel
    // libtest threads, and two writes could interleave into a joined line.
    let line = format!("{message}\n");
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(line.as_bytes());
    let _ = stderr.flush();
}

impl ConformanceGate {
    fn oracle(&self, repo_root: &Path) -> PathBuf {
        repo_root
            .join(ORACLE_DIR)
            .join(format!("{}.expected.dvi", self.name))
    }

    /// Returns every required asset that is absent, oracle first, each with a
    /// short explanation of what it is and where it comes from.
    fn missing_assets(&self, repo_root: &Path) -> Vec<String> {
        let mut missing = Vec::new();
        let oracle = self.oracle(repo_root);
        if !oracle.is_file() {
            missing.push(format!(
                "{}/{}.expected.dvi (locally generated reference DVI oracle; \
                 gitignored on purpose because it derives from a third-party \
                 document and must never be committed)",
                ORACLE_DIR, self.name
            ));
        }
        for input in self.inputs {
            if !repo_root.join(input).is_file() {
                missing.push(format!("{input} (external input)"));
            }
        }
        missing
    }

    fn absence_report(&self, missing: &[String]) -> String {
        let mut report = format!(
            "end-to-end conformance gate `{}` cannot run: {} required asset(s) are absent.\n\n\
             missing:\n",
            self.name,
            missing.len()
        );
        for entry in missing {
            report.push_str("  ");
            report.push_str(entry);
            report.push('\n');
        }
        report.push_str("\nmaterialize them from the repository root with:\n");
        for command in self.materialize {
            report.push_str("  ");
            report.push_str(command);
            report.push('\n');
        }
        report.push_str(
            "\nThis gate is a byte-exact DVI parity result and is never skipped silently.\n\
             If this environment genuinely cannot host the oracles, set\n  ",
        );
        report.push_str(OPT_OUT_VAR);
        report.push('=');
        report.push_str(OPT_OUT_VALUE);
        report.push_str(
            "\nto downgrade every conformance gate to an uncapturable stderr notice, and do\n\
             not report the suite as clean. See the \"End-to-End Conformance Gate Contract\"\n\
             section of docs/testing_infrastructure.md.",
        );
        report
    }
}

/// Returns whether the operator explicitly downgraded absent oracles to a
/// notice. Any value other than [`OPT_OUT_VALUE`] is a hard error rather than
/// a silent fallback to the required default.
fn opted_out() -> bool {
    match std::env::var(OPT_OUT_VAR) {
        Ok(value) if value == OPT_OUT_VALUE => true,
        Ok(value) => panic!(
            "{OPT_OUT_VAR}={value:?} is not a recognized value; \
             the only accepted value is {OPT_OUT_VALUE:?} (unset means the \
             conformance oracles are required)"
        ),
        Err(_) => false,
    }
}

/// Runs `body` against a registered gate's assets.
///
/// There is no caller-visible skip path: either every required asset is
/// present and `body` runs, or the gate fails with a report naming each
/// missing file and the exact command that materializes it. The single
/// non-failing absence path is the explicit [`OPT_OUT_VAR`] opt-out, which
/// emits the same report to real stderr.
pub fn with_gate(name: &str, body: impl FnOnce(&GateAssets)) {
    let gate = GATES
        .iter()
        .find(|gate| gate.name == name)
        .unwrap_or_else(|| {
            panic!(
                "unregistered end-to-end conformance gate {name:?}; \
                 add it to `assets::GATES` so it can never acquire a private \
                 presence check"
            )
        });
    let repo_root = repo_root();
    let missing = gate.missing_assets(&repo_root);
    if !missing.is_empty() {
        let report = gate.absence_report(&missing);
        assert!(opted_out(), "{report}");
        note(&format!("\n{report}\n"));
        return;
    }

    let oracle = gate.oracle(&repo_root);
    note(&format!(
        "conformance gate `{}`: running against {}/{}.expected.dvi",
        gate.name, ORACLE_DIR, gate.name
    ));
    body(&GateAssets {
        name: gate.name,
        repo_root,
        oracle,
    });
}

/// Holds [`GATES`] in exact correspondence with the gitignored oracle entries
/// in `.gitignore`.
///
/// A new byte-exact DVI gate needs a new gitignored `<name>.expected.dvi`
/// entry; this test makes registering it here mandatory, which is what forces
/// it through [`with_gate`] rather than a private presence check of its own.
#[test]
fn conformance_gate_registry_matches_gitignore() {
    let root = repo_root();
    let path = root.join(".gitignore");
    let text = fs::read_to_string(&path).expect("read .gitignore");
    let prefix = format!("/{ORACLE_DIR}/");
    let ignored: BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&prefix))
        .filter_map(|line| line.strip_suffix(".expected.dvi"))
        .collect();
    let registered: BTreeSet<&str> = GATES.iter().map(|gate| gate.name).collect();
    assert_eq!(
        ignored, registered,
        "`.gitignore` and `assets::GATES` disagree about which byte-exact DVI \
         oracles exist; every gitignored {ORACLE_DIR}/<name>.expected.dvi must \
         be a registered gate so it is reached through `assets::with_gate` and \
         can never skip silently"
    );
    assert_eq!(
        registered.len(),
        GATES.len(),
        "`assets::GATES` contains duplicate gate names"
    );
}

/// Holds every registered gate in correspondence with a real [`with_gate`]
/// call site, so the registry cannot become decorative while a gate quietly
/// keeps its own presence check.
#[test]
fn conformance_gate_registry_is_reachable() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/e2e_conformance.rs");
    let text = fs::read_to_string(&source).expect("read e2e_conformance.rs");
    for gate in GATES {
        let call = format!("with_gate(\"{}\"", gate.name);
        assert!(
            text.contains(&call),
            "registered conformance gate `{}` has no `assets::{call})` call site in {}; \
             every gate must reach its assets through the shared choke point",
            gate.name,
            source.display()
        );
    }
}
