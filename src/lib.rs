#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(
    clippy::all,
    clippy::pedantic,
    clippy::expect_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_used
)]
#![allow(clippy::module_name_repetitions)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::missing_errors_doc,
        clippy::missing_panics_doc,
        clippy::panic,
        clippy::unwrap_used
    )
)]

pub mod client;
mod client_uploader_traits_impl;
pub mod downloads;
pub mod endpoint;
pub mod error;
pub mod ids;
pub mod links;
pub mod metadata;
pub mod model;
pub mod pagination;
pub mod poll;
pub mod progress;
pub mod records;
mod serde_util;
pub mod upload;
pub mod workflow;

pub use client::{Auth, ZenodoClient, ZenodoClientBuilder};
pub use downloads::{DownloadStream, ResolvedDownload};
pub use endpoint::Endpoint;
pub use error::{FieldError, ZenodoError};
pub use ids::{BucketUrl, ConceptRecId, DepositionFileId, DepositionId, Doi, DoiError, RecordId};
pub use metadata::{
    AccessRight, CommunityRef, CommunityRefBuilder, Contributor, ContributorBuilder, Creator,
    CreatorBuilder, DepositMetadataBuildError, DepositMetadataUpdate, DepositMetadataUpdateBuilder,
    GrantRef, GrantRefBuilder, LicenseRef, LicenseRefBuilder, MetadataEntryBuildError, RecordDate,
    RecordDateBuilder, RecordIdentifier, RecordIdentifierBuilder, RecordMetadata, RecordRelations,
    RecordVersionRelation, RelatedIdentifier, RelatedIdentifierBuilder, ResourceType,
    ResourceTypeBuilder, Subject, SubjectBuilder, UploadType,
};
pub use model::{
    ArtifactInfo, BucketObject, DepositState, Deposition, DepositionFile, DepositionLinks,
    DepositionStatus, PersistentIdentifier, PublishedRecord, Record, RecordFile, RecordFileLinks,
    RecordLinks, RecordParent, RecordParentCommunities, RecordPids, RecordPublicationStatus,
    RecordStats,
};
pub use pagination::Page;
pub use poll::PollOptions;
pub use progress::TransferProgress;
pub use records::{
    ArtifactSelector, RecordQuery, RecordQueryBuilder, RecordQueryStatus, RecordSelector,
    RecordSort,
};
pub use upload::{FileReplacePolicy, UploadSource, UploadSpec};
