//! Download helpers for record files and archives.
//!
//! Use this module when you already know which published record or DOI you want
//! to consume and need either:
//!
//! - a streaming response via [`DownloadStream`]
//! - a resolved local download via [`ResolvedDownload`]
//! - high-level selector-based downloads via [`crate::records::ArtifactSelector`]
//!
//! For record lookup and DOI resolution before downloading, see
//! [`crate::records`].

use std::path::Path;
use std::pin::Pin;

use futures_core::Stream;
use futures_util::StreamExt;
#[cfg(feature = "checksums")]
use md5::{Digest, Md5};
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::client::ZenodoClient;
use crate::error::ZenodoError;
use crate::ids::{Doi, RecordId};
use crate::model::Record;
use crate::progress::TransferProgress;
use crate::records::{ArtifactSelector, RecordSelector};

/// Streaming download response metadata plus the response body stream.
pub struct DownloadStream {
    /// Parsed `Content-Type` header when the server provides one.
    pub content_type: Option<mime::Mime>,
    /// Parsed `Content-Length` header when the server provides one.
    pub content_length: Option<u64>,
    /// Raw `Content-Disposition` header when the server provides one.
    pub content_disposition: Option<String>,
    /// Byte stream for the response body.
    pub stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ZenodoError>> + Send>>,
}

/// Details about how an artifact download request was resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDownload {
    /// The selector originally requested by the caller.
    pub requested: ArtifactSelector,
    /// The record ID that ultimately provided the bytes.
    pub resolved_record: RecordId,
    /// The DOI on the resolved record, when present.
    pub resolved_doi: Option<Doi>,
    /// The resolved file key for file downloads.
    pub resolved_key: Option<String>,
    /// Number of bytes written to the destination path.
    pub bytes_written: u64,
    /// Checksum reported by Zenodo for the resolved file, when present.
    pub checksum: Option<String>,
}

#[derive(Clone, Debug)]
struct ResolvedArtifact {
    requested: ArtifactSelector,
    resolved_record: Record,
    resolved_key: Option<String>,
    checksum: Option<String>,
    url: Url,
}

impl ZenodoClient {
    /// Opens a download stream for a Zenodo artifact selector.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{ArtifactSelector, Auth, RecordId, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let stream = client
    ///         .open_artifact(&ArtifactSelector::latest_file(RecordId(123), "artifact.tar.gz"))
    ///         .await?;
    ///     let _ = stream.content_length;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if selector resolution fails or if Zenodo returns a
    /// non-success response for the resolved download.
    pub async fn open_artifact(
        &self,
        selector: &ArtifactSelector,
    ) -> Result<DownloadStream, ZenodoError> {
        let resolved = self.resolve_artifact(selector).await?;
        self.open_download_url(&resolved.url).await
    }

    async fn open_download_url(&self, file_url: &Url) -> Result<DownloadStream, ZenodoError> {
        let response = self
            .execute_response(self.download_request_url(reqwest::Method::GET, file_url.clone())?)
            .await?;

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let content_disposition = response
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let stream = response
            .bytes_stream()
            .map(|item| item.map_err(ZenodoError::Transport));

        Ok(DownloadStream {
            content_type,
            content_length,
            content_disposition,
            stream: Box::pin(stream),
        })
    }

    /// Downloads a named file from a specific record to a local path.
    ///
    /// Returns resolution metadata describing the record and file that
    /// ultimately produced the bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails, if the file is missing, or
    /// if writing the destination path fails.
    pub async fn download_record_file_by_key_to_path(
        &self,
        id: RecordId,
        key: &str,
        path: &Path,
    ) -> Result<ResolvedDownload, ZenodoError> {
        self.download_record_file_by_key_to_path_with_progress(id, key, path, ())
            .await
    }

