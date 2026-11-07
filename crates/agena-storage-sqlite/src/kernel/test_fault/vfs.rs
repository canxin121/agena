//! A named, opt-in SQLite VFS for one case's database connections.
//!
//! SQLite owns each sqlite3_file allocation. We keep its layout and original
//! methods intact, replacing only the method table pointer while it is open.
//! Every unselected operation delegates to the original VFS. Registrations are
//! process-lifetime objects (SQLite retains their pointers), never the default.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::PathBuf,
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

use libsqlite3_sys as ffi;

#[derive(Clone)]
struct Registration {
    name: String,
    original: usize,
    control: Arc<PathBuf>,
}

struct FileHook {
    original: usize,
    methods: usize,
    control: Arc<PathBuf>,
    file: String,
}

impl Drop for FileHook {
    fn drop(&mut self) {
        // xClose has restored the original table before the final reference is
        // dropped. SQLite will never access this replacement table again.
        unsafe {
            drop(Box::from_raw(self.methods as *mut ffi::sqlite3_io_methods));
        }
    }
}

static REGISTRATIONS: OnceLock<Mutex<HashMap<usize, Registration>>> = OnceLock::new();
static FILES: OnceLock<Mutex<HashMap<usize, Arc<FileHook>>>> = OnceLock::new();

fn registration(vfs: *mut ffi::sqlite3_vfs) -> Option<Registration> {
    REGISTRATIONS
        .get()?
        .lock()
        .ok()?
        .get(&(vfs as usize))
        .cloned()
}

fn file_hook(file: *mut ffi::sqlite3_file) -> Option<Arc<FileHook>> {
    FILES.get()?.lock().ok()?.get(&(file as usize)).cloned()
}

pub(super) fn register(control: PathBuf) -> Result<String, String> {
    let mut registrations = REGISTRATIONS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "fault VFS registry lock poisoned")?;
    if let Some(existing) = registrations
        .values()
        .find(|value| *value.control == control)
    {
        return Ok(existing.name.clone());
    }
    // SAFETY: SQLite copies callbacks but retains the VFS/name addresses. Both
    // are leaked only after successful registration and kept alive until exit.
    unsafe {
        let original = ffi::sqlite3_vfs_find(ptr::null());
        if original.is_null() {
            return Err("SQLite has no default VFS".into());
        }
        let name = format!("agena-test-{}", uuid::Uuid::new_v4());
        let c_name = CString::new(name.as_str()).map_err(|error| error.to_string())?;
        let mut wrapper = Box::new(ptr::read(original));
        wrapper.zName = c_name.as_ptr();
        wrapper.pNext = ptr::null_mut();
        wrapper.xOpen = Some(open);
        // Forward with the original VFS pointer, preserving its private state
        // even when a platform implementation relies on pointer identity.
        wrapper.xDelete = (*original).xDelete.map(|_| delete as _);
        wrapper.xAccess = (*original).xAccess.map(|_| access as _);
        wrapper.xFullPathname = (*original).xFullPathname.map(|_| full_path as _);
        wrapper.xDlOpen = (*original).xDlOpen.map(|_| dl_open as _);
        wrapper.xDlError = (*original).xDlError.map(|_| dl_error as _);
        wrapper.xDlSym = (*original).xDlSym.map(|_| dl_sym as _);
        wrapper.xDlClose = (*original).xDlClose.map(|_| dl_close as _);
        wrapper.xRandomness = (*original).xRandomness.map(|_| randomness as _);
        wrapper.xSleep = (*original).xSleep.map(|_| sleep as _);
        wrapper.xCurrentTime = (*original).xCurrentTime.map(|_| current_time as _);
        wrapper.xGetLastError = (*original).xGetLastError.map(|_| last_error as _);
        wrapper.xCurrentTimeInt64 = (*original).xCurrentTimeInt64.map(|_| current_time_i64 as _);
        wrapper.xSetSystemCall = (*original).xSetSystemCall.map(|_| set_system_call as _);
        wrapper.xGetSystemCall = (*original).xGetSystemCall.map(|_| get_system_call as _);
        wrapper.xNextSystemCall = (*original).xNextSystemCall.map(|_| next_system_call as _);
        let pointer = &mut *wrapper as *mut ffi::sqlite3_vfs;
        let rc = ffi::sqlite3_vfs_register(pointer, 0);
        if rc != ffi::SQLITE_OK {
            return Err(format!("register test SQLite VFS: {rc}"));
        }
        registrations.insert(
            pointer as usize,
            Registration {
                name: name.clone(),
                original: original as usize,
                control: Arc::new(control),
            },
        );
        let _ = c_name.into_raw();
        let _ = Box::into_raw(wrapper);
        Ok(name)
    }
}

