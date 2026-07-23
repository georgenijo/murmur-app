use super::{update_cpu_percent, CpuTicks};
use crate::MutexExt;
use std::os::raw::c_int;
use std::sync::Mutex;

#[repr(C)]
#[derive(Default)]
struct HostCpuLoadInfo {
    ticks: [u32; 4], // user, system, idle, nice
}

const HOST_CPU_LOAD_INFO: c_int = 3;
const HOST_CPU_LOAD_INFO_COUNT: u32 = 4; // 4 x u32
const KERN_SUCCESS: c_int = 0;

unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn mach_task_self() -> u32;
    fn mach_port_deallocate(task: u32, name: u32) -> c_int;
    fn host_statistics64(
        host: u32,
        flavor: c_int,
        info: *mut HostCpuLoadInfo,
        count: *mut u32,
    ) -> c_int;
}

fn snapshot() -> Option<CpuTicks> {
    let mut info = HostCpuLoadInfo::default();
    let mut count = HOST_CPU_LOAD_INFO_COUNT;
    let host = unsafe { mach_host_self() };
    let result = unsafe { host_statistics64(host, HOST_CPU_LOAD_INFO, &mut info, &mut count) };
    unsafe { mach_port_deallocate(mach_task_self(), host) };
    if result != KERN_SUCCESS {
        return None;
    }

    Some(CpuTicks::new(
        info.ticks[0] as u64 + info.ticks[1] as u64 + info.ticks[3] as u64,
        info.ticks[2] as u64,
    ))
}

static PREVIOUS: Mutex<Option<CpuTicks>> = Mutex::new(None);

pub(super) fn cpu_percent() -> Option<f32> {
    let current = snapshot();
    update_cpu_percent(&mut PREVIOUS.lock_or_recover(), current)
}