    /// Downloads a named file from a specific record to a local path while reporting progress.
    ///
    /// Returns resolution metadata describing the record and file that
    /// ultimately produced the bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails, if the file is missing, or
    /// if writing the destination path fails.
    pub async fn download_record_file_by_key_to_path_with_progress<P>(
        &self,
        id: RecordId,
        key: &str,
        path: &Path,
        progress: P,
    ) -> Result<ResolvedDownload, ZenodoError>
    where
        P: TransferProgress,
    {
        self.download_artifact_with_progress(
            &ArtifactSelector::FileByKey {
                record: RecordSelector::RecordId(id),
                key: key.to_owned(),
                latest: false,
            },
            path,
            progress,
        )
        .await
    }

    /// Downloads a named file from the latest record version to a local path.
    ///
    /// # Errors
    ///
    /// Returns an error if latest-version resolution fails, if the file is
    /// missing, or if writing the destination path fails.
    pub async fn download_latest_record_file_by_key_to_path(
        &self,
        id: RecordId,
        key: &str,
        path: &Path,
    ) -> Result<ResolvedDownload, ZenodoError> {
        self.download_latest_record_file_by_key_to_path_with_progress(id, key, path, ())
            .await
    }

    /// Downloads a named file from the latest record version to a local path while reporting progress.
    ///
    /// # Errors
    ///
    /// Returns an error if latest-version resolution fails, if the file is
    /// missing, or if writing the destination path fails.
    pub async fn download_latest_record_file_by_key_to_path_with_progress<P>(
        &self,
        id: RecordId,
        key: &str,
        path: &Path,
        progress: P,
    ) -> Result<ResolvedDownload, ZenodoError>
    where
        P: TransferProgress,
    {
        self.download_artifact_with_progress(
            &ArtifactSelector::FileByKey {
                record: RecordSelector::RecordId(id),
                key: key.to_owned(),
                latest: true,
            },
            path,
            progress,
        )
        .await
    }

    /// Downloads the archive for a specific record to a local path.
    ///
    /// Returns resolution metadata describing the record that produced the
    /// archive bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails, if the archive link is
    /// missing, or if writing the destination path fails.
    pub async fn download_record_archive_to_path(
        &self,
        id: RecordId,
        path: &Path,
    ) -> Result<ResolvedDownload, ZenodoError> {
        self.download_record_archive_to_path_with_progress(id, path, ())
            .await
    }

    /// Downloads the archive for a specific record to a local path while reporting progress.
    ///
    /// Returns resolution metadata describing the record that produced the
    /// archive bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails, if the archive link is
    /// missing, or if writing the destination path fails.
    pub async fn download_record_archive_to_path_with_progress<P>(
        &self,
        id: RecordId,
        path: &Path,
        progress: P,
    ) -> Result<ResolvedDownload, ZenodoError>
    where
        P: TransferProgress,
    {
        self.download_artifact_with_progress(
            &ArtifactSelector::Archive {
                record: RecordSelector::RecordId(id),
                latest: false,
            },
            path,
            progress,
        )
        .await
    }

    /// Downloads a named file after resolving a DOI to a record.
    ///
    /// # Errors
    ///
    /// Returns an error if DOI resolution fails, if the file is missing, or if
    /// writing the destination path fails.
    pub async fn download_file_by_doi_to_path(
        &self,
        doi: &Doi,
        key: &str,
        latest: bool,
        path: &Path,
    ) -> Result<ResolvedDownload, ZenodoError> {
        self.download_file_by_doi_to_path_with_progress(doi, key, latest, path, ())
            .await
    }

    /// Downloads a named file after resolving a DOI to a record while reporting progress.
    ///
    /// # Errors
    ///
    /// Returns an error if DOI resolution fails, if the file is missing, or if
    /// writing the destination path fails.
    pub async fn download_file_by_doi_to_path_with_progress<P>(
        &self,
        doi: &Doi,
        key: &str,
        latest: bool,
        path: &Path,
        progress: P,
    ) -> Result<ResolvedDownload, ZenodoError>
    where
        P: TransferProgress,
    {
        self.download_artifact_with_progress(
            &ArtifactSelector::FileByKey {
                record: RecordSelector::Doi(doi.clone()),
                key: key.to_owned(),
                latest,
            },
            path,
            progress,
        )
        .await
    }