fn injected(control: &std::path::Path, point: &str, file: &str) -> Option<c_int> {
    match super::take(control, point, Some(file)) {
        Ok(Some((_, state))) => state.sqlite_code,
        Ok(None) => None,
        Err(error) => {
            tracing::error!(%error, point, "SQLite fault control failed");
            Some(ffi::SQLITE_IOERR)
        }
    }
}

unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let Some(reg) = registration(vfs) else {
        return ffi::SQLITE_CANTOPEN;
    };
    let database_file = flags
        & (ffi::SQLITE_OPEN_MAIN_DB | ffi::SQLITE_OPEN_MAIN_JOURNAL | ffi::SQLITE_OPEN_WAL)
        != 0;
    // SAFETY: SQLite supplies a nul-terminated filename for these named files.
    let filename = if name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    };
    if database_file && let Some(code) = injected(&reg.control, "sqlite.open", &filename) {
        unsafe {
            (*file).pMethods = ptr::null();
        }
        return code;
    }
    let original = reg.original as *mut ffi::sqlite3_vfs;
    let Some(call) = (unsafe { (*original).xOpen }) else {
        return ffi::SQLITE_CANTOPEN;
    };
    let result = unsafe { call(original, name, file, flags, out_flags) };
    if result != ffi::SQLITE_OK || !database_file || unsafe { (*file).pMethods.is_null() } {
        return result;
    }
    // The file allocation and all platform fields remain exactly as returned by
    // xOpen. This indirection cannot change the layout expected by the platform.
    let methods = unsafe { (*file).pMethods };
    let mut replacement = Box::new(unsafe { ptr::read(methods) });
    replacement.xClose = Some(close);
    replacement.xRead = unsafe { (*methods).xRead }.map(|_| read as _);
    replacement.xWrite = unsafe { (*methods).xWrite }.map(|_| write as _);
    replacement.xSync = unsafe { (*methods).xSync }.map(|_| sync as _);
    replacement.xTruncate = unsafe { (*methods).xTruncate }.map(|_| truncate as _);
    let pointer = Box::into_raw(replacement);
    let hook = Arc::new(FileHook {
        original: methods as usize,
        methods: pointer as usize,
        control: reg.control,
        file: filename,
    });
    match FILES.get_or_init(Default::default).lock() {
        Ok(mut files) => {
            files.insert(file as usize, hook);
            unsafe {
                (*file).pMethods = pointer;
            }
            result
        }
        Err(_) => {
            if let Some(close) = unsafe { (*methods).xClose } {
                unsafe {
                    close(file);
                }
            }
            unsafe {
                (*file).pMethods = ptr::null();
            }
            ffi::SQLITE_IOERR
        }
    }
}

unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    let hook = FILES
        .get()
        .and_then(|files| files.lock().ok()?.remove(&(file as usize)));
    let Some(hook) = hook else {
        return ffi::SQLITE_IOERR;
    };
    let original = hook.original as *const ffi::sqlite3_io_methods;
    unsafe {
        (*file).pMethods = original;
    }
    match unsafe { (*original).xClose } {
        Some(call) => unsafe { call(file) },
        None => ffi::SQLITE_OK,
    }
}

macro_rules! file_operation {
    ($name:ident, $field:ident, $point:literal, ($($arg:ident: $ty:ty),*)) => {
        unsafe extern "C" fn $name(file: *mut ffi::sqlite3_file, $($arg: $ty),*) -> c_int {
            let Some(hook) = file_hook(file) else { return ffi::SQLITE_IOERR; };
            if let Some(code) = injected(&hook.control, $point, &hook.file) { return code; }
            let original = hook.original as *const ffi::sqlite3_io_methods;
            match unsafe { (*original).$field } {
                Some(call) => unsafe { call(file, $($arg),*) },
                None => ffi::SQLITE_IOERR,
            }
        }
    };
}
file_operation!(read, xRead, "sqlite.read", (buffer: *mut c_void, length: c_int, offset: ffi::sqlite3_int64));
file_operation!(write, xWrite, "sqlite.write", (buffer: *const c_void, length: c_int, offset: ffi::sqlite3_int64));
file_operation!(sync, xSync, "sqlite.sync", (flags: c_int));
file_operation!(truncate, xTruncate, "sqlite.truncate", (size: ffi::sqlite3_int64));

