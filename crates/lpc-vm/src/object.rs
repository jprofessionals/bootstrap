use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use crate::bytecode::{CompiledProgram, LpcValue, ObjectRef};

// Modifier flag constants (must match compiler.rs modifier_to_flag).
const MOD_PRIVATE: u8 = 0x01;
#[allow(dead_code)]
const MOD_STATIC: u8 = 0x02;
#[allow(dead_code)]
const MOD_NOMASK: u8 = 0x04;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectError {
    NotFound(String),
    NotMaster(String),
    AlreadyDestroyed(String),
    InvalidClone(u64),
    InheritanceCycle(String),
}

impl fmt::Display for ObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::NotFound(p) => write!(f, "object not found: {}", p),
            ObjectError::NotMaster(p) => write!(f, "not a master object: {}", p),
            ObjectError::AlreadyDestroyed(p) => write!(f, "object already destroyed: {}", p),
            ObjectError::InvalidClone(id) => write!(f, "invalid clone id: {}", id),
            ObjectError::InheritanceCycle(p) => write!(f, "inheritance cycle detected: {}", p),
        }
    }
}

impl std::error::Error for ObjectError {}

// ---------------------------------------------------------------------------
// Dependency Graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// For each program path, the set of paths that inherit from it.
    dependents: HashMap<String, HashSet<String>>,
    /// For each program path, what it inherits from.
    dependencies: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            dependents: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    /// Register that `path` depends on (inherits from) `parent_path`.
    pub fn add_dependency(&mut self, path: &str, parent_path: &str) {
        self.dependencies
            .entry(path.to_string())
            .or_default()
            .push(parent_path.to_string());
        self.dependents
            .entry(parent_path.to_string())
            .or_default()
            .insert(path.to_string());
    }

    /// Remove all dependencies for a path.
    pub fn remove(&mut self, path: &str) {
        // Remove this path from the dependents sets of all its parents.
        if let Some(parents) = self.dependencies.remove(path) {
            for parent in &parents {
                if let Some(set) = self.dependents.get_mut(parent) {
                    set.remove(path);
                }
            }
        }
        // Also remove the dependents entry for this path (children still exist,
        // but they will be re-registered if they are recompiled).
        // We do NOT remove dependents[path] because children still reference us.
    }

    /// Get direct dependents of a path.
    pub fn get_dependents(&self, path: &str) -> Vec<String> {
        self.dependents
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Walk all transitive dependents (breadth-first).
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
}

// ---------------------------------------------------------------------------
// Master / Clone objects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MasterObject {
    pub path: String,
    pub program: CompiledProgram,
    pub globals: Vec<LpcValue>,
    pub version: u64,
    pub clone_ids: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct CloneObject {
    pub id: u64,
    pub master_path: String,
    pub globals: Vec<LpcValue>,
    pub is_lightweight: bool,
}

// ---------------------------------------------------------------------------
// Object Table
// ---------------------------------------------------------------------------

pub struct ObjectTable {
    masters: HashMap<String, MasterObject>,
    clones: HashMap<u64, CloneObject>,
    next_clone_id: u64,
    dependency_graph: DependencyGraph,
}

impl ObjectTable {
    pub fn new() -> Self {
        Self {
            masters: HashMap::new(),
            clones: HashMap::new(),
            next_clone_id: 1,
            dependency_graph: DependencyGraph::new(),
        }
    }

    /// Register a compiled program as a master object.
    pub fn register_master(&mut self, program: CompiledProgram) -> ObjectRef {
        let path = program.path.clone();
        let global_count = program.global_count as usize;

        // Register dependencies from inherits list.
        for parent in &program.inherits {
            self.dependency_graph.add_dependency(&path, parent);
        }

        let master = MasterObject {
            path: path.clone(),
            program,
            globals: vec![LpcValue::Nil; global_count],
            version: 1,
            clone_ids: Vec::new(),
        };

        self.masters.insert(path.clone(), master);

        ObjectRef {
            id: 0,
            path,
            is_lightweight: false,
        }
    }

    /// Get a master object by path.
    pub fn get_master(&self, path: &str) -> Option<&MasterObject> {
        self.masters.get(path)
    }

    /// Get a mutable reference to a master object.
    pub fn get_master_mut(&mut self, path: &str) -> Option<&mut MasterObject> {
        self.masters.get_mut(path)
    }

