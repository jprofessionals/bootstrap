use std::collections::{HashMap, HashSet, VecDeque};

/// Metadata for a single registered program.
pub struct ProgramInfo {
    pub path: String,
    pub version: u64,
    pub language: String,
    pub dependencies: Vec<String>,
}

/// Tracks program versions and the dependency graph between programs.
///
/// When a program is reloaded the driver needs to know which *other* programs
/// depend on it (its transitive dependents) so it can decide whether to cascade
/// the reload or invalidate cached results.  The `VersionTree` maintains this
/// graph and provides breadth-first traversal of dependents.
pub struct VersionTree {
    programs: HashMap<String, ProgramInfo>,
    /// Reverse-edge index: for each program path, the set of programs that
    /// list it as a dependency.
    dependents: HashMap<String, HashSet<String>>,
}

impl VersionTree {
    pub fn new() -> Self {
        Self {
            programs: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Register a program (or re-register it) with its dependency list.
    ///
    /// If the program was already registered its dependency edges are replaced.
    pub fn register(&mut self, path: &str, language: &str, deps: Vec<String>) {
        // Remove old reverse edges if re-registering.
        if let Some(old) = self.programs.get(path) {
            for dep in &old.dependencies {
                if let Some(set) = self.dependents.get_mut(dep) {
                    set.remove(path);
                }
            }
        }

        // Insert forward edges.
        let version = self.programs.get(path).map(|p| p.version).unwrap_or(1);

        // Insert reverse edges.
        for dep in &deps {
            self.dependents
                .entry(dep.clone())
                .or_default()
                .insert(path.to_string());
        }

        self.programs.insert(
            path.to_string(),
            ProgramInfo {
                path: path.to_string(),
                version,
                language: language.to_string(),
                dependencies: deps,
            },
        );
    }

    /// Remove a program and all its dependency edges (both forward and reverse).
    pub fn unregister(&mut self, path: &str) {
        if let Some(info) = self.programs.remove(path) {
            // Remove forward edges (this program depends on others).
            for dep in &info.dependencies {
                if let Some(set) = self.dependents.get_mut(dep) {
                    set.remove(path);
                }
            }
        }
        // Remove reverse edges (others that depend on this program).
        self.dependents.remove(path);
    }

    /// Bump the version number for a program.  Returns the new version, or
    /// `None` if the program is not registered.
    pub fn bump_version(&mut self, path: &str) -> Option<u64> {
        let info = self.programs.get_mut(path)?;
        info.version += 1;
        Some(info.version)
    }

    /// Return the direct dependents of a program.
    pub fn get_dependents(&self, path: &str) -> Vec<&str> {
        self.dependents
            .get(path)
            .map(|set| set.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Walk all transitive dependents of `path` using breadth-first search.
    ///
    /// The starting program itself is *not* included in the result.
    pub fn walk_dependents(&self, path: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut result = Vec::new();

        // Seed with direct dependents.
        if let Some(direct) = self.dependents.get(path) {
            for dep in direct {
                if visited.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }

        while let Some(current) = queue.pop_front() {
            result.push(current.clone());
            if let Some(next) = self.dependents.get(&current) {
                for dep in next {
                    if visited.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        result
    }

    /// Get program info by path.
    pub fn get(&self, path: &str) -> Option<&ProgramInfo> {
        self.programs.get(path)
    }

    /// Get the language for a program.
    pub fn language_for(&self, path: &str) -> Option<&str> {
        self.programs.get(path).map(|p| p.language.as_str())
    }

    /// Get the current version number for a program.
    pub fn version_of(&self, path: &str) -> Option<u64> {
        self.programs.get(path).map(|p| p.version)
    }

    /// List all registered program paths.
    pub fn all_programs(&self) -> Vec<&str> {
        self.programs.keys().map(|s| s.as_str()).collect()
    }

    /// List programs filtered by language.
    pub fn programs_by_language(&self, language: &str) -> Vec<&str> {
        self.programs
            .values()
            .filter(|p| p.language == language)
            .map(|p| p.path.as_str())
            .collect()
    }
}
