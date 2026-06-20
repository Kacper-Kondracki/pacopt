use std::collections::BTreeMap;
use std::fs;

use alpm::{Alpm, Dep, DepMod, SigLevel};
use eyre::Result;

use crate::model::{
    InstalledPackage, MissingOptionalDep, OptionalDep, OptionalDepRequester, PackageInfo,
};

const IGNORED_MISSING_OPTIONAL_DEP_PREFIXES: &[&str] = &["tesseract-data-", "tesarract-data-"];
const IGNORED_MISSING_OPTIONAL_DEPS: &[&str] = &["tesseract-data", "tesarract-data"];

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

pub(crate) fn missing_optional_deps_from_packages(
    packages: &[PackageInfo],
) -> Vec<MissingOptionalDep> {
    missing_optional_deps_from_packages_with_resolver(packages, |_| None)
}

fn missing_optional_deps_from_packages_and_syncdbs(
    packages: &[PackageInfo],
    syncdbs: &alpm::AlpmList<'_, &alpm::Db>,
) -> Vec<MissingOptionalDep> {
    missing_optional_deps_from_packages_with_resolver(packages, |dep| {
        syncdbs
            .find_satisfier(dep.name.as_str())
            .map(|pkg| MissingOptionalDep {
                name: pkg.name().to_owned(),
                version: Some(pkg.version().to_string()),
                description: pkg.desc().map(str::to_owned),
                wanted_by: Vec::new(),
            })
    })
}

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

pub(crate) fn dep_names_match(candidate: &Dep, requested: &Dep) -> bool {
    candidate.name() == requested.name()
}

fn installed_package_from_alpm(pkg: &alpm::Package) -> InstalledPackage {
    InstalledPackage {
        name: pkg.name().to_owned(),
        version: pkg.version().to_string(),
    }
}

fn missing_optional_deps_from_packages_with_resolver<F>(
    packages: &[PackageInfo],
    mut resolve: F,
) -> Vec<MissingOptionalDep>
where
    F: FnMut(&OptionalDep) -> Option<MissingOptionalDep>,
{
    let mut deps = BTreeMap::<String, MissingOptionalDep>::new();

    for package in packages {
        for dep in package
            .optional_deps
            .iter()
            .filter(|dep| dep.installed_package.is_none())
        {
            let mut resolved = resolve(dep).unwrap_or_else(|| MissingOptionalDep {
                name: dep.name.clone(),
                version: None,
                description: None,
                wanted_by: Vec::new(),
            });
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

fn is_ignored_missing_optional_dep(name: &str) -> bool {
    IGNORED_MISSING_OPTIONAL_DEPS.contains(&name)
        || IGNORED_MISSING_OPTIONAL_DEP_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

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
}
