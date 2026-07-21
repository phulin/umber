use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};

use anyhow::{Result, bail};

use crate::scan::Candidate;

const MAX_PACKAGE_PEERS: usize = 16;
const MAX_DEPENDENCY_REPRESENTATIVES: usize = 2;
const MAX_HINTS: usize = 32;

#[derive(Debug)]
pub(crate) struct PackageDatabase {
    owners: BTreeMap<String, String>,
    dependencies: BTreeMap<String, Vec<String>>,
}

impl PackageDatabase {
    pub(crate) fn parse(input: &str) -> Result<Self> {
        let mut owners = BTreeMap::new();
        let mut dependencies = BTreeMap::new();
        let mut package = None::<String>;
        let mut runfiles = false;
        for line in input.lines() {
            if let Some(name) = line.strip_prefix("name ") {
                package = Some(name.to_owned());
                runfiles = false;
            } else if line.starts_with("runfiles ") {
                runfiles = true;
            } else if !line.starts_with(' ') {
                runfiles = false;
                if let (Some(package), Some(dependency)) =
                    (package.as_ref(), line.strip_prefix("depend "))
                    && !dependency.contains(':')
                    && !dependency.contains('/')
                    && !dependency.ends_with(".ARCH")
                {
                    dependencies
                        .entry(package.clone())
                        .or_insert_with(Vec::new)
                        .push(dependency.to_owned());
                }
            } else if runfiles
                && let (Some(package), Some(path)) =
                    (package.as_ref(), line.strip_prefix(" texmf-dist/"))
                && let Some(previous) = owners.insert(path.to_owned(), package.clone())
                && previous != *package
            {
                bail!("TeX Live runfile {path:?} belongs to both {previous:?} and {package:?}");
            }
        }
        for values in dependencies.values_mut() {
            values.sort();
            values.dedup();
        }
        Ok(Self {
            owners,
            dependencies,
        })
    }

    pub(crate) fn hints(
        &self,
        files: &BTreeMap<String, Candidate>,
    ) -> BTreeMap<String, Vec<String>> {
        let mut package_keys = BTreeMap::<&str, BTreeSet<String>>::new();
        let mut key_packages = BTreeMap::<String, &str>::new();
        for (key, candidate) in files {
            let Some(package) = self.owners.get(&candidate.relative) else {
                continue;
            };
            key_packages.insert(key.clone(), package);
            if is_preferred_key(key, &candidate.relative) {
                package_keys.entry(package).or_default().insert(key.clone());
            }
        }

        let mut result = BTreeMap::new();
        for (owner, package) in key_packages {
            let mut hints = BTreeSet::new();
            if let Some(peers) = package_keys.get(package) {
                extend_peer_hints(&mut hints, &owner, peers);
            }
            for dependency in self.dependencies.get(package).into_iter().flatten() {
                let Some(keys) = package_keys.get(dependency.as_str()) else {
                    continue;
                };
                hints.extend(keys.iter().take(MAX_DEPENDENCY_REPRESENTATIVES).cloned());
                if hints.len() >= MAX_HINTS {
                    break;
                }
            }
            hints.remove(&owner);
            if !hints.is_empty() {
                result.insert(owner, hints.into_iter().take(MAX_HINTS).collect());
            }
        }
        result
    }
}

fn extend_peer_hints(hints: &mut BTreeSet<String>, owner: &str, peers: &BTreeSet<String>) {
    hints.extend(
        peers
            .range::<str, _>((Excluded(owner), Unbounded))
            .chain(peers.range::<str, _>((Unbounded, Excluded(owner))))
            .take(MAX_PACKAGE_PEERS)
            .cloned(),
    );
}

fn is_preferred_key(key: &str, relative: &str) -> bool {
    let basename = relative.rsplit('/').next().unwrap_or(relative);
    key.ends_with(&format!(":{basename}"))
}

#[cfg(test)]
mod tests {
    use super::PackageDatabase;

    #[test]
    fn parses_only_runfiles_and_plain_package_dependencies() {
        let database = PackageDatabase::parse(
            "name alpha\ndepend beta\ndepend binary.ARCH\ndocfiles size=1\n texmf-dist/doc/a.tex\nrunfiles size=1\n texmf-dist/tex/a.sty\n\nname beta\nrunfiles size=1\n texmf-dist/tex/b.sty\n",
        )
        .expect("tlpdb");
        assert_eq!(database.owners["tex/a.sty"], "alpha");
        assert!(!database.owners.contains_key("doc/a.tex"));
        assert_eq!(database.dependencies["alpha"], ["beta"]);
    }
}
