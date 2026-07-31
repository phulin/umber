//! The rendered worklist: the per-fixture accounting, the ordered entries, and
//! the verdict the process exit status encodes.
//!
//! One run's report has to answer three questions at a glance: how many
//! divergences the comparator found, how many distinct root sites those
//! collapse to, and whether anything was left uncompared. The first two are
//! printed as separately labeled numbers because they are compared against
//! different historical figures; the third is printed because every bound this
//! epic has been bitten by was a bound nobody printed.
//!
//! The third question is also the one the exit status answers, because a
//! reader who only checks the status is exactly the reader a partial run
//! misleads. [`RunOutcome`] is that answer: "complete and clean", "complete
//! and diverged", and "partial" are three different statuses, so no consumer
//! can read "did not run" as "ran and found nothing".
//!
//! # The invariant every number here is printed under
//!
//! Two numbers are printed with different units and a third -- the
//! `--max-divergences` budget -- is stated on the command line, so each one
//! names its own unit where it is printed, and any number a bound turned into
//! a floor says so *at the point it is printed*, not only in the verdict at
//! the bottom. The budget counts ordered divergences, never root sites and
//! never printed entries; the grouped worklist prints one entry per root
//! site, so under a bound the entry count is smaller than the budget, and
//! under `--ungrouped` it equals it. No single unit can equal the printed
//! entry count in both views, so equality of the two is not the property to
//! preserve -- labeling is. See [`crate::DEFAULT_MAX_DIVERGENCES`] for why
//! the budget's unit is ordered divergences.

use std::fmt;

use crate::group::{RootSite, group};
use crate::{DEFAULT_MAX_DIVERGENCES, Divergence};

#[cfg(test)]
mod tests;

/// Columns the ordered index list of a grouped entry wraps at.
const INDEX_LIST_WIDTH: usize = 88;

/// What one run means to a consumer reading nothing but the exit status.
///
/// The distinction that matters is not "found something" versus "found
/// nothing": it is whether the numbers in the report are the whole truth.
/// A run that never compared a registered fixture, or that stopped a
/// fixture's comparison at its `--max-divergences` budget, reports a lower
/// bound, and a lower bound of zero is indistinguishable from convergence
/// unless the status says otherwise. So partiality gets its own status and
/// outranks the divergence count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    /// Every registered fixture was compared to exhaustion and none diverged.
    /// The only outcome that means the command core converges.
    Clean,
    /// Every registered fixture was compared to exhaustion and at least one
    /// diverged. The printed totals are exact.
    Diverged,
    /// At least one registered fixture was never compared, or at least one
    /// fixture's comparison stopped at its budget. Every printed total is a
    /// lower bound; the run does not answer whether the core converges.
    Partial,
}

impl RunOutcome {
    /// The process exit status this outcome is reported as.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Clean => 0,
            Self::Diverged => 1,
            Self::Partial => 2,
        }
    }
}

/// The status a run that could not be performed at all exits with: a usage
/// error, an unreadable suite, or a registry inconsistent with its pin. Kept
/// distinct from [`RunOutcome::Diverged`] so "the tool refused to run" is
/// never read as "the tool ran and produced this worklist".
pub const EXIT_NOT_RUN: u8 = 3;

/// What one registered fixture contributed to the run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureState {
    /// The fixture was replayed and compared.
    Compared {
        /// Every ordered divergence this fixture contributed.
        divergences: usize,
        /// How many of `divergences` the `--max-divergences` budget counted:
        /// the stream mismatches. A contained replay failure is reported
        /// outside the budget, so `divergences` can exceed `budgeted` -- and
        /// therefore exceed the budget -- by one. Recorded separately so the
        /// bounded notice can explain that arithmetic instead of printing a
        /// total that looks like the budget was overrun.
        budgeted: usize,
        /// Oracle event index of this fixture's first divergence.
        first_index: Option<usize>,
        /// The `--max-divergences` budget stopped this fixture's comparison
        /// before either stream was exhausted.
        budget_reached: bool,
    },
    /// The document trace tree is not generated on this checkout, so the
    /// fixture was never compared. A fixture that reports no divergence
    /// because it never ran must not read like a clean fixture.
    NotGenerated,
}

/// One registered fixture's line in the per-fixture accounting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureSummary {
    /// Registry name, as the fixture registry spells it.
    pub name: String,
    /// Full identity (`name manifest=...`) each divergence carries, used to
    /// attribute grouped entries back to their fixture.
    pub identity: String,
    /// Whether this fixture contributes diagnostics but never changes the verdict.
    pub advisory: bool,
    pub state: FixtureState,
}

