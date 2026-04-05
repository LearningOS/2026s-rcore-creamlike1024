//! Process management syscalls
use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = crate::timer::get_time_us();
    let time = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let token = crate::task::current_user_token();
    let buffers =
        crate::mm::translated_byte_buffer(token, ts as *const u8, core::mem::size_of::<TimeVal>());
    let mut ts_bytes = &time as *const _ as *const u8; // TimeVal -> *const u8
    for buffer in buffers {
        let len = buffer.len();
        unsafe {
            core::ptr::copy_nonoverlapping(ts_bytes, buffer.as_mut_ptr(), len);
            ts_bytes = ts_bytes.add(len);
        }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = crate::task::current_user_token();
    let page_table = crate::mm::PageTable::from_token(token); // 创建不含实际页表的映射
    let vpn = crate::mm::VirtAddr::from(id).floor(); // 从这个页表中查找虚拟页号

    let pte = page_table.translate(vpn); // 由 vpn 找到对应的 pte
    if pte.is_none() {
        return -1;
    }
    let pte = pte.unwrap();
    if !pte.is_valid() || !pte.flags().contains(crate::mm::PTEFlags::U) {
        return -1;
    }

    match trace_request {
        0 => {
            // read
            if !pte.readable() {
                return -1;
            }
            let ptr = id as *const u8;
            let buffers = crate::mm::translated_byte_buffer(token, ptr, 1);
            if buffers.is_empty() {
                return -1;
            }
            buffers[0][0] as isize
        }
        1 => {
            // write
            if !pte.writable() {
                return -1;
            }
            let ptr = id as *mut u8;
            let val = data as u8;
            let mut buffers = crate::mm::translated_byte_buffer(token, ptr, 1);
            if buffers.is_empty() {
                return -1;
            }
            buffers[0][0] = val;
            0
        }
        2 => {
            crate::task::get_syscall_count(id)
        }
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    crate::task::mmap(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    crate::task::munmap(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
