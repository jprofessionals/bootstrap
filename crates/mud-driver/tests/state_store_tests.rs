use mud_driver::runtime::state_store::StateStore;
use mud_mop::message::Value;

#[test]
fn create_object_verify_it_exists() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/sword.lpc", "lpc");

    let obj = store.get(id).expect("object should exist after creation");
    assert_eq!(obj.id, id);
    assert_eq!(obj.program_path, "/world/sword.lpc");
    assert_eq!(obj.language, "lpc");
    assert_eq!(obj.version, 1);
    assert!(obj.core_properties.is_empty());
    assert!(obj.attached_properties.is_empty());
    assert_eq!(obj.location, None);
}

#[test]
fn set_and_get_core_property() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/npc.lpc", "lpc");

    let ok = store.set_property(id, "name", Value::String("Goblin".into()));
    assert!(ok, "set_property should return true for existing object");

    let val = store
        .get_property(id, "name")
        .expect("property should exist");
    assert_eq!(*val, Value::String("Goblin".into()));
}

#[test]
fn set_property_on_nonexistent_object_returns_false() {
    let mut store = StateStore::new();
    let ok = store.set_property(9999, "name", Value::String("Ghost".into()));
    assert!(!ok);
}

#[test]
fn get_property_on_nonexistent_object_returns_none() {
    let store = StateStore::new();
    assert!(store.get_property(9999, "name").is_none());
}

#[test]
fn get_property_missing_key_returns_none() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/npc.lpc", "lpc");
    assert!(store.get_property(id, "nonexistent").is_none());
}

#[test]
fn attach_property_with_source_and_retrieve_it() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/room.lpc", "lpc");

    let ok = store.attach_property(id, "weather_system", "temperature", Value::Int(72));
    assert!(ok, "attach_property should return true for existing object");

    let val = store
        .get_attached(id, "weather_system", "temperature")
        .expect("attached property should exist");
    assert_eq!(*val, Value::Int(72));
}

#[test]
fn attach_property_updates_in_place_on_same_source_and_key() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/room.lpc", "lpc");

    store.attach_property(id, "weather", "temp", Value::Int(70));
    store.attach_property(id, "weather", "temp", Value::Int(85));

    let val = store.get_attached(id, "weather", "temp").unwrap();
    assert_eq!(*val, Value::Int(85));

    // Should still be only one attached property, not two
    let obj = store.get(id).unwrap();
    assert_eq!(obj.attached_properties.len(), 1);
}

#[test]
fn attach_property_on_nonexistent_object_returns_false() {
    let mut store = StateStore::new();
    let ok = store.attach_property(9999, "src", "key", Value::Null);
    assert!(!ok);
}

#[test]
fn remove_attached_properties_by_source() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/room.lpc", "lpc");

    store.attach_property(id, "weather", "temperature", Value::Int(72));
    store.attach_property(id, "weather", "humidity", Value::Int(45));
    store.attach_property(id, "lighting", "brightness", Value::Int(100));

    store.remove_attached_by_source(id, "weather");

    // Weather properties should be gone
    assert!(store.get_attached(id, "weather", "temperature").is_none());
    assert!(store.get_attached(id, "weather", "humidity").is_none());

    // Lighting property should remain
    let val = store
        .get_attached(id, "lighting", "brightness")
        .expect("lighting property should survive");
    assert_eq!(*val, Value::Int(100));
}

#[test]
fn remove_object() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/item.lpc", "lpc");

    assert!(store.get(id).is_some());

    let removed = store.remove_object(id);
    assert!(
        removed,
        "remove_object should return true for existing object"
    );

    assert!(
        store.get(id).is_none(),
        "object should not exist after removal"
    );
}

#[test]
fn remove_nonexistent_object_returns_false() {
    let mut store = StateStore::new();
    assert!(!store.remove_object(9999));
}

#[test]
fn upgrade_program_version() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/npc.lpc", "lpc");

    assert_eq!(store.get(id).unwrap().version, 1);

    store.upgrade_program(id, 5);
    assert_eq!(store.get(id).unwrap().version, 5);
}

#[test]
fn query_objects_by_program_path() {
    let mut store = StateStore::new();
    let id1 = store.create_object("/world/goblin.lpc", "lpc");
    let id2 = store.create_object("/world/goblin.lpc", "lpc");
    let _id3 = store.create_object("/world/dragon.lpc", "lpc");

    let mut goblins = store.objects_by_program("/world/goblin.lpc");
    goblins.sort();
    let mut expected = vec![id1, id2];
    expected.sort();

    assert_eq!(goblins, expected);
}

#[test]
fn objects_by_program_returns_empty_for_unknown_program() {
    let store = StateStore::new();
    assert!(store.objects_by_program("/world/nothing.lpc").is_empty());
}

#[test]
fn set_and_get_location() {
    let mut store = StateStore::new();
    let room_id = store.create_object("/world/room.lpc", "lpc");
    let player_id = store.create_object("/world/player.lpc", "lpc");

    assert_eq!(store.get(player_id).unwrap().location, None);

    store.set_location(player_id, Some(room_id));
    assert_eq!(store.get(player_id).unwrap().location, Some(room_id));

    // Un-locate
    store.set_location(player_id, None);
    assert_eq!(store.get(player_id).unwrap().location, None);
}

#[test]
fn multiple_objects_with_different_programs() {
    let mut store = StateStore::new();
    let id_lpc = store.create_object("/world/npc.lpc", "lpc");
    let id_ruby = store.create_object("/world/portal.rb", "ruby");
    let id_java = store.create_object("/world/handler.java", "jvm");

    assert_eq!(store.get(id_lpc).unwrap().language, "lpc");
    assert_eq!(store.get(id_ruby).unwrap().language, "ruby");
    assert_eq!(store.get(id_java).unwrap().language, "jvm");

    // Each should have a unique id
    assert_ne!(id_lpc, id_ruby);
    assert_ne!(id_ruby, id_java);
    assert_ne!(id_lpc, id_java);
}

#[test]
fn attached_properties_survive_after_core_property_changes() {
    let mut store = StateStore::new();
    let id = store.create_object("/world/npc.lpc", "lpc");

    // Attach a property first
    store.attach_property(id, "quest_system", "quest_giver", Value::Bool(true));

    // Now modify core properties
    store.set_property(id, "name", Value::String("Vendor".into()));
    store.set_property(id, "hp", Value::Int(100));
    store.set_property(id, "name", Value::String("Merchant".into())); // overwrite

    // Attached property should still be there
    let val = store
        .get_attached(id, "quest_system", "quest_giver")
        .expect("attached property should survive core property changes");
    assert_eq!(*val, Value::Bool(true));

    // Core property should reflect the latest value
    let name = store.get_property(id, "name").unwrap();
    assert_eq!(*name, Value::String("Merchant".into()));
}

#[test]
fn allocate_id_increments() {
    let mut store = StateStore::new();
    let id1 = store.allocate_id();
    let id2 = store.allocate_id();
    assert_eq!(id2, id1 + 1);
}

#[test]
fn create_object_ids_are_sequential() {
    let mut store = StateStore::new();
    let id1 = store.create_object("/a.lpc", "lpc");
    let id2 = store.create_object("/b.lpc", "lpc");
    let id3 = store.create_object("/c.lpc", "lpc");
    assert_eq!(id2, id1 + 1);
    assert_eq!(id3, id2 + 1);
}
