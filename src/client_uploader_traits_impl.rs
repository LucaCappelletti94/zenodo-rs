use std::path::Path;
use std::time::Duration;

use client_uploader_traits::{
    ClientContext, CreatePublication, CreatePublicationRequest, DoiBackedRecord,
    DownloadNamedPublicFile, DraftFilePolicy, DraftFilePolicyKind, DraftResource, DraftState,
    DraftWorkflow, ListResourceFiles, LookupByDoi, MaybeAuthenticatedClient,
    MutablePublicationOutcome, NoCreateTarget, PublicationOutcome, ReadPublicResource,
    RepositoryFile, RepositoryRecord, ResolveLatestPublicResource,
    ResolveLatestPublicResourceByDoi, SearchPublicResources, SearchResultsLike, UpdatePublication,
    UpdatePublicationRequest, UploadSourceKind, UploadSpecLike,
};
use secrecy::ExposeSecret;
use url::Url;

use crate::client::ZenodoClient;
use crate::downloads::ResolvedDownload;
use crate::endpoint::Endpoint;
use crate::error::ZenodoError;
use crate::ids::{DepositionFileId, DepositionId, Doi, RecordId};
use crate::metadata::DepositMetadataUpdate;
use crate::model::{BucketObject, Deposition, DepositionFile, PublishedRecord, Record, RecordFile};
use crate::pagination::Page;
use crate::poll::PollOptions;
use crate::records::RecordQuery;
use crate::upload::{FileReplacePolicy, UploadSource, UploadSpec};

impl ClientContext for ZenodoClient {
    type Endpoint = Endpoint;
    type PollOptions = PollOptions;
    type Error = ZenodoError;

    fn endpoint(&self) -> &Self::Endpoint {
        ZenodoClient::endpoint(self)
    }

    fn poll_options(&self) -> &Self::PollOptions {
        ZenodoClient::poll_options(self)
    }

    fn request_timeout(&self) -> Option<Duration> {
        ZenodoClient::request_timeout(self)
    }

    fn connect_timeout(&self) -> Option<Duration> {
        ZenodoClient::connect_timeout(self)
    }
}

impl MaybeAuthenticatedClient for ZenodoClient {
    fn has_auth(&self) -> bool {
        !self.auth.token.expose_secret().is_empty()
    }
}

impl UploadSpecLike for UploadSpec {
    fn filename(&self) -> &str {
        &self.filename
    }

    fn source_kind(&self) -> UploadSourceKind {
        match self.source {
            UploadSource::Path(_) => UploadSourceKind::Path,
            UploadSource::Reader { .. } => UploadSourceKind::Reader,
        }
    }

    fn content_length(&self) -> Option<u64> {
        UploadSpec::content_length(self).ok()
    }

    fn content_type(&self) -> Option<&str> {
        Some(self.content_type.as_ref())
    }
}

impl RepositoryFile for DepositionFile {
    type Id = DepositionFileId;

    fn file_id(&self) -> Option<Self::Id> {
        Some(self.id.clone())
    }

    fn file_name(&self) -> &str {
        &self.filename
    }

    fn size_bytes(&self) -> Option<u64> {
        Some(self.filesize)
    }

    fn checksum(&self) -> Option<&str> {
        self.checksum.as_deref()
    }
}

impl RepositoryFile for BucketObject {
    type Id = String;

    fn file_id(&self) -> Option<Self::Id> {
        self.id.clone()
    }

    fn file_name(&self) -> &str {
        &self.key
    }

    fn size_bytes(&self) -> Option<u64> {
        Some(self.size)
    }

    fn checksum(&self) -> Option<&str> {
        self.checksum.as_deref()
    }
}

impl RepositoryFile for RecordFile {
    type Id = String;

    fn file_id(&self) -> Option<Self::Id> {
        Some(self.id.clone())
    }

    fn file_name(&self) -> &str {
        &self.key
    }

    fn size_bytes(&self) -> Option<u64> {
        Some(self.size)
    }

