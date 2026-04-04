#![allow(
    dead_code,
    missing_docs,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::unwrap_used
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use url::Url;
use zenodo_rs::{
    AccessRight, Auth, DepositMetadataUpdate, Deposition, DepositionId, Doi, PollOptions, Record,
    RecordId, UploadSpec, UploadType, ZenodoClient, ZenodoError,
};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct StepLog;

impl Drop for StepLog {
    fn drop(&mut self) {
        eprintln!("::endgroup::");
    }
}

pub fn live_client() -> ZenodoClient {
    ZenodoClient::builder(Auth::from_sandbox_env().expect("sandbox token"))
        .sandbox()
        .user_agent("zenodo-rs-live-ci/0.1")
        .request_timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(20))
        .poll_options(PollOptions {
            max_wait: Duration::from_secs(300),
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(15),
        })
        .build()
        .expect("build live sandbox client")
}

pub fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required environment variable {name}"))
}

pub fn unique_suffix(label: &str) -> String {
    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".to_owned());
    let run_attempt = std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| "0".to_owned());
    let sequence = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{label}-{run_id}-{run_attempt}-{sequence}")
}

pub fn step(name: impl AsRef<str>) -> StepLog {
    eprintln!("::group::{}", name.as_ref());
    StepLog
}

pub fn metadata(title_prefix: &str, version: Option<&str>) -> DepositMetadataUpdate {
    let mut builder = DepositMetadataUpdate::builder()
        .title(format!("{title_prefix} {}", unique_suffix("artifact")))
        .upload_type(UploadType::Dataset)
        .description_html("<p>zenodo-rs live CI smoke test artifact.</p>")
        .creator_named("zenodo-rs CI")
        .access_right(AccessRight::Open)
        .keyword("zenodo-rs")
        .keyword("live-ci");

    if let Some(version) = version {
        builder = builder.version(version);
    }

    builder.build().expect("valid live metadata")
}

pub fn path_upload(filename: &str, bytes: &[u8]) -> (TempDir, UploadSpec) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(filename);
    std::fs::write(&path, bytes).expect("write upload fixture");
    let spec = UploadSpec::from_path(&path).expect("path upload spec");
    (dir, spec)
}

pub fn reader_upload(filename: &str, bytes: &[u8]) -> UploadSpec {
    UploadSpec::from_reader(
        filename,
        std::io::Cursor::new(bytes.to_vec()),
        bytes.len() as u64,
        mime::APPLICATION_OCTET_STREAM,
    )
}

pub fn download_path(name: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(name);
    (dir, path)
}

pub async fn wait_for_latest_by_doi(
    client: &ZenodoClient,
    doi: &Doi,
    expected: RecordId,
) -> Record {
    let started = Instant::now();
    let timeout = Duration::from_secs(120);
    let delay = Duration::from_secs(2);

    loop {
        let last_error = match client.resolve_latest_by_doi(doi).await {
            Ok(record) if record.id == expected => return record,
            Ok(record) => format!(
                "resolved latest record {} instead of expected {}",
                record.id, expected
            ),
            Err(error) => error.to_string(),
        };

        assert!(
            started.elapsed() < timeout,
            "timed out waiting for DOI {doi} to resolve latest record {expected}: {}",
            last_error
        );
        tokio::time::sleep(delay).await;
    }
}

pub async fn wait_for_published_deposition(client: &ZenodoClient, id: DepositionId) -> Deposition {
    let started = Instant::now();
    let timeout = Duration::from_secs(180);
    let delay = Duration::from_secs(2);

    loop {
        let last_error = match client.get_deposition(id).await {
            Ok(deposition) if deposition.is_published() && deposition.record_id.is_some() => {
                return deposition;
            }
            Ok(deposition) => format!(
                "published={} state={:?} record_id={:?}",
                deposition.is_published(),
                deposition.status.state,
                deposition.record_id
            ),
            Err(error) => error.to_string(),
        };

        assert!(
            started.elapsed() < timeout,
            "timed out waiting for deposition {id} to publish: {}",
            last_error
        );
        tokio::time::sleep(delay).await;
    }
}

pub async fn wait_for_draft_deposition(client: &ZenodoClient, id: DepositionId) -> Deposition {
    let started = Instant::now();
    let timeout = Duration::from_secs(120);
    let delay = Duration::from_secs(2);

    loop {
        let last_error = match client.get_deposition(id).await {
            Ok(deposition) if !deposition.is_published() && deposition.allows_metadata_edits() => {
                return deposition;
            }
            Ok(deposition) => format!(
                "published={} state={:?}",
                deposition.is_published(),
                deposition.status.state
            ),
            Err(error) => match error {
                ZenodoError::Http { status, .. }
                    if status == reqwest::StatusCode::CONFLICT
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
                {
                    error.to_string()
                }
                _ => error.to_string(),
            },
        };

        assert!(
            started.elapsed() < timeout,
            "timed out waiting for draft deposition {id}: {}",
            last_error
        );
        tokio::time::sleep(delay).await;
    }
}

pub fn deposition_id_from_url(url: &Url) -> DepositionId {
    let id = url
        .path_segments()
        .and_then(Iterator::last)
        .unwrap_or_else(|| panic!("missing deposition id segment in {url}"))
        .parse::<u64>()
        .unwrap_or_else(|error| panic!("invalid deposition id in {url}: {error}"));
    DepositionId(id)
}
