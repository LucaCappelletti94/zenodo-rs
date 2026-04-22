# zenodo-rs

[![CI](https://github.com/LucaCappelletti94/zenodo-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/zenodo-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/zenodo-rs/graph/badge.svg)](https://app.codecov.io/gh/LucaCappelletti94/zenodo-rs)
[![crates.io](https://img.shields.io/crates/v/zenodo-rs.svg)](https://crates.io/crates/zenodo-rs)
[![docs.rs](https://img.shields.io/docsrs/zenodo-rs)](https://docs.rs/zenodo-rs/latest/zenodo_rs/)
[![License](https://img.shields.io/crates/l/zenodo-rs.svg)](https://github.com/LucaCappelletti94/zenodo-rs/blob/main/LICENSE)

Async Rust client for core [Zenodo](https://zenodo.org/) workflows.

It covers deposition create/update/publish flows, safe draft reuse versus `newversion`, published-record lookup, latest-version resolution, and downloads behind a small typed API for automation and CI jobs built on top of the [Zenodo REST API](https://developers.zenodo.org/).

## Start Here

- Use [`ZenodoClient`](https://docs.rs/zenodo-rs/latest/zenodo_rs/client/struct.ZenodoClient.html) for the main entrypoint.
- Use [`workflow`](https://docs.rs/zenodo-rs/latest/zenodo_rs/workflow/index.html) for safe draft, publish, edit, and version helpers.
- Use [`records`](https://docs.rs/zenodo-rs/latest/zenodo_rs/records/index.html) for published-record lookup, DOI resolution, and search.
- Use [`downloads`](https://docs.rs/zenodo-rs/latest/zenodo_rs/downloads/index.html) for file and archive downloads.
- Use [`DepositMetadataUpdate`](https://docs.rs/zenodo-rs/latest/zenodo_rs/metadata/struct.DepositMetadataUpdate.html) and [`UploadSpec`](https://docs.rs/zenodo-rs/latest/zenodo_rs/upload/struct.UploadSpec.html) to describe what you want to publish.

## Install

```toml
[dependencies]
zenodo-rs = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Optional features:

- `checksums`: validate Zenodo `md5:` checksums when downloading to a path
- `indicatif`: implement `TransferProgress` for `indicatif::ProgressBar`
- `native-tls`: use `reqwest` with `native-tls` instead of the default `rustls-tls`

## Read Example

```rust,no_run
use zenodo_rs::ZenodoClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ZenodoClient::from_sandbox_env()?;
    let record = client.get_record_by_doi_str("10.5281/zenodo.123").await?;
    let _ = record.id;

    Ok(())
}
```

## Publish Example

```rust,no_run
use zenodo_rs::{
    AccessRight, Auth, DepositMetadataUpdate, UploadSpec, UploadType, ZenodoClient,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ZenodoClient::new(Auth::new("token"))?;
    let metadata = DepositMetadataUpdate::builder()
        .title("Example dataset")
        .upload_type(UploadType::Dataset)
        .description_html("<p>Example upload</p>")
        .creator_named("Doe, Jane")
        .access_right(AccessRight::Open)
        .build()?;
    let files = UploadSpec::from_named_paths([("artifact.tar.gz", "target/release.tar.gz")])?;

    let published = client
        .create_and_publish_dataset(&metadata, files)
        .await?;
    let _ = published.record.id;

    Ok(())
}
```

## Authentication

- `ZENODO_TOKEN` is the standard env var for the production service at [zenodo.org](https://zenodo.org/).
- `ZENODO_SANDBOX_TOKEN` is the sandbox equivalent for [sandbox.zenodo.org](https://sandbox.zenodo.org/).
- Write flows usually need `deposit:write` and `deposit:actions`.

## Progress Bars

Enable the `indicatif` feature if you want `indicatif::ProgressBar` to work
directly with `upload_path_with_progress`, `upload_reader_with_progress`,
`reconcile_files_with_progress`, and `download_artifact_with_progress`.
Pass `bar.clone()` into the progress-aware upload and download helpers when you
want a real terminal progress bar during transfers.

The full runnable example lives in the [`progress`](https://docs.rs/zenodo-rs/latest/zenodo_rs/progress/index.html)
module docs.

## Notes

Public download APIs use Zenodo IDs and selectors rather than raw URLs, and uploads require a known content length. For Zenodo-side behavior and token scopes, see the [Zenodo developer docs](https://developers.zenodo.org/).

## Zenodo Limits and Retention

Zenodo's current upload docs say a record can contain up to 100 files and a total of 50 GB (`50,000,000,000` bytes), with uploads up to 50 GB by default and additional quota that can bring a record up to 200 GB. If you need larger payloads than Zenodo is a good fit for, consider [`internetarchive-rs`](https://github.com/LucaCappelletti94/internetarchive-rs). Zenodo does not publish an SLA; instead, its principles page states a continuous-availability target of at least 99.7%, cites historical uptime of 99.98% (October 2019), and points to a public status page for live figures. Its preservation policy says items are retained for the lifetime of the repository, currently tied to CERN and described as at least the next 20 years; withdrawn records keep a tombstone page, DOI, and original URL. Zenodo only promises bit-level preservation, not future usability or understandability of deposited objects. `zenodo-rs` does not currently enforce the 100-file or 50 GB client-side limits: it validates duplicate/conflicting filenames and always sends an explicit `Content-Length`, but limit overflows are currently left for Zenodo to reject at request time. Sources: [Manage files](https://help.zenodo.org/docs/deposit/manage-files/), [Manage quota](https://help.zenodo.org/docs/deposit/manage-quota/), [General policies](https://about.zenodo.org/policies/), [Principles](https://about.zenodo.org/principles/), [Status page](https://stats.uptimerobot.com/vlYOVuWgM).