    fn checksum(&self) -> Option<&str> {
        self.checksum.as_deref()
    }

    fn download_url(&self) -> Option<&Url> {
        RecordFile::download_url(self)
    }
}

impl RepositoryRecord for Record {
    type Id = RecordId;
    type File = RecordFile;

    fn resource_id(&self) -> Option<Self::Id> {
        Some(self.id)
    }

    fn title(&self) -> Option<&str> {
        Some(&self.metadata.title)
    }

    fn files(&self) -> &[Self::File] {
        &self.files
    }
}

impl DoiBackedRecord for Record {
    type Doi = Doi;

    fn doi(&self) -> Option<Self::Doi> {
        self.doi.clone()
    }
}

impl RepositoryRecord for Deposition {
    type Id = DepositionId;
    type File = DepositionFile;

    fn resource_id(&self) -> Option<Self::Id> {
        Some(self.id)
    }

    fn title(&self) -> Option<&str> {
        self.metadata
            .get("title")
            .and_then(serde_json::Value::as_str)
    }

    fn files(&self) -> &[Self::File] {
        &self.files
    }
}

impl DoiBackedRecord for Deposition {
    type Doi = Doi;

    fn doi(&self) -> Option<Self::Doi> {
        self.doi.clone()
    }
}

impl DraftResource for Deposition {
    type Id = DepositionId;
    type File = DepositionFile;

    fn draft_id(&self) -> Self::Id {
        self.id
    }

    fn files(&self) -> &[Self::File] {
        &self.files
    }
}

impl DraftState for Deposition {
    fn is_published(&self) -> bool {
        Deposition::is_published(self)
    }

    fn allows_metadata_updates(&self) -> bool {
        Deposition::allows_metadata_edits(self)
    }
}

impl PublicationOutcome for PublishedRecord {
    type PublicResource = Record;

    fn public_resource(&self) -> &Self::PublicResource {
        &self.record
    }
}

impl MutablePublicationOutcome for PublishedRecord {
    type MutableResource = Deposition;

    fn mutable_resource(&self) -> Option<&Self::MutableResource> {
        Some(&self.deposition)
    }
}

impl<T> SearchResultsLike for Page<T> {
    type Item = T;

    fn items(&self) -> &[Self::Item] {
        &self.hits
    }

    fn total_hits(&self) -> Option<u64> {
        self.total
    }
}

impl ReadPublicResource for ZenodoClient {
    type ResourceId = RecordId;
    type Resource = Record;

    async fn get_public_resource(
        &self,
        id: &Self::ResourceId,
    ) -> Result<Self::Resource, Self::Error> {
        ZenodoClient::get_record(self, *id).await
    }
}

impl SearchPublicResources for ZenodoClient {
    type Query = RecordQuery;
    type SearchResults = Page<Record>;

    async fn search_public_resources(
        &self,
        query: &Self::Query,
    ) -> Result<Self::SearchResults, Self::Error> {
        ZenodoClient::search_records(self, query).await
    }
}

impl ListResourceFiles for ZenodoClient {
    type ResourceId = RecordId;
    type File = RecordFile;

    async fn list_resource_files(
        &self,
        id: &Self::ResourceId,
    ) -> Result<Vec<Self::File>, Self::Error> {
        ZenodoClient::list_record_files(self, *id).await
    }
}

impl DownloadNamedPublicFile for ZenodoClient {
    type ResourceId = RecordId;
    type Download = ResolvedDownload;

    async fn download_named_public_file_to_path(
        &self,
        id: &Self::ResourceId,
        name: &str,
        path: &Path,
    ) -> Result<Self::Download, Self::Error> {
        ZenodoClient::download_record_file_by_key_to_path(self, *id, name, path).await
    }
}

impl CreatePublication for ZenodoClient {
    type CreateTarget = NoCreateTarget;
    type Metadata = DepositMetadataUpdate;
    type Upload = UploadSpec;
    type Output = PublishedRecord;

