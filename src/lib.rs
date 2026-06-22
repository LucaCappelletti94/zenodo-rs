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

#[cfg(all(feature = "native-tls", feature = "rustls-ring-tls"))]
compile_error!("features `native-tls` and `rustls-ring-tls` are mutually exclusive");

#[cfg(all(feature = "native-tls", feature = "rustls-tls"))]
compile_error!("features `native-tls` and `rustls-tls` are mutually exclusive");

#[cfg(all(feature = "rustls-tls", feature = "rustls-ring-tls"))]
compile_error!("features `rustls-tls` and `rustls-ring-tls` are mutually exclusive");

#[cfg(all(feature = "native-tls", feature = "rustls-no-provider"))]
compile_error!("features `native-tls` and `rustls-no-provider` are mutually exclusive");

#[cfg(all(feature = "rustls-tls", feature = "rustls-no-provider"))]
compile_error!("features `rustls-tls` and `rustls-no-provider` are mutually exclusive");

#[cfg(all(feature = "rustls-no-provider", feature = "rustls-ring-tls"))]
compile_error!("features `rustls-no-provider` and `rustls-ring-tls` are mutually exclusive");

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
pub mod retry;
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
pub use retry::RetryOptions;
pub use upload::{FileReplacePolicy, UploadSource, UploadSpec};
