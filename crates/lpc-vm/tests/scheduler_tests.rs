use std::time::Duration;

use lpc_vm::bytecode::{LpcValue, ObjectRef};
use lpc_vm::scheduler::Scheduler;

fn make_obj(id: u64) -> ObjectRef {
    ObjectRef {
        id,
        path: format!("/obj/{}", id),
        is_lightweight: false,
    }
}

// =========================================================================
// Schedule and poll due calls
// =========================================================================

#[test]
fn schedule_and_poll_immediate() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    sched.schedule(
        obj.clone(),
        "tick".to_string(),
        vec![LpcValue::Int(42)],
        Duration::ZERO,
    );
    // Immediate delay should be due immediately
    let due = sched.poll_due();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].function, "tick");
    assert_eq!(due[0].object.id, 1);
    assert_eq!(due[0].args[0].as_int(), Some(42));
}

#[test]
fn poll_empty_scheduler() {
    let mut sched = Scheduler::new();
    let due = sched.poll_due();
    assert!(due.is_empty());
}

// =========================================================================
// Cancel returns remaining delay
// =========================================================================

#[test]
fn cancel_returns_some() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    let handle = sched.schedule(
        obj,
        "later".to_string(),
        vec![],
        Duration::from_secs(3600), // far in the future
    );
    let remaining = sched.cancel(handle);
    assert!(remaining.is_some());
    // Remaining should be close to 3600 seconds
    let r = remaining.unwrap();
    assert!(r.as_secs() > 3500);
}

#[test]
fn cancel_nonexistent_returns_none() {
    let mut sched = Scheduler::new();
    let remaining = sched.cancel(9999);
    assert!(remaining.is_none());
}

#[test]
fn cancel_prevents_polling() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    let handle = sched.schedule(obj, "tick".to_string(), vec![], Duration::ZERO);
    sched.cancel(handle);
    let due = sched.poll_due();
    assert!(due.is_empty());
}

// =========================================================================
// Multiple calls ordered by deadline
// =========================================================================

#[test]
fn calls_ordered_by_deadline() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);

    // Schedule in reverse order: later first, earlier second
    sched.schedule(obj.clone(), "second".to_string(), vec![], Duration::ZERO);
    sched.schedule(obj.clone(), "first".to_string(), vec![], Duration::ZERO);

    // Both should fire since delay is ZERO
    let due = sched.poll_due();
    assert_eq!(due.len(), 2);
    // Both are due simultaneously; ordering by handle (first scheduled = lower handle)
    assert_eq!(due[0].function, "second");
    assert_eq!(due[1].function, "first");
}

// =========================================================================
// Remove calls for destructed object
// =========================================================================

#[test]
fn remove_for_object() {
    let mut sched = Scheduler::new();
    let obj1 = make_obj(1);
    let obj2 = make_obj(2);

    sched.schedule(obj1.clone(), "tick1".to_string(), vec![], Duration::ZERO);
    sched.schedule(obj2.clone(), "tick2".to_string(), vec![], Duration::ZERO);
    sched.schedule(obj1.clone(), "tick3".to_string(), vec![], Duration::ZERO);

    // Remove all calls for object 1
    sched.remove_for_object(1);

    let due = sched.poll_due();
    // Only object 2's call should remain
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].function, "tick2");
    assert_eq!(due[0].object.id, 2);
}

// =========================================================================
// Pending count
// =========================================================================

#[test]
fn pending_count() {
    let mut sched = Scheduler::new();
    assert_eq!(sched.pending_count(), 0);

    let obj = make_obj(1);
    sched.schedule(
        obj.clone(),
        "a".to_string(),
        vec![],
        Duration::from_secs(3600),
    );
    sched.schedule(
        obj.clone(),
        "b".to_string(),
        vec![],
        Duration::from_secs(3600),
    );
    assert_eq!(sched.pending_count(), 2);

    // Cancel one
    let h = sched.schedule(obj, "c".to_string(), vec![], Duration::from_secs(3600));
    sched.cancel(h);
    assert_eq!(sched.pending_count(), 2); // 3 queued - 1 cancelled = 2
}

// =========================================================================
// next_deadline
// =========================================================================

#[test]
fn next_deadline_empty() {
    let sched = Scheduler::new();
    assert!(sched.next_deadline().is_none());
}

#[test]
fn next_deadline_with_entry() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    sched.schedule(obj, "tick".to_string(), vec![], Duration::from_secs(10));
    let deadline = sched.next_deadline();
    assert!(deadline.is_some());
    assert!(deadline.unwrap().as_secs() <= 10);
}

// =========================================================================
// Schedule returns unique handles
// =========================================================================

#[test]
fn handles_are_unique() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    let h1 = sched.schedule(obj.clone(), "a".to_string(), vec![], Duration::ZERO);
    let h2 = sched.schedule(obj.clone(), "b".to_string(), vec![], Duration::ZERO);
    let h3 = sched.schedule(obj, "c".to_string(), vec![], Duration::ZERO);
    assert_ne!(h1, h2);
    assert_ne!(h2, h3);
    assert_ne!(h1, h3);
}

// =========================================================================
// Double cancel returns None
// =========================================================================

#[test]
fn double_cancel() {
    let mut sched = Scheduler::new();
    let obj = make_obj(1);
    let handle = sched.schedule(obj, "tick".to_string(), vec![], Duration::from_secs(3600));
    let first = sched.cancel(handle);
    assert!(first.is_some());
    let second = sched.cancel(handle);
    assert!(second.is_none());
}