    async fn create_publication(
        &self,
        request: CreatePublicationRequest<Self::CreateTarget, Self::Metadata, Self::Upload>,
    ) -> Result<Self::Output, Self::Error> {
        let CreatePublicationRequest {
            target: _,
            metadata,
            uploads,
        } = request;
        ZenodoClient::create_and_publish_dataset(self, &metadata, uploads).await
    }
}

impl UpdatePublication for ZenodoClient {
    type ResourceId = DepositionId;
    type Metadata = DepositMetadataUpdate;
    type FilePolicy = FileReplacePolicy;
    type Upload = UploadSpec;
    type Output = PublishedRecord;

    async fn update_publication(
        &self,
        request: UpdatePublicationRequest<
            Self::ResourceId,
            Self::Metadata,
            Self::FilePolicy,
            Self::Upload,
        >,
    ) -> Result<Self::Output, Self::Error> {
        let UpdatePublicationRequest {
            resource_id,
            metadata,
            policy,
            uploads,
        } = request;
        ZenodoClient::publish_dataset_with_policy(self, resource_id, &metadata, policy, uploads)
            .await
    }
}

impl LookupByDoi for ZenodoClient {
    type Doi = Doi;
    type Resource = Record;

    async fn get_public_resource_by_doi(
        &self,
        doi: &Self::Doi,
    ) -> Result<Self::Resource, Self::Error> {
        ZenodoClient::get_record_by_doi(self, doi).await
    }
}

impl ResolveLatestPublicResource for ZenodoClient {
    type ResourceId = RecordId;
    type Resource = Record;

    async fn resolve_latest_public_resource(
        &self,
        id: &Self::ResourceId,
    ) -> Result<Self::Resource, Self::Error> {
        ZenodoClient::resolve_latest_version(self, *id).await
    }
}

impl ResolveLatestPublicResourceByDoi for ZenodoClient {
    type Doi = Doi;
    type Resource = Record;

    async fn resolve_latest_public_resource_by_doi(
        &self,
        doi: &Self::Doi,
    ) -> Result<Self::Resource, Self::Error> {
        ZenodoClient::resolve_latest_by_doi(self, doi).await
    }
}

impl DraftFilePolicy for FileReplacePolicy {
    fn kind(&self) -> DraftFilePolicyKind {
        match self {
            Self::ReplaceAll => DraftFilePolicyKind::ReplaceAll,
            Self::UpsertByFilename => DraftFilePolicyKind::UpsertByFilename,
            Self::KeepExistingAndAdd => DraftFilePolicyKind::KeepExistingAndAdd,
        }
    }
}

impl DraftWorkflow for ZenodoClient {
    type Draft = Deposition;
    type Metadata = DepositMetadataUpdate;
    type Upload = UploadSpec;
    type FilePolicy = FileReplacePolicy;
    type UploadResult = BucketObject;
    type Published = Deposition;

    async fn create_draft(&self, metadata: &Self::Metadata) -> Result<Self::Draft, Self::Error> {
        let draft = ZenodoClient::create_deposition(self).await?;
        ZenodoClient::update_metadata(self, draft.id, metadata).await
    }

    async fn update_draft_metadata(
        &self,
        draft_id: &<Self::Draft as DraftResource>::Id,
        metadata: &Self::Metadata,
    ) -> Result<Self::Draft, Self::Error> {
        ZenodoClient::update_metadata(self, *draft_id, metadata).await
    }

    async fn reconcile_draft_files(
        &self,
        draft: &Self::Draft,
        policy: Self::FilePolicy,
        uploads: Vec<Self::Upload>,
    ) -> Result<Vec<Self::UploadResult>, Self::Error> {
        ZenodoClient::reconcile_files(self, draft, policy, uploads).await
    }

    async fn publish_draft(
        &self,
        draft_id: &<Self::Draft as DraftResource>::Id,
    ) -> Result<Self::Published, Self::Error> {
        ZenodoClient::publish(self, *draft_id).await
    }
}