impl FixtureSummary {
    /// A fixture that was never compared has no identity to attribute
    /// divergences by.
    pub fn not_generated(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            identity: String::new(),
            advisory: false,
            state: FixtureState::NotGenerated,
        }
    }
}

/// Everything one comparison run reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonReport {
    /// Every registered fixture, in replay order, compared or not.
    pub fixtures: Vec<FixtureSummary>,
    /// Every divergence, in stream order across fixtures. Grouping never
    /// removes one; it only decides how they print.
    pub divergences: Vec<Divergence>,
    /// Non-gating geometry diagnostics, retained and counted separately.
    pub advisories: Vec<Divergence>,
    /// Whether exact recurrences are collapsed into one entry each.
    pub grouped: bool,
    /// The per-fixture budget this run used, for the bounded notice.
    pub max_divergences: usize,
}

impl Default for ComparisonReport {
    fn default() -> Self {
        Self {
            fixtures: Vec::new(),
            divergences: Vec::new(),
            advisories: Vec::new(),
            grouped: true,
            max_divergences: DEFAULT_MAX_DIVERGENCES,
        }
    }
}

impl ComparisonReport {
    /// The historical headline metric: every ordered divergence the
    /// comparator found, unaffected by grouping.
    pub fn divergence_count(&self) -> usize {
        self.divergences.len()
    }

    /// Ordered advisory geometry differences, unaffected by grouping.
    pub fn advisory_count(&self) -> usize {
        self.advisories.len()
    }

    /// The grouped worklist. Computed on demand; grouping is a pure function
    /// of the ordered divergences.
    pub fn root_sites(&self) -> Vec<RootSite<'_>> {
        group(&self.divergences)
    }

    /// Registered fixtures this run never compared, in replay order.
    pub fn uncompared(&self) -> Vec<&str> {
        self.fixtures
            .iter()
            .filter(|fixture| {
                !fixture.advisory && matches!(fixture.state, FixtureState::NotGenerated)
            })
            .map(|fixture| fixture.name.as_str())
            .collect()
    }

    /// Registered fixtures whose comparison stopped at the per-fixture
    /// `--max-divergences` budget, in replay order.
    pub fn bounded(&self) -> Vec<&str> {
        self.fixtures
            .iter()
            .filter(|fixture| {
                !fixture.advisory
                    && matches!(
                        fixture.state,
                        FixtureState::Compared {
                            budget_reached: true,
                            ..
                        }
                    )
            })
            .map(|fixture| fixture.name.as_str())
            .collect()
    }

    /// Whether every registered fixture was compared to exhaustion, so the
    /// printed totals are exact rather than lower bounds.
    pub fn is_complete(&self) -> bool {
        self.uncompared().is_empty() && self.bounded().is_empty()
    }

    /// The verdict this run is reported as, and the exit status it earns.
    pub fn outcome(&self) -> RunOutcome {
        if !self.is_complete() {
            RunOutcome::Partial
        } else if self.divergences.is_empty() {
            RunOutcome::Clean
        } else {
            RunOutcome::Diverged
        }
    }
}

impl fmt::Display for ComparisonReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sites = self.root_sites();
        self.write_header(formatter, sites.len())?;
        self.write_fixture_accounting(formatter, &sites)?;
        if self.grouped {
            for (position, site) in sites.iter().enumerate() {
                formatter.write_str("\n")?;
                write_grouped_entry(formatter, position, site)?;
            }
        } else {
            for (position, divergence) in self.divergences.iter().enumerate() {
                write!(formatter, "\n[{position}] {divergence}")?;
            }
        }
        self.write_advisories(formatter)?;
        formatter.write_str("\n")?;
        self.write_verdict(formatter)
    }
}

impl ComparisonReport {
    fn write_advisories(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sites = group(&self.advisories);
        write!(
            formatter,
            "\nADVISORY geometry diagnostics (non-gating): {} ordered difference(s) in {} root site(s)\n",
            self.advisory_count(),
            sites.len()
        )?;
        if self.grouped {
            for (position, site) in sites.iter().enumerate() {
                formatter.write_str("\n")?;
                write_grouped_entry(formatter, position, site)?;
            }
        } else {
            for (position, divergence) in self.advisories.iter().enumerate() {
                write!(formatter, "\n[{position}] {divergence}")?;
            }
        }
        Ok(())
    }

