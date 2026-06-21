//! Pacman and libalpm access helpers.
//!
//! This module is the boundary between the application and the pacman package
//! databases. It loads installed package data, resolves optional dependency
//! satisfiers, and builds the list of missing optional dependencies shown by
//! the UI.

use std::collections::BTreeMap;
use std::fs;

use alpm::{Alpm, Dep, DepMod, SigLevel};
use eyre::Result;

use crate::model::{
    InstalledPackage, MissingOptionalDep, OptionalDep, OptionalDepRequester, PackageInfo,
};

const IGNORED_MISSING_OPTIONAL_DEP_PREFIXES: &[&str] = &["tesseract-data-", "tesarract-data-"];
const IGNORED_MISSING_OPTIONAL_DEPS: &[&str] = &["tesseract-data", "tesarract-data"];

/// Result of trying to resolve a missing optional dependency against sync databases.
enum OptionalDepResolution {
    /// The dependency exists in a sync database and can be shown with package metadata.
    Resolved(MissingOptionalDep),
    /// The dependency could not be resolved and should be hidden from sync-backed results.
    Unresolved,
}

/// Loads installed packages and missing optional dependency suggestions from pacman.
///
/// The local pacman database is read from `/var/lib/pacman`, and sync
/// databases are registered from `/var/lib/pacman/sync` when they are present.
/// Missing optional dependencies are resolved against the sync databases so
/// the UI can show versions and descriptions for installable packages.
pub(crate) fn load_package_data() -> Result<(Vec<PackageInfo>, Vec<MissingOptionalDep>)> {
    let alpm = Alpm::new("/", "/var/lib/pacman")?;
    register_syncdbs(&alpm);

    let local_packages = alpm.localdb().pkgs();
    let syncdbs = alpm.syncdbs();
    let mut packages = local_packages
        .iter()
        .map(|pkg| PackageInfo {
            name: pkg.name().to_owned(),
            version: pkg.version().to_string(),
            description: pkg.desc().map(str::to_owned),
            optional_deps: pkg
                .optdepends()
                .into_iter()
                .map(|dep| {
                    let requirement = dep_requirement(dep);

                    OptionalDep {
                        name: requirement.clone(),
                        optional_for: dep.desc().map(str::to_owned).unwrap_or_default(),
                        installed_package: resolve_installed_optional_dep(
                            &local_packages,
                            dep,
                            requirement.as_str(),
                        ),
                    }
                })
                .collect(),
        })
        .collect::<Vec<_>>();

    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let missing_optional_deps =
        missing_optional_deps_from_packages_and_syncdbs(&packages, &syncdbs);

    Ok((packages, missing_optional_deps))
}

/// Registers all available local sync databases with libalpm.
///
/// Missing or unreadable sync database directories are ignored. A missing sync
/// database only affects package metadata enrichment for missing dependencies;
/// installed package scanning still works.
fn register_syncdbs(alpm: &Alpm) {
    let Ok(entries) = fs::read_dir("/var/lib/pacman/sync") else {
        return;
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(db_name) = file_name.strip_suffix(".db") else {
            continue;
        };

        let _ = alpm.register_syncdb(db_name, SigLevel::NONE);
    }
}

/// Builds missing optional dependency suggestions from already-loaded packages.
///
/// This variant does not consult sync databases, so returned dependencies only
/// contain names and requester information. It is used by tests and callers
/// that already have package data.
pub(crate) fn missing_optional_deps_from_packages(
    packages: &[PackageInfo],
) -> Vec<MissingOptionalDep> {
    missing_optional_deps_from_packages_with_resolver(packages, |_| None)
}

/// Builds missing optional dependency suggestions and enriches them from sync databases.
///
/// Dependencies that cannot be resolved in a sync database are skipped. This
/// avoids suggesting virtual names or provider-only requirements that pacman
/// cannot install directly by that name.
fn missing_optional_deps_from_packages_and_syncdbs(
    packages: &[PackageInfo],
    syncdbs: &alpm::AlpmList<'_, &alpm::Db>,
) -> Vec<MissingOptionalDep> {
    missing_optional_deps_from_packages_with_resolver(packages, |dep| {
        Some(
            syncdbs
                .find_satisfier(dep.name.as_str())
                .map(|pkg| OptionalDepResolution::Resolved(sync_missing_optional_dep(pkg)))
                .unwrap_or(OptionalDepResolution::Unresolved),
        )
    })
}

