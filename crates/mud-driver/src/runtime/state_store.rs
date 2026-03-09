use std::collections::HashMap;

use mud_core::types::ObjectId;
use mud_mop::message::Value;

/// Per-object state held by the driver, independent of any language adapter.
pub struct ObjectState {
    pub id: ObjectId,
    pub program_path: String,
    pub language: String,
    pub version: u64,
    pub core_properties: HashMap<String, Value>,
    pub attached_properties: Vec<AttachedProperty>,
    pub location: Option<ObjectId>,
}

/// A property attached to an object by an external source (e.g. another program
/// or adapter-specific extension).  Removing all attached properties from a
/// given `source` is O(n) in the number of attached properties on that object.
pub struct AttachedProperty {
    pub source: String,
    pub key: String,
    pub value: Value,
}

/// Central object state storage.
///
/// The `StateStore` owns the canonical state for every in-game object.
/// Adapters may read/write properties via MOP calls, but the driver keeps the
/// authoritative copy here so that cross-adapter interactions see a consistent
/// view.
pub struct StateStore {
    objects: HashMap<ObjectId, ObjectState>,
    next_id: ObjectId,
}

impl StateStore {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            next_id: 1,
        }
    }

    /// Allocate the next unique object id without creating an object.
    pub fn allocate_id(&mut self) -> ObjectId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Create a new object backed by `program` in the given `language`.
    /// Returns the newly assigned object id.
    pub fn create_object(&mut self, program: &str, language: &str) -> ObjectId {
        let id = self.allocate_id();
        let state = ObjectState {
            id,
            program_path: program.to_string(),
            language: language.to_string(),
            version: 1,
            core_properties: HashMap::new(),
            attached_properties: Vec::new(),
            location: None,
        };
        self.objects.insert(id, state);
        id
    }

    /// Get an immutable reference to an object's state.
    pub fn get(&self, id: ObjectId) -> Option<&ObjectState> {
        self.objects.get(&id)
    }

    /// Get a mutable reference to an object's state.
    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut ObjectState> {
        self.objects.get_mut(&id)
    }

    /// Set a core property on an object.  Returns `true` if the object exists.
    pub fn set_property(&mut self, id: ObjectId, key: &str, value: Value) -> bool {
        if let Some(state) = self.objects.get_mut(&id) {
            state.core_properties.insert(key.to_string(), value);
            true
        } else {
            false
        }
    }

    /// Get a core property from an object.
    pub fn get_property(&self, id: ObjectId, key: &str) -> Option<&Value> {
        self.objects.get(&id)?.core_properties.get(key)
    }

    /// Attach a property from an external `source` to an object.
    /// If an attached property with the same source and key already exists it is
    /// updated in place.  Returns `true` if the object exists.
    pub fn attach_property(&mut self, id: ObjectId, source: &str, key: &str, value: Value) -> bool {
        if let Some(state) = self.objects.get_mut(&id) {
            // Update in place if already present.
            for prop in &mut state.attached_properties {
                if prop.source == source && prop.key == key {
                    prop.value = value;
                    return true;
                }
            }
            state.attached_properties.push(AttachedProperty {
                source: source.to_string(),
                key: key.to_string(),
                value,
            });
            true
        } else {
            false
        }
    }

    /// Look up an attached property by source and key.
    pub fn get_attached(&self, id: ObjectId, source: &str, key: &str) -> Option<&Value> {
        let state = self.objects.get(&id)?;
        state
            .attached_properties
            .iter()
            .find(|p| p.source == source && p.key == key)
            .map(|p| &p.value)
    }

    /// Remove all attached properties originating from `source` on the given object.
    pub fn remove_attached_by_source(&mut self, id: ObjectId, source: &str) {
        if let Some(state) = self.objects.get_mut(&id) {
            state.attached_properties.retain(|p| p.source != source);
        }
    }

    /// Destroy an object entirely.  Returns `true` if the object existed.
    pub fn remove_object(&mut self, id: ObjectId) -> bool {
        self.objects.remove(&id).is_some()
    }

    /// Bump an object to a new program version (e.g. after a hot-reload).
    pub fn upgrade_program(&mut self, id: ObjectId, new_version: u64) {
        if let Some(state) = self.objects.get_mut(&id) {
            state.version = new_version;
        }
    }

    /// Return the ids of all objects backed by the given `program` path.
    pub fn objects_by_program(&self, program: &str) -> Vec<ObjectId> {
        self.objects
            .values()
            .filter(|s| s.program_path == program)
            .map(|s| s.id)
            .collect()
    }

    /// Move an object into a new location (or `None` to un-locate it).
    pub fn set_location(&mut self, id: ObjectId, location: Option<ObjectId>) {
        if let Some(state) = self.objects.get_mut(&id) {
            state.location = location;
        }
    }
}