#[cfg(test)]
mod tests {
    use client_uploader_traits::{
        collect_upload_filenames, CoreRepositoryClient, DoiVersionedRepositoryClient,
        DraftPublishingRepositoryClient, MaybeAuthenticatedClient, MutablePublicationOutcome,
        PublicationOutcome, RepositoryFile, RepositoryRecord, SearchResultsLike,
    };
    use serde_json::json;

    use super::*;
    use crate::client::Auth;

    fn assert_core_client<T>()
    where
        T: CoreRepositoryClient,
    {
    }

    fn assert_doi_client<T>()
    where
        T: DoiVersionedRepositoryClient,
    {
    }

    fn assert_draft_client<T>()
    where
        T: DraftPublishingRepositoryClient,
    {
    }

    #[test]
    fn zenodo_client_satisfies_repository_client_bundles() {
        assert_core_client::<ZenodoClient>();
        assert_doi_client::<ZenodoClient>();
        assert_draft_client::<ZenodoClient>();
    }

    #[test]
    fn client_context_and_auth_traits_reflect_client_configuration() {
        let client = ZenodoClient::new(Auth::new("token")).unwrap();

        assert!(client.has_auth());
        assert_eq!(ClientContext::request_timeout(&client), None);
        assert_eq!(ClientContext::connect_timeout(&client), None);
        assert_eq!(
            ClientContext::poll_options(&client),
            &PollOptions::default()
        );
        assert!(matches!(
            ClientContext::endpoint(&client),
            &Endpoint::Production
        ));
    }

