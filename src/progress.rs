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
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     use axum::{
//!         body::Body,
//!         extract::State,
//!         http::{header, HeaderValue},
//!         routing::get,
//!         Json, Router,
//!     };
//!     use indicatif::{ProgressBar, ProgressStyle};
//!     use serde_json::json;
//!     use std::sync::Arc;
//!     use zenodo_rs::{ArtifactSelector, Auth, Endpoint, RecordId, ZenodoClient};
//!
//!     #[derive(Clone)]
//!     struct AppState {
//!         base: Arc<String>,
//!     }
//!
//!     async fn record(State(state): State<AppState>) -> Json<serde_json::Value> {
//!         Json(json!({
//!             "id": 123,
//!             "recid": 123,
//!             "metadata": { "title": "Example" },
//!             "files": [{
//!                 "id": "f-123",
//!                 "key": "artifact.bin",
//!                 "size": 5,
//!                 "links": {
//!                     "self": format!("{}download/123/artifact.bin", state.base),
//!                 }
//!             }],
//!             "links": {}
//!         }))
//!     }
//!
//!     async fn artifact() -> axum::response::Response {
//!         let mut response = axum::response::Response::new(Body::from("hello"));
//!         response.headers_mut().insert(
//!             header::CONTENT_TYPE,
//!             HeaderValue::from_static("application/octet-stream"),
//!         );
//!         response
//!     }
//!
//!     let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
//!     let address = listener.local_addr()?;
//!     let base = Arc::new(format!("http://{address}/api/"));
//!     let app = Router::new()
//!         .route("/api/records/123", get(record))
//!         .route("/api/download/123/artifact.bin", get(artifact))
//!         .with_state(AppState { base: Arc::clone(&base) });
//!     let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
//!     let server = tokio::spawn(async move {
//!         axum::serve(listener, app)
//!             .with_graceful_shutdown(async {
//!                 let _ = shutdown_rx.await;
//!             })
//!             .await
//!     });
//!
//!     let client = ZenodoClient::builder(Auth::new("token"))
//!         .endpoint(Endpoint::Custom(base.parse()?))
//!         .build()?;
//!     let bar = ProgressBar::new(0);
//!     bar.set_style(ProgressStyle::with_template(
//!         "{bar:20.cyan/blue} {bytes}/{total_bytes}",
//!     )?);
//!     let temp_dir = tempfile::tempdir()?;
//!     let path = temp_dir.path().join("artifact.bin");
//!
//!     let resolved = client
//!         .download_artifact_with_progress(
//!             &ArtifactSelector::latest_file(RecordId(123), "artifact.bin"),
//!             &path,
//!             bar.clone(),
//!         )
//!         .await?;
//!
//!     assert_eq!(resolved.bytes_written, 5);
//!     assert_eq!(std::fs::read(&path)?, b"hello");
//!     assert_eq!(bar.position(), 5);
//!     let _ = shutdown_tx.send(());
//!     server.await??;
//!     Ok(())
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
