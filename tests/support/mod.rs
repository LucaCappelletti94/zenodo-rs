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
use std::time::Duration;

use tempfile::TempDir;
use zenodo_rs::{
    AccessRight, Auth, DepositMetadataUpdate, DepositionId, PollOptions, UploadSpec, UploadType,
    ZenodoClient,
};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub const DRAFT_ID_ENV_VAR: &str = "ZENODO_SANDBOX_DRAFT_DEPOSITION_ID";
pub const RECORD_DOI_ENV_VAR: &str = "ZENODO_SANDBOX_RECORD_DOI";

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

pub fn draft_deposition_id() -> DepositionId {
    DepositionId(
        required_env(DRAFT_ID_ENV_VAR)
            .parse()
            .expect("valid sandbox draft deposition id"),
    )
}

pub fn published_record_doi() -> String {
    required_env(RECORD_DOI_ENV_VAR)
}

pub fn unique_suffix(label: &str) -> String {
    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".to_owned());
    let run_attempt = std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| "0".to_owned());
    let sequence = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{label}-{run_id}-{run_attempt}-{sequence}")
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
