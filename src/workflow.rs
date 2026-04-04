//! Higher-level workflow helpers that encode Zenodo deposition lifecycles.
//!
//! Use this module when you want the crate to encode Zenodo's safe-path state
//! transitions for you.
//!
//! The main helpers are:
//!
//! - [`ZenodoClient::ensure_editable_draft`] to reuse a draft or create a new version
//! - [`ZenodoClient::enter_edit_mode`] to reopen the current published deposition
//! - [`ZenodoClient::reconcile_files`] to apply a [`FileReplacePolicy`]
//! - [`ZenodoClient::publish_dataset_with_policy`] for end-to-end publish flows
//!
//! If you want to call raw endpoints one by one, use [`crate::client`] instead.

use std::collections::BTreeSet;
use std::time::Instant;

use tokio::time::sleep;
use url::Url;

use crate::client::ZenodoClient;
use crate::error::ZenodoError;
use crate::ids::DepositionId;
use crate::metadata::DepositMetadataUpdate;
use crate::model::{Deposition, PublishedRecord};
use crate::upload::{FileReplacePolicy, UploadSource, UploadSpec};

/// Action needed to obtain an editable draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditableDraftAction {
    /// The current deposition is already editable.
    ReuseExisting,
    /// A new version draft must be created first.
    CreateNewVersion,
}

/// Determines whether a deposition can be reused directly or needs `newversion`.
///
/// # Examples
///
/// ```
/// use zenodo_rs::workflow::{editable_draft_action, EditableDraftAction};
/// use zenodo_rs::Deposition;
///
/// let deposition: Deposition = serde_json::from_value(serde_json::json!({
///     "id": 1,
///     "submitted": true,
///     "state": "done",
///     "metadata": {},
///     "files": [],
///     "links": {}
/// }))?;
///
/// assert_eq!(
///     editable_draft_action(&deposition),
///     EditableDraftAction::CreateNewVersion
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn editable_draft_action(deposition: &Deposition) -> EditableDraftAction {
    if deposition.is_published() {
        EditableDraftAction::CreateNewVersion
    } else {
        EditableDraftAction::ReuseExisting
    }
}

pub(crate) fn file_ids_to_delete(
    policy: FileReplacePolicy,
    deposition: &Deposition,
    uploaded_filenames: &BTreeSet<String>,
) -> Vec<crate::ids::DepositionFileId> {
    match policy {
        FileReplacePolicy::ReplaceAll => deposition
            .files
            .iter()
            .map(|file| file.id.clone())
            .collect(),
        FileReplacePolicy::UpsertByFilename => deposition
            .files
            .iter()
            .filter(|file| uploaded_filenames.contains(&file.filename))
            .map(|file| file.id.clone())
            .collect(),
        FileReplacePolicy::KeepExistingAndAdd => Vec::new(),
    }
}

fn collect_uploaded_filenames(files: &[UploadSpec]) -> Result<BTreeSet<String>, ZenodoError> {
    let mut uploaded_filenames = BTreeSet::new();

    for spec in files {
        if spec.filename.is_empty() {
            return Err(ZenodoError::InvalidState(
                "upload filename cannot be empty".to_owned(),
            ));
        }
        if !uploaded_filenames.insert(spec.filename.clone()) {
            return Err(ZenodoError::DuplicateUploadFilename {
                filename: spec.filename.clone(),
            });
        }
    }

    Ok(uploaded_filenames)
}

fn validate_reconcile_inputs(
    policy: FileReplacePolicy,
    deposition: &Deposition,
    uploaded_filenames: &BTreeSet<String>,
) -> Result<(), ZenodoError> {
    if policy != FileReplacePolicy::KeepExistingAndAdd {
        return Ok(());
    }

    if let Some(filename) = deposition
        .files
        .iter()
        .map(|file| &file.filename)
        .find(|filename| uploaded_filenames.contains(*filename))
    {
        return Err(ZenodoError::ConflictingDraftFile {
            filename: filename.clone(),
        });
    }

    Ok(())
}

impl ZenodoClient {
    /// Enters edit mode for the current published version without versioning.
    ///
    /// Unpublished depositions are reused directly. Published depositions
    /// trigger `edit` and then wait until the current deposition becomes
    /// editable again.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, DepositionId, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let draft = client.enter_edit_mode(DepositionId(42)).await?;
    ///     let _ = draft.id;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the deposition lookup fails, if Zenodo rejects
    /// edit mode, or if the draft never becomes editable.
    pub async fn enter_edit_mode(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        let deposition = self.get_deposition(id).await?;
        if !deposition.is_published() {
            return Ok(deposition);
        }

        let edited = self.edit(id).await?;
        if !edited.is_published() {
            return Ok(edited);
        }

        self.poll_until("edit mode", || async move {
            let deposition = self.get_deposition(id).await?;
            if deposition.is_published() {
                Ok(None)
            } else {
                Ok(Some(deposition))
            }
        })
        .await
    }