    /// Downloads an artifact selected by high-level record or DOI selectors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use zenodo_rs::{ArtifactSelector, Auth, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let resolved = client
    ///         .download_artifact(
    ///             &ArtifactSelector::latest_archive_by_doi("10.5281/zenodo.123")?,
    ///             Path::new("record.zip"),
    ///         )
    ///         .await?;
    ///     let _ = resolved.bytes_written;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if record resolution fails, if the requested artifact
    /// is unavailable, if checksum validation fails, or if writing the
    /// destination path fails.
    pub async fn download_artifact(
        &self,
        selector: &ArtifactSelector,
        destination: &Path,
    ) -> Result<ResolvedDownload, ZenodoError> {
        self.download_artifact_with_progress(selector, destination, ())
            .await
    }

    /// Downloads an artifact selected by high-level record or DOI selectors while reporting progress.
    ///
    /// The supplied progress sink receives the response `Content-Length` when
    /// Zenodo provides one and one `advance` event per chunk successfully
    /// written to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if record resolution fails, if the requested artifact
    /// is unavailable, if checksum validation fails, or if writing the
    /// destination path fails.
    pub async fn download_artifact_with_progress<P>(
        &self,
        selector: &ArtifactSelector,
        destination: &Path,
        progress: P,
    ) -> Result<ResolvedDownload, ZenodoError>
    where
        P: TransferProgress,
    {
        let resolved = self.resolve_artifact(selector).await?;
        let bytes_written = write_stream_to_path_with_progress(
            self.open_download_url(&resolved.url).await?,
            destination,
            resolved.checksum.as_deref(),
            progress,
        )
        .await?;

        Ok(ResolvedDownload {
            requested: resolved.requested,
            resolved_record: resolved.resolved_record.id,
            resolved_doi: resolved.resolved_record.doi,
            resolved_key: resolved.resolved_key,
            bytes_written,
            checksum: resolved.checksum,
        })
    }

    async fn resolve_record_for_download(
        &self,
        selector: &RecordSelector,
        latest: bool,
    ) -> Result<Record, ZenodoError> {
        let record = self.resolve_record_selector(selector).await?;
        if latest {
            self.resolve_latest_from_record(record).await
        } else {
            Ok(record)
        }
    }

    async fn resolve_artifact(
        &self,
        selector: &ArtifactSelector,
    ) -> Result<ResolvedArtifact, ZenodoError> {
        match selector {
            ArtifactSelector::FileByKey {
                record,
                key,
                latest,
            } => {
                let resolved_record = self.resolve_record_for_download(record, *latest).await?;
                let file = resolved_record.file_by_key(key).cloned().ok_or_else(|| {
                    ZenodoError::MissingFile {
                        key: key.to_owned(),
                    }
                })?;
                let url = file
                    .download_url()
                    .cloned()
                    .ok_or(ZenodoError::MissingLink("record_file.links.self"))?;

                Ok(ResolvedArtifact {
                    requested: selector.clone(),
                    resolved_record,
                    resolved_key: Some(file.key),
                    checksum: file.checksum,
                    url,
                })
            }
            ArtifactSelector::Archive { record, latest } => {
                let resolved_record = self.resolve_record_for_download(record, *latest).await?;
                let url = resolved_record
                    .archive_url()
                    .cloned()
                    .ok_or(ZenodoError::MissingLink("archive"))?;

                Ok(ResolvedArtifact {
                    requested: selector.clone(),
                    resolved_record,
                    resolved_key: None,
                    checksum: None,
                    url,
                })
            }
        }
    }
}

