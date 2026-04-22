//! Progress reporting hooks for uploads and downloads.
//!
//! The core client stays generic over any progress sink that implements
//! [`TransferProgress`]. Under the optional `indicatif` feature,
//! [`indicatif::ProgressBar`] implements this trait directly.
//!
//! # Examples
//!
//! ```rust
//! #[cfg(feature = "indicatif")]
//! {
//!     use indicatif::ProgressBar;
//!     use zenodo_rs::TransferProgress;
//!
//!     let bar = ProgressBar::new(0);
//!     bar.begin(Some(5));
//!     bar.advance(2);
//!     assert_eq!(bar.length(), Some(5));
//!     assert_eq!(bar.position(), 2);
//!     bar.finish();
//! }
//! ```
//!
//! Pass `bar.clone()` into the progress-aware upload and download helpers when
//! you want a real terminal progress bar during transfers.

/// Progress sink for streaming uploads and downloads.
///
/// Implement this trait when you want upload and download helpers to report
/// byte-level transfer progress into your own logging, UI, or terminal
/// progress bar implementation.
pub trait TransferProgress: Send + Sync {
    /// Called once before the transfer starts.
    ///
    /// `total_bytes` is `Some(len)` when the total size is known up front and
    /// `None` when the transfer length is unknown.
    fn begin(&self, _total_bytes: Option<u64>) {}

    /// Called after each successfully transferred chunk.
    fn advance(&self, _delta: u64) {}

    /// Called once after a transfer completes successfully.
    fn finish(&self) {}
}

impl TransferProgress for () {}

impl<P> TransferProgress for std::sync::Arc<P>
where
    P: TransferProgress + ?Sized,
{
    fn begin(&self, total_bytes: Option<u64>) {
        self.as_ref().begin(total_bytes);
    }

    fn advance(&self, delta: u64) {
        self.as_ref().advance(delta);
    }

    fn finish(&self) {
        self.as_ref().finish();
    }
}

#[cfg(feature = "indicatif")]
impl TransferProgress for indicatif::ProgressBar {
    fn begin(&self, total_bytes: Option<u64>) {
        self.set_position(0);
        if let Some(total_bytes) = total_bytes {
            self.set_length(total_bytes);
        }
    }

    fn advance(&self, delta: u64) {
        self.inc(delta);
    }

    fn finish(&self) {
        indicatif::ProgressBar::finish(self);
    }
}

#[cfg(all(test, feature = "indicatif"))]
mod tests {
    use super::TransferProgress;

    #[test]
    fn indicatif_progress_bar_tracks_transfer_progress() {
        let bar = indicatif::ProgressBar::new(0);

        bar.begin(Some(5));
        bar.advance(2);

        assert_eq!(bar.length(), Some(5));
        assert_eq!(bar.position(), 2);

        bar.finish();
    }
}