    /// Returns an editable draft for the given deposition ID.
    ///
    /// Unpublished depositions are reused directly. Published depositions are
    /// first resolved to the latest published version and then trigger
    /// `newversion`, after which the helper follows `latest_draft`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, DepositionId, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let draft = client.ensure_editable_draft(DepositionId(42)).await?;
    ///     let _ = draft.id;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the deposition lookup fails, if Zenodo rejects
    /// version creation, or if the resulting draft never becomes available.
    pub async fn ensure_editable_draft(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        let deposition = self.get_deposition(id).await?;
        match editable_draft_action(&deposition) {
            EditableDraftAction::ReuseExisting => Ok(deposition),
            EditableDraftAction::CreateNewVersion => {
                let latest_published = self
                    .latest_published_deposition_for_new_version(deposition)
                    .await?;
                let latest = self.new_version(latest_published.id).await?;
                let latest_draft = latest
                    .latest_draft_url()
                    .cloned()
                    .ok_or(ZenodoError::MissingLink("latest_draft"))?;
                self.wait_for_deposition_url(&latest_draft, "latest draft")
                    .await
            }
        }
    }

    /// Replaces all currently visible draft files with the provided uploads.
    ///
    /// # Errors
    ///
    /// Returns an error if the draft cannot be refreshed, if the bucket link is
    /// missing, if file deletion fails, or if any upload fails.
    pub async fn replace_all_files<I>(
        &self,
        draft: &Deposition,
        files: I,
    ) -> Result<Vec<crate::model::BucketObject>, ZenodoError>
    where
        I: IntoIterator<Item = UploadSpec>,
    {
        self.reconcile_files(draft, FileReplacePolicy::ReplaceAll, files)
            .await
    }

    /// Reconciles draft files using the requested replacement policy.
    ///
    /// `ReplaceAll` deletes all currently visible draft files before upload.
    /// `UpsertByFilename` deletes only draft files whose filename matches one
    /// of the new uploads. `KeepExistingAndAdd` leaves all existing files in
    /// place and uploads additional files alongside them.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, DepositionId, FileReplacePolicy, UploadSpec, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let draft = client.ensure_editable_draft(DepositionId(42)).await?;
    ///     client
    ///         .reconcile_files(
    ///             &draft,
    ///             FileReplacePolicy::UpsertByFilename,
    ///             vec![UploadSpec::from_path("artifact.tar.gz")?],
    ///         )
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the draft cannot be refreshed, if the bucket link is
    /// missing, if duplicate upload filenames are provided, if a keep-existing
    /// upload would overwrite an existing draft filename, if file deletion
    /// fails, or if any upload fails.
    pub async fn reconcile_files<I>(
        &self,
        draft: &Deposition,
        policy: FileReplacePolicy,
        files: I,
    ) -> Result<Vec<crate::model::BucketObject>, ZenodoError>
    where
        I: IntoIterator<Item = UploadSpec>,
    {
        let files: Vec<_> = files.into_iter().collect();
        let refreshed = self.get_deposition(draft.id).await?;
        let bucket = refreshed
            .bucket_url()
            .cloned()
            .ok_or(ZenodoError::MissingLink("bucket"))?;
        let uploaded_filenames = collect_uploaded_filenames(&files)?;
        validate_reconcile_inputs(policy, &refreshed, &uploaded_filenames)?;

        for file_id in file_ids_to_delete(policy, &refreshed, &uploaded_filenames) {
            self.delete_file(refreshed.id, file_id).await?;
        }

        let mut uploaded = Vec::new();
        for spec in files {
            uploaded.push(self.upload_spec(&bucket, spec).await?);
        }

        Ok(uploaded)
    }

    /// Runs the full publish workflow for a deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if any draft lookup, metadata update, file upload,
    /// publish step, or final record lookup fails.
    pub async fn publish_dataset(
        &self,
        root: DepositionId,
        metadata: &DepositMetadataUpdate,
        files: impl IntoIterator<Item = UploadSpec>,
    ) -> Result<PublishedRecord, ZenodoError> {
        self.publish_dataset_with_policy(root, metadata, FileReplacePolicy::ReplaceAll, files)
            .await
    }

