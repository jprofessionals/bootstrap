use mud_driver::web::build_log::{BuildLog, LogLevel};

#[test]
fn test_build_log_stores_and_retrieves_entries() {
    let log = BuildLog::new(100);
    log.append("testarea", LogLevel::Info, "compile", "Starting build");
    log.append(
        "testarea",
        LogLevel::Error,
        "compile",
        "Syntax error on line 5",
    );

    let entries = log.recent("testarea", 10, None);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].level, LogLevel::Info);
    assert_eq!(entries[0].event, "compile");
    assert_eq!(entries[0].message, "Starting build");
    assert_eq!(entries[1].level, LogLevel::Error);
    assert_eq!(entries[1].message, "Syntax error on line 5");
}

#[test]
fn test_build_log_filters_by_level() {
    let log = BuildLog::new(100);
    log.append("testarea", LogLevel::Info, "compile", "Starting build");
    log.append(
        "testarea",
        LogLevel::Error,
        "compile",
        "Syntax error on line 5",
    );

    let entries = log.recent("testarea", 10, Some(LogLevel::Error));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].level, LogLevel::Error);
    assert_eq!(entries[0].message, "Syntax error on line 5");
}

#[test]
fn test_build_log_respects_capacity() {
    let log = BuildLog::new(2);
    log.append("testarea", LogLevel::Info, "compile", "First");
    log.append("testarea", LogLevel::Info, "compile", "Second");
    log.append("testarea", LogLevel::Info, "compile", "Third");

    let entries = log.recent("testarea", 10, None);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "Second");
    assert_eq!(entries[1].message, "Third");
}

#[test]
fn test_build_log_separates_areas() {
    let log = BuildLog::new(100);
    log.append("area_a", LogLevel::Info, "compile", "Build A");
    log.append("area_b", LogLevel::Error, "deploy", "Deploy B");

    let entries_a = log.recent("area_a", 10, None);
    assert_eq!(entries_a.len(), 1);
    assert_eq!(entries_a[0].message, "Build A");

    let entries_b = log.recent("area_b", 10, None);
    assert_eq!(entries_b.len(), 1);
    assert_eq!(entries_b[0].message, "Deploy B");

    // Non-existent area returns empty
    let entries_c = log.recent("area_c", 10, None);
    assert!(entries_c.is_empty());
}
