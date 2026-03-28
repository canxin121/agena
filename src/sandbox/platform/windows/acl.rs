#![allow(unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Foundation::HLOCAL;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::ACCESS_ALLOWED_ACE;
use windows_sys::Win32::Security::ACE_HEADER;
use windows_sys::Win32::Security::ACL;
use windows_sys::Win32::Security::ACL_SIZE_INFORMATION;
use windows_sys::Win32::Security::AclSizeInformation;
use windows_sys::Win32::Security::Authorization::EXPLICIT_ACCESS_W;
use windows_sys::Win32::Security::Authorization::GetEffectiveRightsFromAclW;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::GetSecurityInfo;
use windows_sys::Win32::Security::Authorization::SetEntriesInAclW;
use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_SID;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_UNKNOWN;
use windows_sys::Win32::Security::Authorization::TRUSTEE_W;
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::Security::EqualSid;
use windows_sys::Win32::Security::GetAce;
use windows_sys::Win32::Security::GetAclInformation;
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows_sys::Win32::Storage::FileSystem::OPEN_EXISTING;

use crate::sandbox::SandboxError;

use super::util::to_wide;

const SE_KERNEL_OBJECT: u32 = 6;
const INHERIT_ONLY_ACE: u8 = 0x08;

pub(super) struct AclRollback {
    sid: *mut c_void,
    paths: Vec<PathBuf>,
}

impl AclRollback {
    pub(super) fn new(sid: *mut c_void) -> Self {
        Self {
            sid,
            paths: Vec::new(),
        }
    }

    pub(super) fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for AclRollback {
    fn drop(&mut self) {
        unsafe {
            for path in &self.paths {
                revoke_ace(path, self.sid);
            }
        }
    }
}

pub(super) unsafe fn dacl_has_write_allow_for_sid(p_dacl: *mut ACL, sid: *mut c_void) -> bool {
    if p_dacl.is_null() {
        return false;
    }

    let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
    let ok = GetAclInformation(
        p_dacl as *const ACL,
        &mut info as *mut _ as *mut c_void,
        std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
        AclSizeInformation,
    );
    if ok == 0 {
        return false;
    }

    for index in 0..(info.AceCount as usize) {
        let mut p_ace: *mut c_void = std::ptr::null_mut();
        if GetAce(p_dacl as *const ACL, index as u32, &mut p_ace) == 0 {
            continue;
        }

        let header = &*(p_ace as *const ACE_HEADER);
        if header.AceType != 0 {
            continue;
        }
        if (header.AceFlags & INHERIT_ONLY_ACE) != 0 {
            continue;
        }

        let ace = &*(p_ace as *const ACCESS_ALLOWED_ACE);
        let base = p_ace as usize;
        let sid_ptr =
            (base + std::mem::size_of::<ACE_HEADER>() + std::mem::size_of::<u32>()) as *mut c_void;

        if EqualSid(sid_ptr, sid) != 0 && (ace.Mask & FILE_GENERIC_WRITE) != 0 {
            return true;
        }
    }

    false
}

pub(super) unsafe fn dacl_effective_allows_write(p_dacl: *mut ACL, sid: *mut c_void) -> bool {
    if p_dacl.is_null() {
        return false;
    }

    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: sid as *mut u16,
    };

    let mut rights: u32 = 0;
    let ok = GetEffectiveRightsFromAclW(p_dacl, &trustee, &mut rights);
    if ok != 0 {
        let write_bits = FILE_GENERIC_WRITE
            | windows_sys::Win32::Storage::FileSystem::FILE_WRITE_DATA
            | windows_sys::Win32::Storage::FileSystem::FILE_APPEND_DATA
            | windows_sys::Win32::Storage::FileSystem::FILE_WRITE_EA
            | windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES;
        return (rights & write_bits) != 0;
    }

    dacl_has_write_allow_for_sid(p_dacl, sid)
}

pub(super) unsafe fn add_allow_ace(path: &Path, sid: *mut c_void) -> Result<bool, SandboxError> {
    let mut p_sd: *mut c_void = std::ptr::null_mut();
    let mut p_dacl: *mut ACL = std::ptr::null_mut();
    let code = GetNamedSecurityInfoW(
        to_wide(path).as_ptr(),
        1,
        DACL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut p_dacl,
        std::ptr::null_mut(),
        &mut p_sd,
    );
    if code != ERROR_SUCCESS {
        return Err(SandboxError::Windows(format!(
            "GetNamedSecurityInfoW failed with code {code} for {}",
            path.display()
        )));
    }

    let mut added = false;
    if !dacl_has_write_allow_for_sid(p_dacl, sid) {
        let trustee = TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        };

        let mut explicit: EXPLICIT_ACCESS_W = std::mem::zeroed();
        explicit.grfAccessPermissions =
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE;
        explicit.grfAccessMode = 2;
        explicit.grfInheritance = windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
            | windows_sys::Win32::Security::OBJECT_INHERIT_ACE;
        explicit.Trustee = trustee;

        let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
        let code2 = SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl);
        if code2 == ERROR_SUCCESS {
            let code3 = SetNamedSecurityInfoW(
                to_wide(path).as_ptr() as *mut u16,
                1,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                p_new_dacl,
                std::ptr::null_mut(),
            );
            if code3 == ERROR_SUCCESS {
                added = true;
            }

            if !p_new_dacl.is_null() {
                LocalFree(p_new_dacl as HLOCAL);
            }
        }
    }

    if !p_sd.is_null() {
        LocalFree(p_sd as HLOCAL);
    }
    Ok(added)
}