    /// The last line of the report, naming the outcome and the exit status
    /// that carries it. A reader who scrolls to the bottom and a reader who
    /// only inspects `$?` must reach the same conclusion, so the same three
    /// words appear in both places.
    fn write_verdict(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = self.outcome();
        let code = outcome.exit_code();
        let divergences = self.divergence_count();
        let advisories = self.advisory_count();
        match outcome {
            RunOutcome::Clean => write!(
                formatter,
                "\nVERDICT: CLEAN (exit {code}) -- every registered fixture was compared to\n  \
                 exhaustion and no gating channel diverged; {advisories} advisory geometry difference(s)."
            ),
            RunOutcome::Diverged => write!(
                formatter,
                "\nVERDICT: DIVERGED (exit {code}) -- every registered fixture was compared to\n  \
                 exhaustion; {divergences} ordered gating divergence(s) is the exact total;\n  \
                 {advisories} geometry difference(s) is advisory."
            ),
            RunOutcome::Partial => {
                write!(
                    formatter,
                    "\nVERDICT: PARTIAL (exit {code}) -- this run did not compare everything it\n  \
                     registers, so {divergences} ordered divergence(s) is a LOWER BOUND, not a\n  \
                     total, and a total of 0 would not mean convergence."
                )?;
                let uncompared = self.uncompared();
                if !uncompared.is_empty() {
                    write!(
                        formatter,
                        "\n  never compared ({}): {}\n  \
                         generate the missing traces with \
                         scripts/build-tex82-document-traces.sh",
                        uncompared.len(),
                        uncompared.join(", "),
                    )?;
                }
                let bounded = self.bounded();
                if !bounded.is_empty() {
                    write!(
                        formatter,
                        "\n  stopped at the --max-divergences {} budget ({}): {}\n  \
                         raise the budget to see the rest",
                        self.max_divergences,
                        bounded.len(),
                        bounded.join(", "),
                    )?;
                }
                Ok(())
            }
        }
    }

    fn write_header(&self, formatter: &mut fmt::Formatter<'_>, sites: usize) -> fmt::Result {
        let divergences = self.divergence_count();
        if self.grouped {
            writeln!(
                formatter,
                "{divergences} ordered divergence(s) in {sites} root site(s):"
            )?;
            self.write_totals_status(formatter)?;
            formatter.write_str(
                "  root site(s): the entries below, one per group of divergences that are\n    \
                 identical after erasing source positions and nothing else. Every\n    \
                 divergence is in exactly one group; none is dropped, sampled, or\n    \
                 truncated. Pass --ungrouped for one entry per divergence.\n",
            )
        } else {
            writeln!(formatter, "{divergences} ordered divergence(s):")?;
            self.write_totals_status(formatter)?;
            writeln!(
                formatter,
                "  ungrouped (--ungrouped): one entry per divergence. Collapsing exact\n    \
                 recurrences would report {sites} root site(s); the divergence total above\n    \
                 is the same either way."
            )
        }
    }

    /// What the headline totals are, immediately under the headline.
    ///
    /// A partial run's totals are already named as lower bounds by the verdict
    /// at the bottom, but the header is where a reader takes a number from,
    /// and the complete-run wording tells them to compare it against a
    /// historical figure. Printing that instruction over a bounded total is
    /// the mild form of this epic's recurring defect -- a number labeled as
    /// something it is not -- so the instruction is withdrawn exactly when it
    /// stops being true, in the same place it was given.
    fn write_totals_status(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_complete() {
            formatter.write_str(
                "  LOWER BOUND: this run stopped short of comparing everything it registers\n    \
                 (the per-fixture accounting below and the VERDICT line at the end name\n    \
                 which fixtures and why). Every total above is a floor, not a total, and\n    \
                 none of them is comparable against a historical figure.\n",
            )?;
        }
        if !self.grouped {
            return Ok(());
        }
        if self.is_complete() {
            formatter.write_str(
                "  divergence(s): what the comparator found. Grouping does not change this\n    \
                 number; it is the one to compare against historical totals.\n",
            )
        } else {
            formatter.write_str(
                "  divergence(s): what the comparator found before it stopped short.\n    \
                 Grouping does not change this number; the bound above does.\n",
            )
        }
    }

