//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;

/// A large constant used by stride scheduling.
/// BIG_STRIDE / priority gives the pass (increment) for each schedule.
/// Using a value that fits in usize and avoids overflow for reasonable runs.
const BIG_STRIDE: usize = 1_000_000;

/// Task manager that holds the ready queue and implements stride scheduling.
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }

    /// Pick and remove the task with the smallest stride (stride scheduling).
    /// On a tie, any of the tied tasks may be chosen.
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }
        // Find the index of the task with the minimum stride.
        let min_idx = self
            .ready_queue
            .iter()
            .enumerate()
            .min_by_key(|(_, task)| task.inner_exclusive_access().stride)
            .map(|(idx, _)| idx)
            .unwrap(); // safe: queue is non-empty

        let task = self.ready_queue.remove(min_idx).unwrap();

        // Advance the chosen task's stride by its pass value.
        {
            let mut inner = task.inner_exclusive_access();
            let pass = BIG_STRIDE / inner.priority;
            inner.stride = inner.stride.wrapping_add(pass);
        }

        Some(task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
