//! OS counters for the isolated kernel probe. Darwin's V4 layout follows the
//! SDK's sys/resource.h; other platforms report unavailable, never zero data.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Usage {
    pub resident_bytes: u64,
    pub physical_footprint_bytes: u64,
    pub lifetime_max_physical_footprint_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
    pub darwin_ri_logical_writes: u64,
}

#[cfg(target_os = "macos")]
pub fn sample() -> Option<Usage> {
    #[repr(C)]
    struct RusageV4 {
        uuid: [u8; 16],
        fields: [u64; 35],
    }
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut *mut std::ffi::c_void) -> i32;
    }
    let mut info = RusageV4 {
        uuid: [0; 16],
        fields: [0; 35],
    };
    // SAFETY: RUSAGE_INFO_V4 writes exactly the SDK's repr(C) V4 buffer;
    // its UUID and 35 uint64 fields have the documented size/alignment. The
    // buffer remains alive and writable for the duration of this synchronous
    // call. pid belongs to this probe, so no process inspection is delegated.
    let result = unsafe {
        proc_pid_rusage(
            std::process::id() as i32,
            4,
            (&mut info as *mut RusageV4).cast(),
        )
    };
    (result == 0).then(|| Usage {
        resident_bytes: info.fields[6],
        physical_footprint_bytes: info.fields[7],
        lifetime_max_physical_footprint_bytes: info.fields[28],
        disk_read_bytes: info.fields[16],
        disk_written_bytes: info.fields[17],
        darwin_ri_logical_writes: info.fields[27],
    })
}

#[cfg(not(target_os = "macos"))]
pub fn sample() -> Option<Usage> {
    None
}