    /// Clone a master object. Returns the clone's ObjectRef.
    pub fn clone_object(&mut self, master_path: &str) -> Result<ObjectRef, ObjectError> {
        let master = self
            .masters
            .get_mut(master_path)
            .ok_or_else(|| ObjectError::NotFound(master_path.to_string()))?;

        let id = self.next_clone_id;
        self.next_clone_id += 1;

        let globals = master.globals.clone();
        master.clone_ids.push(id);

        let clone = CloneObject {
            id,
            master_path: master_path.to_string(),
            globals,
            is_lightweight: false,
        };
        self.clones.insert(id, clone);

        Ok(ObjectRef {
            id,
            path: master_path.to_string(),
            is_lightweight: false,
        })
    }

    /// Create a lightweight object from a master.
    pub fn new_lightweight(&mut self, master_path: &str) -> Result<ObjectRef, ObjectError> {
        let master = self
            .masters
            .get_mut(master_path)
            .ok_or_else(|| ObjectError::NotFound(master_path.to_string()))?;

        let id = self.next_clone_id;
        self.next_clone_id += 1;

        let globals = master.globals.clone();
        master.clone_ids.push(id);

        let clone = CloneObject {
            id,
            master_path: master_path.to_string(),
            globals,
            is_lightweight: true,
        };
        self.clones.insert(id, clone);

        Ok(ObjectRef {
            id,
            path: master_path.to_string(),
            is_lightweight: true,
        })
    }

    /// Find object by path (master) or by clone ID encoded as "path#N".
    pub fn find_object(&self, path: &str) -> Option<ObjectRef> {
        // Try "path#id" format first.
        if let Some(hash_pos) = path.rfind('#') {
            let id_str = &path[hash_pos + 1..];
            if let Ok(id) = id_str.parse::<u64>() {
                return self.find_by_id(id);
            }
        }

        // Plain path — look up master.
        if self.masters.contains_key(path) {
            Some(ObjectRef {
                id: 0,
                path: path.to_string(),
                is_lightweight: false,
            })
        } else {
            None
        }
    }

    /// Find a clone by its numeric ID.
    pub fn find_by_id(&self, id: u64) -> Option<ObjectRef> {
        self.clones.get(&id).map(|c| ObjectRef {
            id: c.id,
            path: c.master_path.clone(),
            is_lightweight: c.is_lightweight,
        })
    }

    /// Get the object name string (path for master, path#N for clone, path#-1 for LWO).
    pub fn object_name(&self, obj: &ObjectRef) -> String {
        if obj.id == 0 {
            // Master object.
            obj.path.clone()
        } else if obj.is_lightweight {
            format!("{}#-1", obj.path)
        } else {
            format!("{}#{}", obj.path, obj.id)
        }
    }

    /// Destroy an object.
    pub fn destruct(&mut self, obj: &ObjectRef) -> Result<(), ObjectError> {
        if obj.id == 0 {
            // Destructing a master: remove it and all its clones.
            let master = self
                .masters
                .remove(&obj.path)
                .ok_or_else(|| ObjectError::NotFound(obj.path.clone()))?;

            for clone_id in &master.clone_ids {
                self.clones.remove(clone_id);
            }

            self.dependency_graph.remove(&obj.path);
            Ok(())
        } else {
            // Destructing a clone.
            let clone = self
                .clones
                .remove(&obj.id)
                .ok_or_else(|| ObjectError::InvalidClone(obj.id))?;

            // Remove from master's clone list.
            if let Some(master) = self.masters.get_mut(&clone.master_path) {
                master.clone_ids.retain(|&id| id != obj.id);
            }

            Ok(())
        }
    }

    /// Check if an object is a master (not a clone).
    pub fn is_master(&self, obj: &ObjectRef) -> bool {
        obj.id == 0 && self.masters.contains_key(&obj.path)
    }

    /// Get a global variable on an object.
    pub fn get_global(&self, obj: &ObjectRef, index: u16) -> Result<&LpcValue, ObjectError> {
        let globals = if obj.id == 0 {
            &self
                .masters
                .get(&obj.path)
                .ok_or_else(|| ObjectError::NotFound(obj.path.clone()))?
                .globals
        } else {
            &self
                .clones
                .get(&obj.id)
                .ok_or_else(|| ObjectError::InvalidClone(obj.id))?
                .globals
        };

        globals
            .get(index as usize)
            .ok_or_else(|| ObjectError::NotFound(format!("global index {}", index)))
    }

