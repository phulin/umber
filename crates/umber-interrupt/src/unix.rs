use std::io;
use std::mem::MaybeUninit;
use std::ptr;

use crate::record_interrupt;

extern "C" fn handle_sigint(_signal: libc::c_int) {
    record_interrupt();
}

pub(super) fn install() -> io::Result<()> {
    let mut action = MaybeUninit::<libc::sigaction>::zeroed();
    let mut previous = MaybeUninit::<libc::sigaction>::zeroed();

    // SAFETY: both values point to initialized storage of the exact types
    // required by libc. The callback has the required C ABI and remains valid
    // for the process lifetime. sigaction copies both structures.
    unsafe {
        let action = action.assume_init_mut();
        action.sa_sigaction = handle_sigint as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        if libc::sigemptyset(&mut action.sa_mask) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::sigaction(libc::SIGINT, action, previous.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }

        let previous = previous.assume_init();
        if previous.sa_sigaction != libc::SIG_DFL {
            let restore_result = libc::sigaction(libc::SIGINT, &previous, ptr::null_mut());
            if restore_result != 0 {
                return Err(io::Error::last_os_error());
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "SIGINT already has a non-default handler",
            ));
        }
    }
    Ok(())
}
