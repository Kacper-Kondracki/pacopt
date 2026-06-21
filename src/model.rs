//! Data structures used by the package scanner and terminal UI.

/// An installed package and the optional dependencies declared by that package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    /// Package name from the local pacman database.
    pub name: String,
    /// Installed package version.
    pub version: String,
    /// Package description, when pacman has one.
    pub description: Option<String>,
    /// Optional dependencies declared by this package.
    pub optional_deps: Vec<OptionalDep>,
}

/// An optional dependency declared by an installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDep {
    /// Dependency requirement as pacman would resolve it.
    ///
    /// This may include a version constraint, such as `foo>=1.0`.
    pub name: String,
    /// Human-readable reason shown after the dependency in package metadata.
    pub optional_for: String,
    /// Installed package satisfying this optional dependency, if any.
    pub installed_package: Option<InstalledPackage>,
}

/// A locally installed package that satisfies an optional dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// Package name from the local pacman database.
    pub name: String,
    /// Installed package version.
    pub version: String,
}

/// An optional dependency that is requested by installed packages but not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingOptionalDep {
    /// Package name from the sync database, or the unresolved dependency name.
    pub name: String,
    /// Available package version from a sync database, when the dependency was resolved.
    pub version: Option<String>,
    /// Package description from a sync database, when available.
    pub description: Option<String>,
    /// Installed packages that declare this dependency as optional.
    pub wanted_by: Vec<OptionalDepRequester>,
}

/// A package that declares a missing optional dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDepRequester {
    /// Name of the installed package that wants the dependency.
    pub package_name: String,
    /// Reason text from the package's optional dependency metadata.
    pub reason: String,
}

impl PackageInfo {
    /// Returns `true` when this package or any of its optional dependencies match `query`.
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.version.to_lowercase().contains(query)
            || self
                .description
                .as_deref()
                .is_some_and(|description| description.to_lowercase().contains(query))
            || self.optional_deps.iter().any(|dep| dep.matches(query))
    }
}

impl OptionalDep {
    /// Returns the dependency reason, or a fallback when pacman did not provide one.
    pub(crate) fn reason(&self) -> String {
        if self.optional_for.is_empty() {
            "No reason provided.".to_owned()
        } else {
            self.optional_for.clone()
        }
    }

    /// Returns `true` when this dependency, its reason, or its satisfier match `query`.
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.optional_for.to_lowercase().contains(query)
            || self
                .installed_package
                .as_ref()
                .is_some_and(|package| package.matches(query))
    }
}

impl MissingOptionalDep {
    /// Returns `true` when this missing dependency or any requester match `query`.
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self
                .version
                .as_ref()
                .is_some_and(|version| version.to_lowercase().contains(query))
            || self
                .description
                .as_ref()
                .is_some_and(|description| description.to_lowercase().contains(query))
            || self.wanted_by.iter().any(|requester| {
                requester.package_name.to_lowercase().contains(query)
                    || requester.reason.to_lowercase().contains(query)
            })
    }
}

impl InstalledPackage {
    /// Returns `true` when the package name or version match `query`.
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query) || self.version.to_lowercase().contains(query)
    }
}