/// Creates a missing dependency record when no sync package metadata is available.
fn unresolved_missing_optional_dep(dep: &OptionalDep) -> MissingOptionalDep {
    MissingOptionalDep {
        name: dep.name.clone(),
        version: None,
        description: None,
        wanted_by: Vec::new(),
    }
}

/// Converts a sync database package into a missing optional dependency record.
fn sync_missing_optional_dep(pkg: &alpm::Package) -> MissingOptionalDep {
    MissingOptionalDep {
        name: pkg.name().to_owned(),
        version: Some(pkg.version().to_string()),
        description: pkg.desc().map(str::to_owned),
        wanted_by: Vec::new(),
    }
}

/// Finds the installed package that satisfies an optional dependency.
///
/// The lookup first asks libalpm for a direct satisfier of the dependency
/// requirement. If that fails, it also accepts installed packages that conflict
/// with or replace the requested dependency name, which handles common package
/// rename or provider cases.
fn resolve_installed_optional_dep(
    local_packages: &alpm::AlpmList<'_, &alpm::Package>,
    dep: &Dep,
    requirement: &str,
) -> Option<InstalledPackage> {
    local_packages
        .find_satisfier(requirement)
        .or_else(|| find_installed_replacement(local_packages, dep))
        .map(installed_package_from_alpm)
}

/// Finds an installed package that should be treated as replacing the dependency.
fn find_installed_replacement<'a>(
    local_packages: &alpm::AlpmList<'_, &'a alpm::Package>,
    dep: &Dep,
) -> Option<&'a alpm::Package> {
    local_packages.iter().find(|pkg| {
        pkg.conflicts()
            .iter()
            .any(|conflict| dep_names_match(conflict, dep))
            || pkg
                .replaces()
                .iter()
                .any(|replacement| dep_names_match(replacement, dep))
    })
}

/// Returns `true` when two libalpm dependency names refer to the same package name.
pub(crate) fn dep_names_match(candidate: &Dep, requested: &Dep) -> bool {
    candidate.name() == requested.name()
}

/// Converts a libalpm package into the small installed-package model used by the UI.
fn installed_package_from_alpm(pkg: &alpm::Package) -> InstalledPackage {
    InstalledPackage {
        name: pkg.name().to_owned(),
        version: pkg.version().to_string(),
    }
}

/// Groups uninstalled optional dependencies by dependency name.
///
/// Each missing dependency is listed once with all packages that requested it.
/// Results are sorted by requester count descending and then by dependency name.
fn missing_optional_deps_from_packages_with_resolver<F>(
    packages: &[PackageInfo],
    mut resolve: F,
) -> Vec<MissingOptionalDep>
where
    F: FnMut(&OptionalDep) -> Option<OptionalDepResolution>,
{
    let mut deps = BTreeMap::<String, MissingOptionalDep>::new();

    for package in packages {
        for dep in package
            .optional_deps
            .iter()
            .filter(|dep| dep.installed_package.is_none())
        {
            let mut resolved = match resolve(dep) {
                Some(OptionalDepResolution::Resolved(resolved)) => resolved,
                Some(OptionalDepResolution::Unresolved) => continue,
                None => unresolved_missing_optional_dep(dep),
            };
            if is_ignored_missing_optional_dep(&resolved.name) {
                continue;
            }

            let key = resolved.name.clone();

            deps.entry(key)
                .or_insert_with(|| {
                    resolved.wanted_by.clear();
                    resolved
                })
                .wanted_by
                .push(OptionalDepRequester {
                    package_name: package.name.clone(),
                    reason: dep.reason(),
                });
        }
    }

    let mut deps = deps
        .into_values()
        .map(|mut dep| {
            dep.wanted_by
                .sort_by(|a, b| a.package_name.cmp(&b.package_name));
            dep
        })
        .collect::<Vec<_>>();

    deps.sort_by(|a, b| {
        b.wanted_by
            .len()
            .cmp(&a.wanted_by.len())
            .then_with(|| a.name.cmp(&b.name))
    });

    deps
}

