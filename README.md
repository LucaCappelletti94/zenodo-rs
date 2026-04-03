# zenodo-rs

[![CI](https://github.com/LucaCappelletti94/zenodo-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/zenodo-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/zenodo-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/zenodo-rs)
[![crates.io](https://img.shields.io/crates/v/zenodo-rs.svg)](https://crates.io/crates/zenodo-rs)
[![docs.rs](https://img.shields.io/docsrs/zenodo-rs)](https://docs.rs/zenodo-rs)
[![License](https://img.shields.io/crates/l/zenodo-rs.svg)](https://github.com/LucaCappelletti94/zenodo-rs/blob/main/LICENSE)

Async Rust client for core [Zenodo](https://zenodo.org/) workflows.

It covers deposition create/update/publish flows, safe draft reuse versus `newversion`, published-record lookup, latest-version resolution, and downloads behind a small typed API for automation and CI jobs built on top of the [Zenodo REST API](https://developers.zenodo.org/).

## Install

```toml
[dependencies]
zenodo-rs = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Add the optional `checksums` feature if you want download-to-path helpers to validate Zenodo `md5:` checksums.

## Example

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

## Notes

`ZENODO_TOKEN` is the standard env var for the production service at [zenodo.org](https://zenodo.org/), and `ZENODO_SANDBOX_TOKEN` is the sandbox equivalent for [sandbox.zenodo.org](https://sandbox.zenodo.org/). Write flows usually need `deposit:write` and `deposit:actions`; see the [Zenodo developer docs](https://developers.zenodo.org/). Public download APIs use Zenodo IDs and selectors rather than raw URLs, and uploads require a known content length.
