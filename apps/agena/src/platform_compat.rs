//! Compatibility exports for older hosted targets used by the universal build.
//!
//! These symbols bridge APIs expected by current dependencies to the stable OS
//! primitives available in the pinned cross sysroots.  They are compiled only
//! for the affected targets and become part of the final Agena executable.

#[cfg(target_os = "netbsd")]
#[unsafe(no_mangle)]
unsafe extern "C" fn getentropy(buf: *mut libc::c_void, buflen: libc::size_t) -> libc::c_int {
    // getentropy(2) limits each request to 256 bytes.  AWS-LC already chunks
    // requests to this size, but preserve the system API's failure contract.
    if buflen > 256 {
        unsafe {
            *libc::__errno() = libc::EIO;
        }
        return -1;
    }

    // NetBSD 9.x predates libc getentropy, but arc4random_buf is the native
    // CSPRNG interface and requires no caller-managed state or file descriptor.
    unsafe {
        libc::arc4random_buf(buf, buflen);
    }
    0
}

#[cfg(target_os = "solaris")]
unsafe fn close_solaris_pty_pair(master: libc::c_int, slave: libc::c_int) -> libc::c_int {
    let errno = unsafe { *libc::___errno() };
    if slave >= 0 {
        unsafe {
            libc::close(slave);
        }
    }
    if master >= 0 {
        unsafe {
            libc::close(master);
        }
    }
    unsafe {
        *libc::___errno() = errno;
    }
    -1
}

#[cfg(target_os = "solaris")]
#[unsafe(no_mangle)]
unsafe extern "C" fn openpty(
    amaster: *mut libc::c_int,
    aslave: *mut libc::c_int,
    name: *mut libc::c_char,
    termp: *mut libc::termios,
    winp: *mut libc::winsize,
) -> libc::c_int {
    const PTEM: &[u8] = b"ptem\0";
    const LDTERM: &[u8] = b"ldterm\0";

    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master < 0 {
        return -1;
    }

    if unsafe { libc::grantpt(master) } < 0 || unsafe { libc::unlockpt(master) } < 0 {
        return unsafe { close_solaris_pty_pair(master, -1) };
    }

    let slave_path = unsafe { libc::ptsname(master) };
    if slave_path.is_null() {
        return unsafe { close_solaris_pty_pair(master, -1) };
    }

    let slave = unsafe { libc::open(slave_path, libc::O_RDWR | libc::O_NOCTTY) };
    if slave < 0 {
        return unsafe { close_solaris_pty_pair(master, -1) };
    }

    // Solaris pseudo-terminals use STREAMS.  Ensure the terminal modules are
    // present before applying termios/window settings, matching illumos libc's
    // compatibility implementation.
    let has_ldterm =
        unsafe { libc::ioctl(slave, libc::I_FIND, LDTERM.as_ptr().cast::<libc::c_char>()) };
    if has_ldterm < 0 {
        return unsafe { close_solaris_pty_pair(master, slave) };
    }
    if has_ldterm == 0
        && (unsafe { libc::ioctl(slave, libc::I_PUSH, PTEM.as_ptr().cast::<libc::c_char>()) } < 0
            || unsafe { libc::ioctl(slave, libc::I_PUSH, LDTERM.as_ptr().cast::<libc::c_char>()) }
                < 0)
    {
        return unsafe { close_solaris_pty_pair(master, slave) };
    }

    if !termp.is_null() && unsafe { libc::tcsetattr(slave, libc::TCSAFLUSH, termp) } != 0 {
        return unsafe { close_solaris_pty_pair(master, slave) };
    }
    if !winp.is_null() && unsafe { libc::ioctl(slave, libc::TIOCSWINSZ, winp) } < 0 {
        return unsafe { close_solaris_pty_pair(master, slave) };
    }

    if !name.is_null() {
        unsafe {
            libc::strcpy(name, slave_path);
        }
    }

    unsafe {
        *amaster = master;
        *aslave = slave;
    }
    0
}
