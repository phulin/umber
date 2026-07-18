use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Process-local telemetry timer that is inert on hosts without `Instant`.
pub(crate) struct TelemetryTimer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl TelemetryTimer {
    #[allow(clippy::disallowed_methods)] // Telemetry only; TeX state never observes it.
    pub(crate) fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        }
    }

    pub(crate) fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
    }
}
