//! Protected owner/System ACLs for application-private files and directories.

use std::{ffi::c_void, io, os::windows::ffi::OsStrExt, path::Path, ptr};

#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: *const c_void,
        present: *mut i32,
        acl: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
    fn SetNamedSecurityInfoW(
        name: *mut u16,
        object_type: u32,
        information: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: *mut c_void,
        sacl: *mut c_void,
    ) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

pub(super) fn restrict(path: &Path, directory: bool) -> io::Result<()> {
    // OW is the well-known Owner Rights SID. The protected DACL grants only
    // the object owner and LocalSystem access, independently of inherited ACLs.
    let sddl = if directory {
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;OW)"
    } else {
        "D:P(A;;FA;;;SY)(A;;FA;;;OW)"
    };
    let descriptor_string: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: all buffers remain valid for each Windows API call; LocalFree
    // releases the descriptor allocated by ConvertStringSecurityDescriptor.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_string.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut present = 0;
            let mut defaulted = 0;
            let mut acl = ptr::null_mut();
            if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) == 0 {
                return Err(io::Error::last_os_error());
            }
            if present == 0 || acl.is_null() {
                return Err(io::Error::other("private storage ACL is missing"));
            }
            let mut name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let error = SetNamedSecurityInfoW(
                name.as_mut_ptr(),
                1,
                0x00000004 | 0x80000000,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            );
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            Ok(())
        })();
        LocalFree(descriptor);
        result
    }
}