    /// Runs the full publish workflow for a deposition using a file policy.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{
    ///     AccessRight, Auth, Creator, DepositMetadataUpdate, DepositionId, FileReplacePolicy,
    ///     UploadSpec, UploadType, ZenodoClient,
    /// };
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let metadata = DepositMetadataUpdate::builder()
    ///         .title("Example dataset")
    ///         .upload_type(UploadType::Dataset)
    ///         .description_html("<p>Example upload</p>")
    ///         .creator(Creator::builder().name("Doe, Jane").build()?)
    ///         .access_right(AccessRight::Open)
    ///         .build()?;
    ///
    ///     let published = client
    ///         .publish_dataset_with_policy(
    ///             DepositionId(42),
    ///             &metadata,
    ///             FileReplacePolicy::KeepExistingAndAdd,
    ///             vec![UploadSpec::from_path("artifact.tar.gz")?],
    ///         )
    ///         .await?;
    ///     let _ = published.record.id;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any draft lookup, metadata update, file upload,
    /// publish step, duplicate/conflicting filename validation, or final
    /// record lookup fails.
    pub async fn publish_dataset_with_policy(
        &self,
        root: DepositionId,
        metadata: &DepositMetadataUpdate,
        policy: FileReplacePolicy,
        files: impl IntoIterator<Item = UploadSpec>,
    ) -> Result<PublishedRecord, ZenodoError> {
        let draft = self.ensure_editable_draft(root).await?;
        let draft = self.update_metadata(draft.id, metadata).await?;
        self.reconcile_files(&draft, policy, files).await?;
        let published = self.publish(draft.id).await?;
        let published = self.wait_for_published_deposition(published.id).await?;
        let record_id = published.record_id.ok_or_else(|| {
            ZenodoError::InvalidState("published deposition is missing record_id".into())
        })?;
        let record = self.get_record(record_id).await?;

        Ok(PublishedRecord {
            deposition: published,
            record,
        })
    }

    async fn latest_published_deposition_for_new_version(
        &self,
        deposition: Deposition,
    ) -> Result<Deposition, ZenodoError> {
        if !deposition.is_published() {
            return Ok(deposition);
        }

        if let Some(latest_url) = deposition.links.latest.as_ref() {
            let self_url = deposition.links.self_.as_ref();
            if self_url != Some(latest_url) {
                return self
                    .resolve_latest_published_deposition_url(latest_url)
                    .await;
            }
        }

        if let Some(record_id) = deposition.record_id {
            let latest_record = self.resolve_latest_version(record_id).await?;
            if latest_record.id.0 != deposition.id.0 {
                return self.get_deposition(DepositionId(latest_record.id.0)).await;
            }
        }

        Ok(deposition)
    }

    async fn resolve_latest_published_deposition_url(
        &self,
        url: &Url,
    ) -> Result<Deposition, ZenodoError> {
        if url.path().contains("/api/deposit/depositions/") {
            return self.get_deposition_by_url(url).await;
        }

        if url.path().contains("/api/records/") {
            let record = self.get_record_by_url(url).await?;
            return self.get_deposition(DepositionId(record.id.0)).await;
        }

        Err(ZenodoError::InvalidState(format!(
            "unsupported latest published deposition link: {url}"
        )))
    }

    async fn upload_spec(
        &self,
        bucket: &crate::ids::BucketUrl,
        spec: UploadSpec,
    ) -> Result<crate::model::BucketObject, ZenodoError> {
        match spec.source {
            UploadSource::Path(path) => {
                self.upload_path_with_content_type(bucket, &spec.filename, &path, spec.content_type)
                    .await
            }
            UploadSource::Reader {
                reader,
                content_length,
            } => {
                self.upload_reader(
                    bucket,
                    &spec.filename,
                    reader,
                    content_length,
                    spec.content_type,
                )
                .await
            }
        }
    }

    async fn wait_for_published_deposition(
        &self,
        id: DepositionId,
    ) -> Result<Deposition, ZenodoError> {
        self.poll_until("publication", || async move {
            let deposition = self.get_deposition(id).await?;
            if deposition.is_published() {
                Ok(Some(deposition))
            } else {
                Ok(None)
            }
        })
        .await
    }

    async fn wait_for_deposition_url(
        &self,
        url: &Url,
        label: &'static str,
    ) -> Result<Deposition, ZenodoError> {
        let url = url.clone();
        self.poll_until(label, || {
            let url = url.clone();
            async move {
                match self.get_deposition_by_url(&url).await {
                    Ok(deposition) => Ok(Some(deposition)),
                    Err(error) if retryable_error(&error) => Ok(None),
                    Err(error) => Err(error),
                }
            }
        })
        .await
    }

