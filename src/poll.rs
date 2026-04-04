//! Polling configuration for asynchronous Zenodo state transitions.

use std::time::Duration;

/// Backoff settings used by workflow helpers while waiting for Zenodo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollOptions {
    /// Maximum total time to wait before failing.
    pub max_wait: Duration,
    /// Delay before the first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
}

impl Default for PollOptions {
    fn default() -> Self {
        Self {
            max_wait: Duration::from_secs(60),
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
        }
    }
}