/// Returns `true` for optional dependency names that should not be suggested.
///
/// Tesseract language data packages are intentionally ignored because package
/// metadata commonly requests broad language-data names that are not useful as
/// general install suggestions.
fn is_ignored_missing_optional_dep(name: &str) -> bool {
    IGNORED_MISSING_OPTIONAL_DEPS.contains(&name)
        || IGNORED_MISSING_OPTIONAL_DEP_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Formats a libalpm dependency with its version constraint.
fn dep_requirement(dep: &Dep) -> String {
    match dep.depmod() {
        DepMod::Any => dep.name().to_owned(),
        DepMod::Eq => format!(
            "{}={}",
            dep.name(),
            dep.version().expect("missing dep version")
        ),
        DepMod::Ge => format!(
            "{}>={}",
            dep.name(),
            dep.version().expect("missing dep version")
        ),
        DepMod::Le => format!(
            "{}<={}",
            dep.name(),
            dep.version().expect("missing dep version")
        ),
        DepMod::Gt => format!(
            "{}>{}",
            dep.name(),
            dep.version().expect("missing dep version")
        ),
        DepMod::Lt => format!(
            "{}<{}",
            dep.name(),
            dep.version().expect("missing dep version")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpm::Depend;

    #[test]
    fn missing_optional_deps_skip_satisfied_dependencies() {
        let packages = vec![PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![
                OptionalDep {
                    name: "sqlite".to_owned(),
                    optional_for: "database support".to_owned(),
                    installed_package: Some(InstalledPackage {
                        name: "sqlite".to_owned(),
                        version: "3.51.1-1".to_owned(),
                    }),
                },
                OptionalDep {
                    name: "mysql".to_owned(),
                    optional_for: "database support".to_owned(),
                    installed_package: None,
                },
            ],
        }];

        let missing = missing_optional_deps_from_packages(&packages);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "mysql");
        assert_eq!(missing[0].wanted_by[0].package_name, "example");
    }

    #[test]
    fn replacement_deps_match_requested_optional_dep_names() {
        let replacement = Depend::new("pulseaudio");
        let requested = Depend::new("pulseaudio");
        let unrelated = Depend::new("jack");

        assert!(dep_names_match(&replacement, &requested));
        assert!(!dep_names_match(&replacement, &unrelated));
    }

    #[test]
    fn missing_optional_deps_sort_by_wanted_count_and_filter_ignored_names() {
        let packages = vec![
            PackageInfo {
                name: "alpha".to_owned(),
                version: "1.0.0".to_owned(),
                description: None,
                optional_deps: vec![
                    OptionalDep {
                        name: "zlib".to_owned(),
                        optional_for: "compression".to_owned(),
                        installed_package: None,
                    },
                    OptionalDep {
                        name: "tesseract-data-eng".to_owned(),
                        optional_for: "OCR language data".to_owned(),
                        installed_package: None,
                    },
                ],
            },
            PackageInfo {
                name: "beta".to_owned(),
                version: "1.0.0".to_owned(),
                description: None,
                optional_deps: vec![
                    OptionalDep {
                        name: "sqlite".to_owned(),
                        optional_for: "database".to_owned(),
                        installed_package: None,
                    },
                    OptionalDep {
                        name: "zlib".to_owned(),
                        optional_for: "compression".to_owned(),
                        installed_package: None,
                    },
                ],
            },
        ];

        let missing = missing_optional_deps_from_packages(&packages);

        assert_eq!(
            missing
                .iter()
                .map(|dep| dep.name.as_str())
                .collect::<Vec<_>>(),
            vec!["zlib", "sqlite"]
        );
    }

    #[test]
    fn missing_optional_deps_skip_unresolved_syncdb_dependencies() {
        let packages = vec![PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![
                OptionalDep {
                    name: "zlib".to_owned(),
                    optional_for: "compression".to_owned(),
                    installed_package: None,
                },
                OptionalDep {
                    name: "journalctl-desktop-notification".to_owned(),
                    optional_for: "desktop notifications".to_owned(),
                    installed_package: None,
                },
            ],
        }];

        let missing = missing_optional_deps_from_packages_with_resolver(&packages, |dep| {
            Some(match dep.name.as_str() {
                "zlib" => OptionalDepResolution::Resolved(MissingOptionalDep {
                    name: dep.name.clone(),
                    version: Some("1.3.1-2".to_owned()),
                    description: Some("Compression library".to_owned()),
                    wanted_by: Vec::new(),
                }),
                _ => OptionalDepResolution::Unresolved,
            })
        });

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "zlib");
    }
}
