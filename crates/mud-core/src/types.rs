use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for in-game objects.
pub type ObjectId = u64;

/// Unique identifier for player sessions.
pub type SessionId = u64;

/// Supported scripting languages for area code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Ruby,
    Python,
    JavaScript,
    Lpc,
}

/// Identifies an area by namespace and name.
///
/// Areas live under `<namespace>/<name>` and may have a dev variant
/// indicated by a `@dev` suffix on the name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AreaId {
    pub namespace: String,
    pub name: String,
}

impl AreaId {
    /// Create a new AreaId.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    /// Return the canonical key in `"namespace/name"` form.
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    /// Check whether this area id refers to a dev checkout (name ends with `@dev`).
    pub fn is_dev(&self) -> bool {
        self.name.ends_with("@dev")
    }
}

impl fmt::Display for AreaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_id_new() {
        let id = AreaId::new("system", "lobby");
        assert_eq!(id.namespace, "system");
        assert_eq!(id.name, "lobby");
    }

    #[test]
    fn area_id_new_from_string() {
        let id = AreaId::new(String::from("ns"), String::from("area"));
        assert_eq!(id.namespace, "ns");
        assert_eq!(id.name, "area");
    }

    #[test]
    fn area_id_key() {
        let id = AreaId::new("game", "tavern");
        assert_eq!(id.key(), "game/tavern");
    }

    #[test]
    fn area_id_display() {
        let id = AreaId::new("system", "lobby");
        assert_eq!(format!("{}", id), "system/lobby");
    }

    #[test]
    fn area_id_is_dev_false() {
        let id = AreaId::new("game", "tavern");
        assert!(!id.is_dev());
    }

    #[test]
    fn area_id_is_dev_true() {
        let id = AreaId::new("game", "tavern@dev");
        assert!(id.is_dev());
    }

    #[test]
    fn area_id_equality() {
        let a = AreaId::new("ns", "area");
        let b = AreaId::new("ns", "area");
        assert_eq!(a, b);
    }

    #[test]
    fn area_id_inequality() {
        let a = AreaId::new("ns1", "area");
        let b = AreaId::new("ns2", "area");
        assert_ne!(a, b);
    }

    #[test]
    fn area_id_clone() {
        let a = AreaId::new("ns", "area");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn area_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(AreaId::new("ns", "a"));
        set.insert(AreaId::new("ns", "b"));
        set.insert(AreaId::new("ns", "a")); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn area_id_serialization() {
        let id = AreaId::new("game", "tavern");
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.contains("game"));
        assert!(json.contains("tavern"));

        let deserialized: AreaId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn language_variants() {
        let _ruby = Language::Ruby;
        let _python = Language::Python;
        let _js = Language::JavaScript;
        let _lpc = Language::Lpc;
    }

    #[test]
    fn language_equality() {
        assert_eq!(Language::Ruby, Language::Ruby);
        assert_ne!(Language::Ruby, Language::Python);
    }

    #[test]
    fn language_serialization() {
        let lang = Language::Ruby;
        let json = serde_json::to_string(&lang).unwrap();
        let deserialized: Language = serde_json::from_str(&json).unwrap();
        assert_eq!(lang, deserialized);
    }
}