    /// Set a global variable on an object.
    pub fn set_global(
        &mut self,
        obj: &ObjectRef,
        index: u16,
        value: LpcValue,
    ) -> Result<(), ObjectError> {
        let globals = if obj.id == 0 {
            &mut self
                .masters
                .get_mut(&obj.path)
                .ok_or_else(|| ObjectError::NotFound(obj.path.clone()))?
                .globals
        } else {
            &mut self
                .clones
                .get_mut(&obj.id)
                .ok_or_else(|| ObjectError::InvalidClone(obj.id))?
                .globals
        };

        let idx = index as usize;
        if idx >= globals.len() {
            globals.resize(idx + 1, LpcValue::Nil);
        }
        globals[idx] = value;
        Ok(())
    }

    /// Get the program for an object (follows clone -> master).
    pub fn get_program(&self, obj: &ObjectRef) -> Result<&CompiledProgram, ObjectError> {
        let path = if obj.id == 0 {
            &obj.path
        } else {
            let clone = self
                .clones
                .get(&obj.id)
                .ok_or_else(|| ObjectError::InvalidClone(obj.id))?;
            &clone.master_path
        };

        self.masters
            .get(path.as_str())
            .map(|m| &m.program)
            .ok_or_else(|| ObjectError::NotFound(path.to_string()))
    }

    /// Hot-reload: replace a master's program with a new version.
    /// Returns list of dependent paths that need upgrading.
    pub fn upgrade_program(
        &mut self,
        path: &str,
        new_program: CompiledProgram,
    ) -> Result<Vec<String>, ObjectError> {
        let master = self
            .masters
            .get_mut(path)
            .ok_or_else(|| ObjectError::NotFound(path.to_string()))?;

        // 1. Replace the program and bump version.
        let new_global_count = new_program.global_count as usize;
        master.program = new_program;
        master.version += 1;

        // 2. Resize master globals if the new program has a different count.
        //    Preserve existing values, pad with Nil, or truncate.
        master.globals.resize(new_global_count, LpcValue::Nil);

        // 3. Update clone globals similarly.
        let clone_ids: Vec<u64> = master.clone_ids.clone();
        for &cid in &clone_ids {
            if let Some(clone) = self.clones.get_mut(&cid) {
                // Preserve values, pad/truncate to new size.
                clone.globals.resize(new_global_count, LpcValue::Nil);
            }
        }

        // 4. Update dependency graph: remove old edges, add new ones.
        self.dependency_graph.remove(path);
        let inherits: Vec<String> = self
            .masters
            .get(path)
            .map(|m| m.program.inherits.clone())
            .unwrap_or_default();
        for parent in &inherits {
            self.dependency_graph.add_dependency(path, parent);
        }

        // 5. Walk transitive dependents.
        let dependents = self.dependency_graph.walk_dependents(path);

        Ok(dependents)
    }

    /// Check if a program inherits from another (transitive).
    pub fn inherits_from(&self, program_path: &str, ancestor_path: &str) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![program_path.to_string()];

        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(master) = self.masters.get(&current) {
                for parent in &master.program.inherits {
                    if parent == ancestor_path {
                        return true;
                    }
                    stack.push(parent.clone());
                }
            }
        }

        false
    }

    /// Get function by name, searching the inheritance chain.
    ///
    /// Returns `(defining_program_path, function_index)`.
    /// Searches the object's own program first, then walks the inheritance
    /// chain in declaration order.
    pub fn resolve_function(&self, program_path: &str, name: &str) -> Option<(String, usize)> {
        let mut visited = HashSet::new();
        self.resolve_function_inner(program_path, name, &mut visited, false)
    }

    /// Internal recursive function resolver.
    ///
    /// `from_child` indicates whether we are searching on behalf of a child
    /// program (affects visibility of `private` functions).
    fn resolve_function_inner(
        &self,
        program_path: &str,
        name: &str,
        visited: &mut HashSet<String>,
        from_child: bool,
    ) -> Option<(String, usize)> {
        if !visited.insert(program_path.to_string()) {
            return None;
        }

        let master = self.masters.get(program_path)?;

        // Search own functions.
        for (idx, func) in master.program.functions.iter().enumerate() {
            if func.name == name {
                // Visibility check: private functions only visible to the
                // defining program itself (not to children walking the chain).
                if from_child && func.modifiers.contains(&MOD_PRIVATE) {
                    continue;
                }
                return Some((program_path.to_string(), idx));
            }
        }

        // Walk inheritance chain in declaration order.
        for parent in &master.program.inherits {
            if let Some(result) = self.resolve_function_inner(parent, name, visited, true) {
                return Some(result);
            }
        }

        None
    }

    /// Find which program in the inheritance chain defines a function.
    pub fn function_origin(&self, program_path: &str, name: &str) -> Option<String> {
        self.resolve_function(program_path, name)
            .map(|(path, _)| path)
    }
}