    fn write_fixture_accounting(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        sites: &[RootSite<'_>],
    ) -> fmt::Result {
        let advisory_sites = group(&self.advisories);
        if self.fixtures.is_empty() {
            return Ok(());
        }
        let width = self
            .fixtures
            .iter()
            .map(|fixture| fixture.name.len())
            .max()
            .unwrap_or(0);
        formatter.write_str("per fixture, in replay order:\n")?;
        for fixture in &self.fixtures {
            write!(formatter, "  {:width$}  ", fixture.name)?;
            if fixture.advisory {
                formatter.write_str("ADVISORY (non-gating): ")?;
            }
            match &fixture.state {
                FixtureState::NotGenerated => {
                    formatter.write_str("not compared -- trace not generated on this checkout\n")?
                }
                FixtureState::Compared { divergences: 0, .. } => {
                    formatter.write_str("0 divergence(s)\n")?
                }
                FixtureState::Compared {
                    divergences,
                    budgeted,
                    first_index,
                    budget_reached,
                } => {
                    let fixture_sites = if fixture.advisory {
                        &advisory_sites
                    } else {
                        sites
                    };
                    let root_sites = fixture_sites
                        .iter()
                        .filter(|site| site.fixture() == fixture.identity)
                        .count();
                    write!(
                        formatter,
                        "{divergences} divergence(s) in {root_sites} root site(s)"
                    )?;
                    if let Some(index) = first_index {
                        write!(formatter, ", first at oracle event {index}")?;
                    }
                    formatter.write_str("\n")?;
                    if *budget_reached {
                        self.write_bounded_notice(
                            formatter,
                            width,
                            *divergences,
                            *budgeted,
                            root_sites,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl ComparisonReport {
    /// The notice under a fixture whose comparison the budget stopped.
    ///
    /// It has to reconcile up to three numbers a reader sees together on one
    /// screen -- the budget on the command line, the fixture's divergence
    /// total, and the root-site count next to it -- because two of the three
    /// invite a wrong inference. The root-site count is smaller than the
    /// budget, which reads as the budget not having been spent; and the
    /// divergence total can exceed the budget by the one contained replay
    /// failure that is deliberately reported outside it, which reads as the
    /// budget having been overrun. Both are stated here rather than left to be
    /// derived.
    fn write_bounded_notice(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        width: usize,
        divergences: usize,
        budgeted: usize,
        root_sites: usize,
    ) -> fmt::Result {
        let budget = self.max_divergences;
        let mut lines = vec![
            format!("BOUNDED: --max-divergences {budget} counts ordered divergences; it"),
            "counts neither root sites nor printed entries. Comparison of this".to_string(),
            format!(
                "fixture stopped at {budgeted} of them, so its {divergences} divergence(s) and"
            ),
            format!("{root_sites} root site(s) above are both floors: more of each exist"),
            "beyond its last entry.".to_string(),
        ];
        if divergences > budgeted {
            lines.push("Its contained replay failure is reported outside the mismatch".to_string());
            lines.push(format!(
                "budget, which is why {divergences} is more than the budget of {budget}."
            ));
        }
        for line in lines {
            writeln!(formatter, "  {:width$}  {line}", "")?;
        }
        Ok(())
    }
}

/// One grouped worklist entry: the representative rendered exactly as the
/// ungrouped worklist renders it, plus the accounting for the recurrences it
/// stands for.
fn write_grouped_entry(
    formatter: &mut fmt::Formatter<'_>,
    position: usize,
    site: &RootSite<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "[{position}] x{} {}",
        site.count(),
        site.representative()
    )?;
    if site.count() == 1 {
        return Ok(());
    }
    write!(
        formatter,
        "\n  recurrence: {} exact occurrence(s) of this root site, {} suppressed cascade \
         event(s) in total;\n    the entry above is the first. Every occurrence, by oracle \
         event index:",
        site.count(),
        site.suppressed_cascade(),
    )?;
    write_index_list(formatter, site)
}

/// Every occurrence's oracle event index, wrapped. The whole list is printed:
/// an entry that stands for a hundred occurrences has to let a dispatched
/// agent reach all hundred, so this is deliberately never elided.
fn write_index_list(formatter: &mut fmt::Formatter<'_>, site: &RootSite<'_>) -> fmt::Result {
    let mut column = INDEX_LIST_WIDTH;
    for (position, divergence) in site.occurrences().iter().enumerate() {
        let rendered = format!(
            "{}{}",
            divergence.index(),
            if position + 1 == site.count() {
                ""
            } else {
                ","
            }
        );
        if column + rendered.len() + 1 > INDEX_LIST_WIDTH {
            formatter.write_str("\n   ")?;
            column = 3;
        }
        write!(formatter, " {rendered}")?;
        column += rendered.len() + 1;
    }
    Ok(())
}
