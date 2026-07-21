use std::io;

use windows_sys::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::core::BOOL;

use crate::record_interrupt;

unsafe extern "system" fn handle_console_event(event: u32) -> BOOL {
    if event == CTRL_C_EVENT {
        record_interrupt();
        1
    } else {
        0
    }
}

pub(super) fn install() -> io::Result<()> {
    // SAFETY: the callback uses the required ABI, remains valid for the
    // process lifetime, and only touches a lock-free atomic.
    if unsafe { SetConsoleCtrlHandler(Some(handle_console_event), 1) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
