#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub optional_deps: Vec<OptionalDep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDep {
    pub name: String,
    pub optional_for: String,
    pub installed_package: Option<InstalledPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingOptionalDep {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub wanted_by: Vec<OptionalDepRequester>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDepRequester {
    pub package_name: String,
    pub reason: String,
}

impl PackageInfo {
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
    pub(crate) fn reason(&self) -> String {
        if self.optional_for.is_empty() {
            "No reason provided.".to_owned()
        } else {
            self.optional_for.clone()
        }
    }

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
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query) || self.version.to_lowercase().contains(query)
    }
}