async fn write_stream_to_path_with_progress<P>(
    mut stream: DownloadStream,
    path: &Path,
    #[cfg(feature = "checksums")] expected_checksum: Option<&str>,
    #[cfg(not(feature = "checksums"))] _expected_checksum: Option<&str>,
    progress: P,
) -> Result<u64, ZenodoError>
where
    P: TransferProgress,
{
    progress.begin(stream.content_length);
    let temp = tempfile::Builder::new()
        .prefix(".zenodo-rs-download-")
        .tempfile_in(download_parent_directory(path))?;
    let temp_path = temp.path().to_path_buf();
    let mut file = tokio::fs::File::from_std(temp.reopen()?);
    let mut bytes_written = 0_u64;
    #[cfg(feature = "checksums")]
    let mut checksum_validator = checksum_validator(expected_checksum)?;

    while let Some(chunk) = stream.stream.next().await {
        let result = async {
            let chunk = chunk?;
            #[cfg(feature = "checksums")]
            if let Some(validator) = checksum_validator.as_mut() {
                validator.update(&chunk);
            }
            file.write_all(&chunk).await?;
            bytes_written += chunk.len() as u64;
            progress.advance(chunk.len() as u64);
            Ok::<(), ZenodoError>(())
        }
        .await;
        if let Err(error) = result {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(error);
        }
    }

    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    #[cfg(feature = "checksums")]
    if let Some(validator) = checksum_validator {
        if let Err(error) = validator.finish() {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(error);
        }
    }
    temp.persist(path)
        .map_err(|error| ZenodoError::Io(error.error))?;
    progress.finish();
    Ok(bytes_written)
}

fn download_parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(feature = "checksums")]
#[derive(Debug)]
struct ChecksumValidator {
    expected: String,
    hasher: Md5,
}

#[cfg(feature = "checksums")]
impl ChecksumValidator {
    fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    fn finish(self) -> Result<(), ZenodoError> {
        let actual = hex::encode(self.hasher.finalize());
        if actual == self.expected {
            Ok(())
        } else {
            Err(ZenodoError::ChecksumMismatch {
                expected: self.expected,
                actual,
            })
        }
    }
}

#[cfg(feature = "checksums")]
fn checksum_validator(
    expected_checksum: Option<&str>,
) -> Result<Option<ChecksumValidator>, ZenodoError> {
    let Some(expected_checksum) = expected_checksum else {
        return Ok(None);
    };

    let Some((algorithm, expected)) = expected_checksum.split_once(':') else {
        return Err(ZenodoError::InvalidState(format!(
            "unsupported checksum format: {expected_checksum}"
        )));
    };

    if !algorithm.eq_ignore_ascii_case("md5") {
        return Err(ZenodoError::InvalidState(format!(
            "unsupported checksum algorithm: {algorithm}"
        )));
    }

    Ok(Some(ChecksumValidator {
        expected: expected.trim().to_ascii_lowercase(),
        hasher: Md5::new(),
    }))
}

#[cfg(test)]
mod tests {
    use crate::model::Record;

    #[test]
    fn artifact_lookup_uses_file_key() {
        let record: Record = serde_json::from_value(serde_json::json!({
            "id": 42,
            "recid": 42,
            "metadata": { "title": "artifact" },
            "files": [
                {
                    "id": "abc",
                    "key": "bundle.tar.gz",
                    "size": 10,
                    "links": { "self": "https://zenodo.org/api/files/1" }
                }
            ],
            "links": {
                "archive": "https://zenodo.org/api/records/42/files-archive"
            }
        }))
        .unwrap();

        assert_eq!(record.file_by_key("bundle.tar.gz").unwrap().id, "abc");
        assert_eq!(
            record.archive_url().unwrap().as_str(),
            "https://zenodo.org/api/records/42/files-archive"
        );
    }

    #[cfg(feature = "checksums")]
    #[test]
    fn checksum_validator_accepts_md5_and_rejects_unsupported_formats() {
        let mut validator = super::checksum_validator(Some("md5:900150983cd24fb0d6963f7d28e17f72"))
            .unwrap()
            .unwrap();
        validator.update(b"abc");
        assert!(validator.finish().is_ok());

        let error = super::checksum_validator(Some("sha256:deadbeef")).unwrap_err();
        assert!(matches!(error, crate::ZenodoError::InvalidState(_)));

        let error = super::checksum_validator(Some("deadbeef")).unwrap_err();
        assert!(matches!(error, crate::ZenodoError::InvalidState(_)));
    }
}
