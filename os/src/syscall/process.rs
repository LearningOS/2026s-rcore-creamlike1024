//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// Get time with second and microsecond, writes into user-space TimeVal struct.
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    let token = current_user_token();
    let us = crate::timer::get_time_us();
    let time_val = translated_refmut(token, ts);
    *time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    0
}

/// Implement mmap: map anonymous pages into the process address space.
/// start must be page-aligned, prot must be nonzero and a subset of 0b111.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    use crate::config::PAGE_SIZE;
    use crate::mm::{MapPermission, VirtAddr};
    // Validate: start must be page-aligned
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    // port must be nonzero and not have bits beyond [0..2]
    if port & !0x7 != 0 || port & 0x7 == 0 {
        return -1;
    }
    if len == 0 {
        return -1;
    }
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    // Check no overlap with existing mappings
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();
    for vpn in crate::mm::VPNRange::new(start_vpn, end_vpn) {
        if inner.memory_set.translate(vpn).map_or(false, |e| e.is_valid()) {
            return -1;
        }
    }
    // Build permission: always U, plus R/W/X from port bits [0..2]
    let mut perm = MapPermission::U;
    if port & 1 != 0 { perm |= MapPermission::R; }
    if port & 2 != 0 { perm |= MapPermission::W; }
    if port & 4 != 0 { perm |= MapPermission::X; }
    inner.memory_set.insert_framed_area(start_va, end_va, perm);
    0
}

/// Implement munmap: unmap pages from the process address space.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    use crate::config::PAGE_SIZE;
    use crate::mm::VirtAddr;
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    if len == 0 {
        return -1;
    }
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    // Verify all pages in the range are mapped
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();
    for vpn in crate::mm::VPNRange::new(start_vpn, end_vpn) {
        if !inner.memory_set.translate(vpn).map_or(false, |e| e.is_valid()) {
            return -1;
        }
    }
    inner.memory_set.remove_area_with_start_vpn(start_vpn);
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// Implement spawn: create a new child process by loading the given ELF directly.
/// The new process inherits the parent's fd table (stdin/stdout/stderr).
pub fn sys_spawn(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let current = current_task().unwrap();
        // Create new task from ELF (gets fresh stdin/stdout/stderr)
        let new_task = Arc::new(crate::task::TaskControlBlock::new(all_data.as_slice()));
        // Inherit the parent's fd table and register as child — all in one lock
        {
            let mut parent_inner = current.inner_exclusive_access();
            let mut new_inner = new_task.inner_exclusive_access();
            // Replace the default fd table with parent's
            new_inner.fd_table.clear();
            for fd in parent_inner.fd_table.iter() {
                if let Some(file) = fd {
                    new_inner.fd_table.push(Some(file.clone()));
                } else {
                    new_inner.fd_table.push(None);
                }
            }
            new_inner.parent = Some(Arc::downgrade(&current));
            parent_inner.children.push(new_task.clone());
        }
        let new_pid = new_task.pid.0;
        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}


// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}