    #[test]
    fn upload_spec_trait_reports_filename_source_kind_and_metadata() {
        let spec = UploadSpec::from_reader(
            "artifact.bin",
            std::io::Cursor::new(vec![1_u8, 2, 3]),
            3,
            mime::APPLICATION_OCTET_STREAM,
        );

        assert_eq!(spec.filename(), "artifact.bin");
        assert_eq!(spec.source_kind(), UploadSourceKind::Reader);
        assert_eq!(UploadSpecLike::content_length(&spec), Some(3));
        assert_eq!(
            UploadSpecLike::content_type(&spec),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn shared_upload_filename_helper_accepts_zenodo_upload_specs() {
        let uploads = [
            UploadSpec::from_reader(
                "artifact.bin",
                std::io::Cursor::new(vec![1_u8]),
                1,
                mime::APPLICATION_OCTET_STREAM,
            ),
            UploadSpec::from_reader(
                "manifest.json",
                std::io::Cursor::new(vec![2_u8]),
                1,
                mime::APPLICATION_JSON,
            ),
        ];

        let filenames = collect_upload_filenames(uploads.iter()).unwrap();
        assert!(filenames.contains("artifact.bin"));
        assert!(filenames.contains("manifest.json"));
    }

    #[test]
    fn file_policy_trait_matches_zenodo_replace_policy_variants() {
        assert_eq!(
            DraftFilePolicy::kind(&FileReplacePolicy::ReplaceAll),
            DraftFilePolicyKind::ReplaceAll
        );
        assert_eq!(
            DraftFilePolicy::kind(&FileReplacePolicy::UpsertByFilename),
            DraftFilePolicyKind::UpsertByFilename
        );
        assert_eq!(
            DraftFilePolicy::kind(&FileReplacePolicy::KeepExistingAndAdd),
            DraftFilePolicyKind::KeepExistingAndAdd
        );
    }

    #[test]
    fn record_related_traits_expose_expected_views() {
        let record: Record = serde_json::from_value(json!({
            "id": 42,
            "recid": "42",
            "doi": "10.5281/zenodo.42",
            "metadata": { "title": "Record title" },
            "files": [{
                "id": "file-1",
                "key": "artifact.bin",
                "size": 3,
                "checksum": "md5:abc",
                "links": {
                    "content": "https://example.invalid/content"
                }
            }],
            "links": {}
        }))
        .unwrap();

        assert_eq!(record.resource_id(), Some(RecordId(42)));
        assert_eq!(record.title(), Some("Record title"));
        assert_eq!(record.doi(), Some(Doi::new("10.5281/zenodo.42").unwrap()));
        assert_eq!(record.files()[0].file_name(), "artifact.bin");
        assert_eq!(record.files()[0].size_bytes(), Some(3));
        assert_eq!(record.files()[0].checksum(), Some("md5:abc"));
        assert_eq!(
            record.files()[0].download_url().map(Url::as_str),
            Some("https://example.invalid/content")
        );
    }

    #[test]
    fn bucket_object_and_deposition_expose_shared_repository_views() {
        let uploaded = BucketObject {
            id: Some("bucket-file".to_owned()),
            key: "artifact.bin".to_owned(),
            size: 3,
            checksum: Some("md5:abc".to_owned()),
            extra: std::collections::BTreeMap::default(),
        };
        let deposition: Deposition = serde_json::from_value(json!({
            "id": 7,
            "doi": "10.5281/zenodo.7",
            "submitted": false,
            "state": "inprogress",
            "metadata": {
                "title": "Draft title"
            },
            "files": [{
                "id": "draft-file",
                "filename": "artifact.bin",
                "filesize": 3
            }],
            "links": {}
        }))
        .unwrap();

        assert_eq!(uploaded.file_id(), Some("bucket-file".to_owned()));
        assert_eq!(uploaded.file_name(), "artifact.bin");
        assert_eq!(uploaded.size_bytes(), Some(3));
        assert_eq!(uploaded.checksum(), Some("md5:abc"));

        assert_eq!(deposition.resource_id(), Some(DepositionId(7)));
        assert_eq!(deposition.title(), Some("Draft title"));
        assert_eq!(
            deposition.doi(),
            Some(Doi::new("10.5281/zenodo.7").unwrap())
        );
        assert_eq!(
            RepositoryRecord::files(&deposition)[0].file_name(),
            "artifact.bin"
        );
    }

    #[test]
    fn deposition_and_publication_traits_expose_expected_views() {
        let deposition: Deposition = serde_json::from_value(json!({
            "id": 7,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [{
                "id": "draft-file",
                "filename": "artifact.bin",
                "filesize": 3
            }],
            "links": {}
        }))
        .unwrap();
        let record: Record = serde_json::from_value(json!({
            "id": 8,
            "recid": "8",
            "metadata": { "title": "Published title" },
            "files": [],
            "links": {}
        }))
        .unwrap();
        let published = PublishedRecord {
            deposition: deposition.clone(),
            record,
        };

        assert_eq!(deposition.draft_id(), DepositionId(7));
        assert!(!DraftState::is_published(&deposition));
        assert!(DraftState::allows_metadata_updates(&deposition));
        assert_eq!(
            DraftResource::files(&deposition)[0].file_id(),
            Some(DepositionFileId::from("draft-file"))
        );
        assert_eq!(
            DraftResource::files(&deposition)[0].file_name(),
            "artifact.bin"
        );
        assert_eq!(DraftResource::files(&deposition)[0].size_bytes(), Some(3));
        assert_eq!(published.public_resource().id, RecordId(8));
        assert_eq!(published.created(), None);
        assert_eq!(
            published.mutable_resource().map(DraftResource::draft_id),
            Some(DepositionId(7))
        );
    }

    #[test]
    fn page_trait_exposes_hits_and_total() {
        let page: Page<Record> = Page {
            hits: vec![
                serde_json::from_value(json!({
                    "id": 1,
                    "recid": "1",
                    "metadata": { "title": "one" },
                    "files": [],
                    "links": {}
                }))
                .unwrap(),
                serde_json::from_value(json!({
                    "id": 2,
                    "recid": "2",
                    "metadata": { "title": "two" },
                    "files": [],
                    "links": {}
                }))
                .unwrap(),
            ],
            total: Some(10),
            next: None,
            prev: None,
        };

        assert_eq!(page.items().len(), 2);
        assert_eq!(page.total_hits(), Some(10));
        assert_eq!(page.page_len(), 2);
        assert!(!page.is_empty());
    }
}
