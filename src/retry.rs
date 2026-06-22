//! Retry policy for transient transport and server failures.
//!
//! Idempotent operations (record reads, downloads, and path-based uploads)
//! retry automatically on transient failures classified by
//! [`ZenodoError::is_retryable`], using the backoff configured in
//! [`RetryOptions`]. Non-idempotent actions (deposition creation, publishing,
//! and reader-based uploads) are never retried.

use std::time::Duration;

use tokio::time::sleep;

use crate::client::ZenodoClient;
use crate::error::ZenodoError;

/// Backoff settings for retrying transient failures on idempotent operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryOptions {
    /// Number of retries attempted after the first try. Total attempts are
    /// `max_retries + 1`.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries, capping the exponential backoff.
    pub max_delay: Duration,
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RetryOptions {
    /// Returns options that disable retries, so operations attempt exactly once.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::RetryOptions;
    ///
    /// assert_eq!(RetryOptions::disabled().max_retries, 0);
    /// ```
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }
}

impl ZenodoClient {
    /// Runs `operation`, retrying transient failures per the configured
    /// [`RetryOptions`] with exponential backoff.
    ///
    /// The operation is rebuilt and re-run on each attempt, so callers must
    /// reconstruct any single-use request body inside `operation`. Retries stop
    /// at the first non-retryable error or once the attempt budget is spent.
    pub(crate) async fn retry<T, F, Fut>(&self, mut operation: F) -> Result<T, ZenodoError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, ZenodoError>>,
    {
        let options = self.retry_options();
        let mut attempts_left = options.max_retries;
        let mut delay = options.initial_delay;

        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if attempts_left == 0 || !error.is_retryable() {
                        return Err(error);
                    }
                    attempts_left -= 1;
                    sleep(delay).await;
                    delay = std::cmp::min(delay.saturating_mul(2), options.max_delay);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use reqwest::StatusCode;

    use super::RetryOptions;
    use crate::client::{Auth, ZenodoClient};
    use crate::error::ZenodoError;

    fn retryable() -> ZenodoError {
        ZenodoError::Http {
            status: StatusCode::BAD_GATEWAY,
            message: None,
            field_errors: Vec::new(),
            raw_body: None,
        }
    }

    fn client_with(max_retries: u32) -> ZenodoClient {
        ZenodoClient::builder(Auth::new("token"))
            .retry_options(RetryOptions {
                max_retries,
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(1),
            })
            .build()
            .unwrap()
    }

    #[test]
    fn defaults_enable_retries_and_disabled_turns_them_off() {
        assert_eq!(RetryOptions::default().max_retries, 3);
        assert_eq!(RetryOptions::disabled().max_retries, 0);
    }

    #[tokio::test]
    async fn retries_exhaust_the_attempt_budget_for_retryable_errors() {
        let client = client_with(2);
        let calls = AtomicU32::new(0);
        let result: Result<(), ZenodoError> = client
            .retry(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(retryable()) }
            })
            .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_stop_immediately_on_non_retryable_errors() {
        let client = client_with(5);
        let calls = AtomicU32::new(0);
        let result: Result<(), ZenodoError> = client
            .retry(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ZenodoError::InvalidState("terminal".to_owned())) }
            })
            .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_return_the_first_success() {
        let client = client_with(5);
        let calls = AtomicU32::new(0);
        let result: Result<u8, ZenodoError> = client
            .retry(|| {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt < 2 {
                        Err(retryable())
                    } else {
                        Ok(7)
                    }
                }
            })
            .await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
