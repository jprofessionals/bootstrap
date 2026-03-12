//! Dynamic module (.so) loader for extending the driver with native kfuns.

use std::collections::HashMap;

use crate::kfun::KfunFn;

/// Type of the initialization function exported by each `.so` module.
///
/// Each module must export:
/// ```c
/// pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar)
/// ```
pub type ModuleInitFn = unsafe extern "C" fn(&mut ModuleRegistrar);

/// Data collected during module initialization. The module's `mud_module_init`
/// function populates this via the builder methods.
pub struct ModuleRegistrar {
    pub path: String,
    pub version: u64,
    pub dependencies: Vec<String>,
    pub kfuns: Vec<(String, KfunFn)>,
}

impl ModuleRegistrar {
    /// Create an empty registrar.
    pub fn new() -> Self {
        ModuleRegistrar {
            path: String::new(),
            version: 0,
            dependencies: Vec::new(),
            kfuns: Vec::new(),
        }
    }

    /// Set the logical path of this module (e.g. "/usr/mudlib/modules/foo").
    pub fn set_path(&mut self, path: &str) {
        self.path = path.to_string();
    }

    /// Set the version number of this module.
    pub fn set_version(&mut self, version: u64) {
        self.version = version;
    }

    /// Declare a dependency on another module path.
    pub fn add_dependency(&mut self, dep: &str) {
        self.dependencies.push(dep.to_string());
    }

    /// Register a kernel function implemented by this module.
    pub fn register_kfun(&mut self, name: &str, func: KfunFn) {
        self.kfuns.push((name.to_string(), func));
    }
}

impl Default for ModuleRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a loaded dynamic module.
pub struct LoadedModule {
    path: String,
    so_path: String,
    version: u64,
    dependencies: Vec<String>,
    kfun_names: Vec<String>,
    _library: libloading::Library,
}

impl LoadedModule {
    /// Logical path of this module.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Filesystem path of the `.so` file.
    pub fn so_path(&self) -> &str {
        &self.so_path
    }

    /// Module version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Module dependencies.
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    /// Names of kfuns registered by this module.
    pub fn kfun_names(&self) -> &[String] {
        &self.kfun_names
    }
}

/// Errors from module loading operations.
#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("failed to load library: {0}")]
    LoadError(String),
    #[error("module init function not found")]
    InitNotFound,
    #[error("module not loaded: {0}")]
    NotLoaded(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Loader for native `.so` extension modules.
pub struct ModuleLoader {
    loaded: HashMap<String, LoadedModule>,
}

impl ModuleLoader {
    /// Create a new, empty module loader.
    pub fn new() -> Self {
        ModuleLoader {
            loaded: HashMap::new(),
        }
    }

    /// Load a `.so` module from disk.
    ///
    /// Calls the module's `mud_module_init` function to collect registrations,
    /// then stores the module and returns the registrar so the caller can wire
    /// the kfuns into the [`KfunRegistry`](crate::kfun::KfunRegistry).
    pub fn load(&mut self, so_path: &str) -> Result<ModuleRegistrar, ModuleError> {
        // Safety: we trust that the .so exports a valid mud_module_init symbol
        // with the correct signature.
        let library = unsafe { libloading::Library::new(so_path) }
            .map_err(|e| ModuleError::LoadError(e.to_string()))?;

        let init_fn: libloading::Symbol<ModuleInitFn> =
            unsafe { library.get(b"mud_module_init") }.map_err(|_| ModuleError::InitNotFound)?;

        let mut registrar = ModuleRegistrar::new();
        unsafe {
            init_fn(&mut registrar);
        }

        let kfun_names: Vec<String> = registrar.kfuns.iter().map(|(n, _)| n.clone()).collect();

        let module = LoadedModule {
            path: registrar.path.clone(),
            so_path: so_path.to_string(),
            version: registrar.version,
            dependencies: registrar.dependencies.clone(),
            kfun_names,
            _library: library,
        };

        self.loaded.insert(registrar.path.clone(), module);
        Ok(registrar)
    }

    /// Reload a module: unload the old version, then load a (potentially new)
    /// `.so`. Returns the new registrar.
    pub fn reload(&mut self, path: &str, so_path: &str) -> Result<ModuleRegistrar, ModuleError> {
        // Remove the old module (drop closes the library handle).
        self.loaded.remove(path);
        self.load(so_path)
    }

    /// Unload a module.
    pub fn unload(&mut self, path: &str) -> Result<(), ModuleError> {
        self.loaded
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| ModuleError::NotLoaded(path.to_string()))
    }

    /// Check if a module is loaded.
    pub fn is_loaded(&self, path: &str) -> bool {
        self.loaded.contains_key(path)
    }

    /// Get all loaded module paths.
    pub fn loaded_modules(&self) -> Vec<&str> {
        self.loaded.keys().map(|s| s.as_str()).collect()
    }

    /// Get module info by logical path.
    pub fn get_module(&self, path: &str) -> Option<&LoadedModule> {
        self.loaded.get(path)
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}