    async fn poll_until<F, Fut, T>(
        &self,
        label: &'static str,
        mut attempt: F,
    ) -> Result<T, ZenodoError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Option<T>, ZenodoError>>,
    {
        let started = Instant::now();
        let mut delay = self.poll.initial_delay;

        loop {
            if let Some(value) = attempt().await? {
                return Ok(value);
            }

            let elapsed = started.elapsed();
            if elapsed >= self.poll.max_wait {
                return Err(ZenodoError::Timeout(label));
            }

            let remaining = self.poll.max_wait.saturating_sub(elapsed);
            sleep(std::cmp::min(delay, remaining)).await;
            delay = std::cmp::min(delay.saturating_mul(2), self.poll.max_delay);
        }
    }
}

fn retryable_error(error: &ZenodoError) -> bool {
    match error {
        ZenodoError::Http { status, .. } => {
            *status == reqwest::StatusCode::CONFLICT
                || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
        }
        ZenodoError::Transport(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        collect_uploaded_filenames, editable_draft_action, file_ids_to_delete, retryable_error,
        validate_reconcile_inputs, EditableDraftAction,
    };
    use crate::client::{Auth, ZenodoClient};
    use crate::error::ZenodoError;
    use crate::model::Deposition;
    use crate::upload::{FileReplacePolicy, UploadSpec};
    use crate::{Endpoint, PollOptions};
    use axum::routing::get;
    use axum::{Json, Router};
    use tokio::net::TcpListener;
    use url::Url;

    #[test]
    fn unpublished_deposition_reuses_current_draft() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {}
        }))
        .unwrap();

        assert_eq!(
            editable_draft_action(&deposition),
            EditableDraftAction::ReuseExisting
        );
    }

    #[test]
    fn published_deposition_requires_new_version() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }))
        .unwrap();

        assert_eq!(
            editable_draft_action(&deposition),
            EditableDraftAction::CreateNewVersion
        );
    }

    #[test]
    fn replace_all_deletes_existing_files_first() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [
                { "id": "a", "filename": "one.txt", "filesize": 1 },
                { "id": "b", "filename": "two.txt", "filesize": 2 }
            ],
            "links": {}
        }))
        .unwrap();

        let ids = file_ids_to_delete(
            FileReplacePolicy::ReplaceAll,
            &deposition,
            &std::collections::BTreeSet::new(),
        );
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].0, "a");
        assert_eq!(ids[1].0, "b");
    }

    #[test]
    fn upsert_by_filename_only_deletes_matching_files() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [
                { "id": "a", "filename": "one.txt", "filesize": 1 },
                { "id": "b", "filename": "two.txt", "filesize": 1 }
            ],
            "links": {}
        }))
        .unwrap();

        let uploaded =
            std::collections::BTreeSet::from(["two.txt".to_owned(), "three.txt".to_owned()]);

        let ids = file_ids_to_delete(FileReplacePolicy::UpsertByFilename, &deposition, &uploaded);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].0, "b");
    }

    #[test]
    fn keep_existing_and_add_does_not_delete_existing_files() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [
                { "id": "a", "filename": "one.txt", "filesize": 1 }
            ],
            "links": {}
        }))
        .unwrap();

        let uploaded = std::collections::BTreeSet::from(["one.txt".to_owned()]);
        assert!(file_ids_to_delete(
            FileReplacePolicy::KeepExistingAndAdd,
            &deposition,
            &uploaded
        )
        .is_empty());
    }

    #[test]
    fn duplicate_uploaded_filenames_are_rejected() {
        let files = vec![
            UploadSpec::from_reader(
                "artifact.bin",
                std::io::Cursor::new(vec![1_u8]),
                1,
                mime::APPLICATION_OCTET_STREAM,
            ),
            UploadSpec::from_reader(
                "artifact.bin",
                std::io::Cursor::new(vec![2_u8]),
                1,
                mime::APPLICATION_OCTET_STREAM,
            ),
        ];

        let error = collect_uploaded_filenames(&files).unwrap_err();
        assert!(matches!(
            error,
            ZenodoError::DuplicateUploadFilename { filename } if filename == "artifact.bin"
        ));
    }

    #[test]
    fn keep_existing_and_add_rejects_existing_filename_collisions() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 1,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [{ "id": "stale", "filename": "artifact.bin" }],
            "links": {}
        }))
        .unwrap();
        let uploaded_filenames = ["artifact.bin".to_owned()].into_iter().collect();

        let error = validate_reconcile_inputs(
            FileReplacePolicy::KeepExistingAndAdd,
            &deposition,
            &uploaded_filenames,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ZenodoError::ConflictingDraftFile { filename } if filename == "artifact.bin"
        ));
    }

    #[test]
    fn empty_uploaded_filenames_are_rejected() {
        let files = vec![UploadSpec::from_reader(
            "",
            std::io::Cursor::new(vec![1_u8]),
            1,
            mime::APPLICATION_OCTET_STREAM,
        )];

        let error = collect_uploaded_filenames(&files).unwrap_err();
        assert!(
            matches!(error, ZenodoError::InvalidState(message) if message == "upload filename cannot be empty")
        );
    }

    #[test]
    fn retryable_error_matches_retryable_http_statuses() {
        let conflict = ZenodoError::Http {
            status: reqwest::StatusCode::CONFLICT,
            message: None,
            field_errors: Vec::new(),
            raw_body: None,
        };
        let bad_request = ZenodoError::Http {
            status: reqwest::StatusCode::BAD_REQUEST,
            message: None,
            field_errors: Vec::new(),
            raw_body: None,
        };

        assert!(retryable_error(&conflict));
        assert!(!retryable_error(&bad_request));
    }

    #[tokio::test]
    async fn retryable_error_treats_transport_errors_as_retryable() {
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:9")
            .send()
            .await
            .unwrap_err();
        assert!(retryable_error(&ZenodoError::Transport(error)));
        assert!(!retryable_error(&ZenodoError::Io(std::io::Error::other(
            "io"
        ))));
    }

    #[tokio::test]
    async fn poll_until_does_not_sleep_past_max_wait() {
        let client = ZenodoClient::builder(Auth::new("token"))
            .endpoint(Endpoint::Production)
            .poll_options(PollOptions {
                max_wait: Duration::from_millis(20),
                initial_delay: Duration::from_millis(100),
                max_delay: Duration::from_millis(100),
            })
            .build()
            .unwrap();
        let started = Instant::now();

        let error = client
            .poll_until("probe", || async { Ok::<Option<()>, ZenodoError>(None) })
            .await
            .unwrap_err();

        assert!(matches!(error, ZenodoError::Timeout("probe")));
        assert!(started.elapsed() < Duration::from_millis(80));
    }

    #[tokio::test]
    async fn latest_published_deposition_short_circuits_when_resolution_is_not_needed() {
        let client = ZenodoClient::new(Auth::new("token")).unwrap();
        let unpublished: Deposition = serde_json::from_value(serde_json::json!({
            "id": 10,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {}
        }))
        .unwrap();
        let already_latest: Deposition = serde_json::from_value(serde_json::json!({
            "id": 11,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": "https://zenodo.example/api/deposit/depositions/11",
                "latest": "https://zenodo.example/api/deposit/depositions/11"
            }
        }))
        .unwrap();

        assert_eq!(
            client
                .latest_published_deposition_for_new_version(unpublished.clone())
                .await
                .unwrap()
                .id,
            unpublished.id
        );
        assert_eq!(
            client
                .latest_published_deposition_for_new_version(already_latest.clone())
                .await
                .unwrap()
                .id,
            already_latest.id
        );
    }

    #[tokio::test]
    async fn latest_published_deposition_url_resolves_record_links_and_rejects_unknown_paths() {
        async fn record() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "id": 22,
                "recid": "22",
                "metadata": { "title": "record" },
                "files": [],
                "links": {}
            }))
        }

        async fn deposition() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "id": 22,
                "submitted": true,
                "state": "done",
                "metadata": {},
                "files": [],
                "links": {}
            }))
        }

        let app = Router::new()
            .route("/api/records/22", get(record))
            .route("/api/deposit/depositions/22", get(deposition));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = ZenodoClient::builder(Auth::new("token"))
            .endpoint(Endpoint::Custom(
                Url::parse(&format!("http://{addr}/api/")).unwrap(),
            ))
            .build()
            .unwrap();

        let resolved = client
            .resolve_latest_published_deposition_url(
                &Url::parse(&format!("http://{addr}/api/records/22")).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resolved.id.0, 22);

        let error = client
            .resolve_latest_published_deposition_url(
                &Url::parse(&format!("http://{addr}/something/else")).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, ZenodoError::InvalidState(message) if message.contains("unsupported latest published deposition link"))
        );

        server.abort();
    }
}
