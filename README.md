# zenodo-rs

Async Rust client for core Zenodo workflows.

It covers:

- deposition create/fetch/update
- safe draft reuse vs `newversion`
- policy-aware file upload and replacement
- publish/edit/discard actions
- record lookup, latest-version resolution, and downloads

## Install

```toml
[dependencies]
zenodo-rs = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Enable checksum validation on download-to-path helpers with:

```toml
[dependencies]
zenodo-rs = { version = "0.1", features = ["checksums"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Example

```rust,no_run
use zenodo_rs::{
    AccessRight, Creator, DepositMetadataUpdate, UploadSpec, UploadType, ZenodoClient,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ZenodoClient::from_sandbox_env()?;

    let metadata = DepositMetadataUpdate::builder()
        .title("Example dataset")
        .upload_type(UploadType::Dataset)
        .description_html("<p>Example upload</p>")
        .creator(
            Creator::builder()
                .name("Doe, Jane")
                .affiliation("Zenodo")
                .build()?,
        )
        .access_right(AccessRight::Open)
        .build()?;

    let draft = client.create_deposition().await?;
    let _published = client
        .publish_dataset(
            draft.id,
            &metadata,
            vec![UploadSpec::from_path("artifact.tar.gz")?],
        )
        .await?;

    Ok(())
}
```

## Notes

- Standard token env vars:
  `ZENODO_TOKEN` for production and `ZENODO_SANDBOX_TOKEN` for sandbox.
- Use `Endpoint::Sandbox` for sandbox runs.
- Write operations usually need `deposit:write` and `deposit:actions`.
- Public download APIs use Zenodo IDs and selectors, not raw URLs.
- DOI selectors are validated and normalized internally, including `doi:` and `https://doi.org/...` forms.
- With the optional `checksums` feature, file downloads validate Zenodo `md5:` checksums before replacing the destination path.
- High-level workflows default to replace-all semantics, and also expose explicit file replacement policies. `KeepExistingAndAdd` rejects filename collisions instead of silently overwriting an existing draft file.
- `Endpoint::Custom` accepts either a deployment root or an API base and normalizes it to end in `/api/`.
- Uploads require a known content length.
