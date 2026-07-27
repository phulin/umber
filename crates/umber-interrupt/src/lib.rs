//! Safe, process-wide Ctrl-C dispatch over small platform FFI boundaries.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use self::unix as platform;
#[cfg(windows)]
use self::windows as platform;

#[cfg(not(any(unix, windows)))]
compile_error!("umber-interrupt supports only Unix and Windows targets");

const IDLE: u8 = 0;
const INSTALLING: u8 = 1;
const ACTIVE: u8 = 2;
const FAILED: u8 = 3;
const DISPATCH_INTERVAL: Duration = Duration::from_millis(10);

static INSTALL_STATE: AtomicU8 = AtomicU8::new(IDLE);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Installs the process-wide Ctrl-C handler.
///
/// The OS callback only publishes to a lock-free atomic. `handler` runs on a
/// dedicated Rust thread, so it may safely lock mutexes and run ordinary Rust
/// cancellation code. Installation is intentionally process-wide and may be
/// performed only once.
pub fn set_handler(handler: impl Fn() + Send + 'static) -> Result<(), InstallError> {
    INSTALL_STATE
        .compare_exchange(IDLE, INSTALLING, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| InstallError::AlreadyInstalled)?;

    let dispatcher = thread::Builder::new()
        .name("umber-ctrl-c".into())
        .spawn(move || dispatch(handler));
    if let Err(error) = dispatcher {
        INSTALL_STATE.store(FAILED, Ordering::Release);
        return Err(InstallError::Thread(error));
    }

    if let Err(error) = platform::install() {
        INSTALL_STATE.store(FAILED, Ordering::Release);
        return Err(InstallError::Os(error));
    }
    INSTALL_STATE.store(ACTIVE, Ordering::Release);
    Ok(())
}

fn dispatch(handler: impl Fn()) {
    while INSTALL_STATE.load(Ordering::Acquire) == INSTALLING {
        thread::yield_now();
    }
    while INSTALL_STATE.load(Ordering::Acquire) == ACTIVE {
        if INTERRUPTED.swap(false, Ordering::AcqRel) {
            handler();
        }
        thread::sleep(DISPATCH_INTERVAL);
    }
}

fn record_interrupt() {
    INTERRUPTED.store(true, Ordering::Release);
}

#[derive(Debug)]
pub enum InstallError {
    AlreadyInstalled,
    Thread(io::Error),
    Os(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInstalled => f.write_str("a Ctrl-C handler is already installed"),
            Self::Thread(error) => write!(f, "failed to start Ctrl-C dispatcher: {error}"),
            Self::Os(error) => write!(f, "failed to register Ctrl-C with the OS: {error}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AlreadyInstalled => None,
            Self::Thread(error) | Self::Os(error) => Some(error),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::mem::MaybeUninit;
    use std::ptr;
    use std::sync::mpsc;

    use super::*;

    /// Gives SIGINT the default, deliverable disposition `install` requires.
    ///
    /// A shell starting a job in the background sets SIGINT to `SIG_IGN` in the
    /// child, and every descendant inherits it. `install` deliberately refuses
    /// to displace a non-default disposition -- a process handed an ignored
    /// SIGINT is meant to keep ignoring it -- so this test fails outright when
    /// its runner was backgrounded, as `scripts/check-and-test.sh` backgrounds
    /// the suite. The precondition was always assumed and never established;
    /// the test only ever passed because nothing had run it from a background
    /// job (`umber2-johp.211`).
    fn establish_default_sigint() {
        let mut action = MaybeUninit::<libc::sigaction>::zeroed();
        let mut unblocked = MaybeUninit::<libc::sigset_t>::zeroed();

        // SAFETY: both values point to initialized storage of the exact types
        // libc requires, and both calls copy out of that storage rather than
        // retaining it.
        unsafe {
            let action = action.assume_init_mut();
            action.sa_sigaction = libc::SIG_DFL;
            assert_eq!(libc::sigemptyset(&mut action.sa_mask), 0);
            assert_eq!(libc::sigaction(libc::SIGINT, action, ptr::null_mut()), 0);

            let unblocked = unblocked.assume_init_mut();
            assert_eq!(libc::sigemptyset(unblocked), 0);
            assert_eq!(libc::sigaddset(unblocked, libc::SIGINT), 0);
            assert_eq!(
                libc::sigprocmask(libc::SIG_UNBLOCK, unblocked, ptr::null_mut()),
                0
            );
        }
    }

    #[test]
    fn dispatches_sigint_once_and_rejects_reinstallation() {
        establish_default_sigint();

        let (sent, received) = mpsc::channel();
        set_handler(move || sent.send(()).expect("test receiver remains alive"))
            .expect("install handler");

        // SAFETY: SIGINT is a valid signal number and this test installed its
        // process handler immediately above.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("Ctrl-C callback");

        assert!(matches!(
            set_handler(|| {}),
            Err(InstallError::AlreadyInstalled)
        ));
    }
}
