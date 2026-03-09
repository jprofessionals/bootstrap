//! call_out scheduler: priority queue of delayed function calls.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::time::{Duration, Instant};

use crate::bytecode::{LpcValue, ObjectRef};

/// A scheduled function call waiting to fire.
struct ScheduledCall {
    handle: u64,
    execute_at: Instant,
    object: ObjectRef,
    function: String,
    args: Vec<LpcValue>,
}

// Reverse ordering so BinaryHeap acts as a min-heap (earliest deadline first).
impl Ord for ScheduledCall {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .execute_at
            .cmp(&self.execute_at)
            .then_with(|| self.handle.cmp(&other.handle))
    }
}

impl PartialOrd for ScheduledCall {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for ScheduledCall {}

impl PartialEq for ScheduledCall {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle
    }
}

/// A call that is due for execution, returned by [`Scheduler::poll_due`].
pub struct DueCall {
    pub handle: u64,
    pub object: ObjectRef,
    pub function: String,
    pub args: Vec<LpcValue>,
}

/// Priority-queue scheduler for `call_out` delayed function calls.
pub struct Scheduler {
    queue: BinaryHeap<ScheduledCall>,
    next_handle: u64,
    cancelled: HashSet<u64>,
}

impl Scheduler {
    /// Create a new, empty scheduler.
    pub fn new() -> Self {
        Scheduler {
            queue: BinaryHeap::new(),
            next_handle: 1,
            cancelled: HashSet::new(),
        }
    }

    /// Schedule a function call after `delay`. Returns a handle that can be
    /// used to cancel the call with [`cancel`](Self::cancel).
    pub fn schedule(
        &mut self,
        object: ObjectRef,
        function: String,
        args: Vec<LpcValue>,
        delay: Duration,
    ) -> u64 {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.queue.push(ScheduledCall {
            handle,
            execute_at: Instant::now() + delay,
            object,
            function,
            args,
        });
        handle
    }

    /// Cancel a scheduled call. Returns the remaining delay if the call was
    /// still pending, or `None` if it was not found (already executed or
    /// already cancelled).
    pub fn cancel(&mut self, handle: u64) -> Option<Duration> {
        // Check if it exists in the queue (not already cancelled).
        if self.cancelled.contains(&handle) {
            return None;
        }
        // Walk the queue to find the entry and compute remaining time.
        let now = Instant::now();
        let remaining = self.queue.iter().find(|c| c.handle == handle).map(|c| {
            if c.execute_at > now {
                c.execute_at - now
            } else {
                Duration::ZERO
            }
        });
        if remaining.is_some() {
            self.cancelled.insert(handle);
        }
        remaining
    }

    /// Drain all calls whose deadline has passed. Cancelled calls are silently
    /// skipped.
    pub fn poll_due(&mut self) -> Vec<DueCall> {
        let now = Instant::now();
        let mut due = Vec::new();
        while let Some(top) = self.queue.peek() {
            if top.execute_at > now {
                break;
            }
            let call = self.queue.pop().unwrap();
            if self.cancelled.remove(&call.handle) {
                continue; // was cancelled
            }
            due.push(DueCall {
                handle: call.handle,
                object: call.object,
                function: call.function,
                args: call.args,
            });
        }
        due
    }

    /// Duration until the next non-cancelled call fires, if any.
    pub fn next_deadline(&self) -> Option<Duration> {
        let now = Instant::now();
        // Skip cancelled entries at the front (peek only sees the top).
        for call in self.queue.iter() {
            if !self.cancelled.contains(&call.handle) {
                return Some(if call.execute_at > now {
                    call.execute_at - now
                } else {
                    Duration::ZERO
                });
            }
        }
        None
    }

    /// Number of pending (non-cancelled) calls.
    pub fn pending_count(&self) -> usize {
        self.queue.len() - self.cancelled.len()
    }

    /// Remove all scheduled calls for a specific object (e.g. on `destruct`).
    pub fn remove_for_object(&mut self, object_id: u64) {
        // Collect handles to cancel.
        let handles: Vec<u64> = self
            .queue
            .iter()
            .filter(|c| c.object.id == object_id && !self.cancelled.contains(&c.handle))
            .map(|c| c.handle)
            .collect();
        for h in handles {
            self.cancelled.insert(h);
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