macro_rules! vfs_operation {
    // C void callbacks have no return type or value to propagate.
    ($name:ident, $field:ident, ($($arg:ident: $ty:ty),*), (), {}) => {
        unsafe extern "C" fn $name(vfs: *mut ffi::sqlite3_vfs, $($arg: $ty),*) {
            let Some(reg) = registration(vfs) else { return; };
            let original = reg.original as *mut ffi::sqlite3_vfs;
            if let Some(call) = unsafe { (*original).$field } {
                unsafe { call(original, $($arg),*); }
            }
        }
    };
    // Optional C callbacks have an Option return as well as an optional
    // function pointer; propagate absence at either boundary explicitly.
    ($name:ident, $field:ident, ($($arg:ident: $ty:ty),*), $ret:ty, None) => {
        unsafe extern "C" fn $name(vfs: *mut ffi::sqlite3_vfs, $($arg: $ty),*) -> $ret {
            let reg = registration(vfs)?;
            let original = reg.original as *mut ffi::sqlite3_vfs;
            let call = unsafe { (*original).$field }?;
            unsafe { call(original, $($arg),*) }
        }
    };
    ($name:ident, $field:ident, ($($arg:ident: $ty:ty),*), $ret:ty, $failure:expr) => {
        unsafe extern "C" fn $name(vfs: *mut ffi::sqlite3_vfs, $($arg: $ty),*) -> $ret {
            let Some(reg) = registration(vfs) else { return $failure; };
            let original = reg.original as *mut ffi::sqlite3_vfs;
            match unsafe { (*original).$field } {
                Some(call) => unsafe { call(original, $($arg),*) },
                None => $failure,
            }
        }
    };
}
type DlSymbol = Option<unsafe extern "C" fn(*mut ffi::sqlite3_vfs, *mut c_void, *const c_char)>;
vfs_operation!(delete, xDelete, (name: *const c_char, sync_dir: c_int), c_int, ffi::SQLITE_IOERR_DELETE);
vfs_operation!(access, xAccess, (name: *const c_char, flags: c_int, result: *mut c_int), c_int, ffi::SQLITE_IOERR_ACCESS);
vfs_operation!(full_path, xFullPathname, (name: *const c_char, length: c_int, result: *mut c_char), c_int, ffi::SQLITE_CANTOPEN);
vfs_operation!(dl_open, xDlOpen, (name: *const c_char), *mut c_void, ptr::null_mut());
vfs_operation!(dl_error, xDlError, (length: c_int, message: *mut c_char), (), {});
vfs_operation!(dl_sym, xDlSym, (handle: *mut c_void, symbol: *const c_char), DlSymbol, None);
vfs_operation!(dl_close, xDlClose, (handle: *mut c_void), (), {});
vfs_operation!(randomness, xRandomness, (length: c_int, result: *mut c_char), c_int, 0);
vfs_operation!(sleep, xSleep, (micros: c_int), c_int, 0);
vfs_operation!(current_time, xCurrentTime, (result: *mut f64), c_int, ffi::SQLITE_ERROR);
vfs_operation!(last_error, xGetLastError, (length: c_int, result: *mut c_char), c_int, 0);
vfs_operation!(current_time_i64, xCurrentTimeInt64, (result: *mut ffi::sqlite3_int64), c_int, ffi::SQLITE_ERROR);
vfs_operation!(set_system_call, xSetSystemCall, (name: *const c_char, function: ffi::sqlite3_syscall_ptr), c_int, ffi::SQLITE_NOTFOUND);
vfs_operation!(get_system_call, xGetSystemCall, (name: *const c_char), ffi::sqlite3_syscall_ptr, None);
vfs_operation!(next_system_call, xNextSystemCall, (name: *const c_char), *const c_char, ptr::null());

#[cfg(test)]
mod tests;