pub(super) unsafe fn add_deny_write_ace(
    path: &Path,
    sid: *mut c_void,
) -> Result<bool, SandboxError> {
    let mut p_sd: *mut c_void = std::ptr::null_mut();
    let mut p_dacl: *mut ACL = std::ptr::null_mut();
    let code = GetNamedSecurityInfoW(
        to_wide(path).as_ptr(),
        1,
        DACL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut p_dacl,
        std::ptr::null_mut(),
        &mut p_sd,
    );
    if code != ERROR_SUCCESS {
        return Err(SandboxError::Windows(format!(
            "GetNamedSecurityInfoW failed with code {code} for {}",
            path.display()
        )));
    }

    let mut added = false;
    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: sid as *mut u16,
    };
    let mut explicit: EXPLICIT_ACCESS_W = std::mem::zeroed();
    explicit.grfAccessPermissions = FILE_GENERIC_WRITE;
    explicit.grfAccessMode = 3;
    explicit.grfInheritance = windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
        | windows_sys::Win32::Security::OBJECT_INHERIT_ACE;
    explicit.Trustee = trustee;

    let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
    let code2 = SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl);
    if code2 == ERROR_SUCCESS {
        let code3 = SetNamedSecurityInfoW(
            to_wide(path).as_ptr() as *mut u16,
            1,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            p_new_dacl,
            std::ptr::null_mut(),
        );
        if code3 == ERROR_SUCCESS {
            added = true;
        }

        if !p_new_dacl.is_null() {
            LocalFree(p_new_dacl as HLOCAL);
        }
    }

    if !p_sd.is_null() {
        LocalFree(p_sd as HLOCAL);
    }
    Ok(added)
}

pub(super) unsafe fn revoke_ace(path: &Path, sid: *mut c_void) {
    let mut p_sd: *mut c_void = std::ptr::null_mut();
    let mut p_dacl: *mut ACL = std::ptr::null_mut();
    let code = GetNamedSecurityInfoW(
        to_wide(path).as_ptr(),
        1,
        DACL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut p_dacl,
        std::ptr::null_mut(),
        &mut p_sd,
    );
    if code != ERROR_SUCCESS {
        if !p_sd.is_null() {
            LocalFree(p_sd as HLOCAL);
        }
        return;
    }

    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: sid as *mut u16,
    };
    let mut explicit: EXPLICIT_ACCESS_W = std::mem::zeroed();
    explicit.grfAccessPermissions = 0;
    explicit.grfAccessMode = 4;
    explicit.grfInheritance = windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
        | windows_sys::Win32::Security::OBJECT_INHERIT_ACE;
    explicit.Trustee = trustee;

    let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
    let code2 = SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl);
    if code2 == ERROR_SUCCESS {
        let _ = SetNamedSecurityInfoW(
            to_wide(path).as_ptr() as *mut u16,
            1,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            p_new_dacl,
            std::ptr::null_mut(),
        );
        if !p_new_dacl.is_null() {
            LocalFree(p_new_dacl as HLOCAL);
        }
    }

    if !p_sd.is_null() {
        LocalFree(p_sd as HLOCAL);
    }
}

pub(super) unsafe fn allow_null_device(sid: *mut c_void) {
    let desired = 0x0002_0000 | 0x0004_0000;
    let handle = CreateFileW(
        to_wide(r"\\.\NUL").as_ptr(),
        desired,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        std::ptr::null_mut(),
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        0,
    );
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        return;
    }

    let mut p_sd: *mut c_void = std::ptr::null_mut();
    let mut p_dacl: *mut ACL = std::ptr::null_mut();
    let code = GetSecurityInfo(
        handle,
        SE_KERNEL_OBJECT as i32,
        DACL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut p_dacl,
        std::ptr::null_mut(),
        &mut p_sd,
    );
    if code == ERROR_SUCCESS {
        let trustee = TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        };
        let mut explicit: EXPLICIT_ACCESS_W = std::mem::zeroed();
        explicit.grfAccessPermissions =
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE;
        explicit.grfAccessMode = 2;
        explicit.grfInheritance = 0;
        explicit.Trustee = trustee;

        let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
        if SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl) == ERROR_SUCCESS {
            let _ = SetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT as i32,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                p_new_dacl,
                std::ptr::null_mut(),
            );
            if !p_new_dacl.is_null() {
                LocalFree(p_new_dacl as HLOCAL);
            }
        }
    }

    if !p_sd.is_null() {
        LocalFree(p_sd as HLOCAL);
    }
    CloseHandle(handle);
}
