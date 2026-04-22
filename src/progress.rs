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
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     use indicatif::{ProgressBar, ProgressStyle};
//!     use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!     use tokio::net::TcpListener;
//!     use zenodo_rs::{ArtifactSelector, Auth, Endpoint, RecordId, RecordSelector, ZenodoClient};
//!
//!     tokio::runtime::Runtime::new()?.block_on(async {
//!         let listener = TcpListener::bind("127.0.0.1:0").await?;
//!         let address = listener.local_addr()?;
//!         let base = format!("http://{address}/api/");
//!         let record_body = format!(
//!             concat!(
//!                 r#"{{"id":123,"recid":123,"metadata":{{"title":"Example"}},"files":["#,
//!                 r#"{{"id":"f-123","key":"artifact.bin","size":5,"links":{{"self":"{}download/123/artifact.bin"}}}}"#,
//!                 r#"],"links":{{}}}}"#
//!             ),
//!             base,
//!         );
//!
//!         let server = tokio::spawn(async move {
//!             let responses = [
//!                 format!(
//!                     "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
//!                     record_body.len(),
//!                     record_body,
//!                 )
//!                 .into_bytes(),
//!                 b"HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: 5\r\n\r\nhello".to_vec(),
//!             ];
//!
//!             for response in responses {
//!                 let (mut stream, _) = listener.accept().await?;
//!                 let mut buffer = [0_u8; 2048];
//!                 let _ = stream.read(&mut buffer).await;
//!                 stream.write_all(&response).await?;
//!                 stream.shutdown().await?;
//!             }
//!
//!             Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//!         });
//!
//!         let client = ZenodoClient::builder(Auth::new("token"))
//!             .endpoint(Endpoint::Custom(base.parse()?))
//!             .build()?;
//!         let bar = ProgressBar::new(0);
//!         bar.set_style(ProgressStyle::with_template(
//!             "{bar:20.cyan/blue} {bytes}/{total_bytes}",
//!         )?);
//!         let suffix = std::time::SystemTime::now()
//!             .duration_since(std::time::UNIX_EPOCH)?
//!             .as_nanos();
//!         let path = std::env::temp_dir().join(format!(
//!             "zenodo-rs-progress-{}-{suffix}.bin",
//!             std::process::id(),
//!         ));
//!
//!         let download = client
//!             .download_artifact_with_progress(
//!                 &ArtifactSelector::FileByKey {
//!                     record: RecordSelector::RecordId(RecordId(123)),
//!                     key: "artifact.bin".into(),
//!                     latest: false,
//!                 },
//!                 &path,
//!                 bar.clone(),
//!             )
//!             .await?;
//!
//!         assert_eq!(download.bytes_written, 5);
//!         assert_eq!(std::fs::read(&path)?, b"hello");
//!         assert_eq!(bar.position(), 5);
//!
//!         let _ = std::fs::remove_file(&path);
//!         server.await??;
//!         Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//!     })
//! }
//!
//! #[cfg(not(feature = "indicatif"))]
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     Ok(())
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
