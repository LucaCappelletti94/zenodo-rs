//! Core data models for depositions, records, and files.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use url::Url;

use crate::ids::{BucketUrl, ConceptRecId, DepositionFileId, DepositionId, Doi, RecordId};
use crate::metadata::RecordMetadata;

macro_rules! string_enum {
    ($(#[$enum_meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$enum_meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
            /// A server value unknown to this crate version.
            Unknown(
                /// Raw server value.
                String
            ),
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(match self {
                    $(Self::$variant => $value,)+
                    Self::Unknown(value) => value.as_str(),
                })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Ok(match value.as_str() {
                    $($value => Self::$variant,)+
                    _ => Self::Unknown(value),
                })
            }
        }
    };
}

string_enum!(
    #[derive(Default)]
    /// High-level Zenodo deposition workflow state.
    DepositState {
        /// Draft work is still in progress.
        #[default]
        InProgress => "inprogress",
        /// Processing completed successfully.
        Done => "done",
        /// Processing failed.
        Error => "error"
    }
);

string_enum!(
    #[derive(Default)]
    /// Publication status for a record payload.
    RecordPublicationStatus {
        /// The record is published and publicly visible.
        #[default]
        Published => "published",
        /// The record is still a draft.
        Draft => "draft"
    }
);

fn deserialize_stringish<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stringish {
        String(String),
        Number(u64),
    }

    match Stringish::deserialize(deserializer)? {
        Stringish::String(value) => Ok(value),
        Stringish::Number(value) => Ok(value.to_string()),
    }
}

/// Combined publication and processing status for a deposition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DepositionStatus {
    /// Whether the deposition has been published.
    #[serde(default)]
    pub submitted: bool,
    /// Zenodo's processing state for the deposition.
    #[serde(default)]
    pub state: DepositState,
}

/// File attached to a draft deposition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositionFile {
    /// Deposition file identifier.
    pub id: DepositionFileId,
    /// Original filename.
    pub filename: String,
    /// File size in bytes.
    #[serde(default)]
    pub filesize: u64,
    /// Reported checksum, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Link relations returned on a deposition resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DepositionLinks {
    /// Canonical API URL for the deposition.
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_: Option<Url>,
    /// Bucket URL used for draft file uploads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BucketUrl>,
    /// URL for the draft file listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Url>,
    /// URL for the publish action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<Url>,
    /// URL for the edit action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<Url>,
    /// URL for the discard action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discard: Option<Url>,
    /// URL for the latest editable draft after `newversion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_draft: Option<Url>,
    /// URL for the latest published record in the family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<Url>,
    /// URL for the versions listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<Url>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Zenodo deposition resource, including draft and published depositions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deposition {
    /// Deposition identifier.
    pub id: DepositionId,
    /// Concept record identifier shared across versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conceptrecid: Option<ConceptRecId>,
    /// Published record identifier, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<RecordId>,
    /// Version-specific DOI, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<Doi>,
    /// Concept DOI shared across versions, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conceptdoi: Option<Doi>,
    /// Publication and processing status fields.
    #[serde(flatten)]
    pub status: DepositionStatus,
    /// Raw deposition metadata.
    #[serde(default)]
    pub metadata: Value,
    /// Files currently visible on the deposition.
    #[serde(default)]
    pub files: Vec<DepositionFile>,
    /// Known deposition link relations.
    #[serde(default)]
    pub links: DepositionLinks,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Deposition {
    /// Returns `true` when the deposition has been published.
    #[must_use]
    pub fn is_published(&self) -> bool {
        self.status.submitted
    }

    /// Returns the `latest_draft` link, when present.
    #[must_use]
    pub fn latest_draft_url(&self) -> Option<&Url> {
        self.links.latest_draft.as_ref()
    }

    /// Returns the bucket upload URL, when present.
    #[must_use]
    pub fn bucket_url(&self) -> Option<&BucketUrl> {
        self.links.bucket.as_ref()
    }
}

/// Result of a successful bucket upload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BucketObject {
    /// Server-side object identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Uploaded object key.
    pub key: String,
    /// Object size in bytes.
    #[serde(default)]
    pub size: u64,
    /// Reported checksum, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Per-file links returned on a record file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordFileLinks {
    /// Canonical API URL for the file.
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_: Option<Url>,
    /// Direct content download URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Url>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// File attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordFile {
    /// Server-side file identifier.
    pub id: String,
    /// File key used for downloads.
    pub key: String,
    /// File size in bytes.
    #[serde(default)]
    pub size: u64,
    /// Reported checksum, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// Known file link relations.
    #[serde(default)]
    pub links: RecordFileLinks,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl RecordFile {
    /// Returns the best download URL for the file.
    #[must_use]
    pub fn download_url(&self) -> Option<&Url> {
        self.links.content.as_ref().or(self.links.self_.as_ref())
    }
}

/// Link relations returned on a record resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordLinks {
    /// Canonical API URL for the record.
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_: Option<Url>,
    /// Canonical HTML page for the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_html: Option<Url>,
    /// Alternate HTML page link, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<Url>,
    /// URL for the latest record version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<Url>,
    /// URL for the versions listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<Url>,
    /// URL for the record's files listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Url>,
    /// URL for downloading the record archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<Url>,
    /// DOI URL, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<Url>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Basic record usage statistics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordStats {
    /// Number of downloads, when Zenodo reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    /// Number of views, when Zenodo reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<u64>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Persistent identifier entry attached to a record or parent record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PersistentIdentifier {
    /// Identifier value.
    pub identifier: String,
    /// PID provider identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// PID client identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Persistent identifier block attached to a record or parent record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordPids {
    /// DOI PID entry, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<PersistentIdentifier>,
    /// Concept DOI PID entry, when present.
    #[serde(
        rename = "concept-doi",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub concept_doi: Option<PersistentIdentifier>,
    /// Record ID PID entry, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recid: Option<PersistentIdentifier>,
    /// OAI PID entry, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oai: Option<PersistentIdentifier>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Parent-community block attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordParentCommunities {
    /// Default community identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Community identifiers attached to the parent record.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Parent-record block attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordParent {
    /// Parent identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Parent communities block, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communities: Option<RecordParentCommunities>,
    /// Parent persistent identifiers block, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids: Option<RecordPids>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Published record resource returned by Zenodo's records API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Record creation timestamp, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime<Utc>>,
    /// Record modification timestamp, when present.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "updated")]
    pub modified: Option<DateTime<Utc>>,
    /// Record identifier.
    pub id: RecordId,
    /// String-form record identifier as returned by Zenodo.
    #[serde(deserialize_with = "deserialize_stringish")]
    pub recid: String,
    /// Concept record identifier shared across versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conceptrecid: Option<ConceptRecId>,
    /// Version-specific DOI, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<Doi>,
    /// Concept DOI shared across versions, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conceptdoi: Option<Doi>,
    /// Typed record metadata.
    #[serde(default)]
    pub metadata: RecordMetadata,
    /// Files exposed on the record.
    #[serde(default)]
    pub files: Vec<RecordFile>,
    /// Known record link relations.
    #[serde(default)]
    pub links: RecordLinks,
    /// Parent record block, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<RecordParent>,
    /// Persistent identifiers block, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids: Option<RecordPids>,
    /// Record usage statistics, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RecordStats>,
    /// Publication status reported by Zenodo.
    #[serde(default)]
    pub status: RecordPublicationStatus,
    /// Revision number, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Record {
    /// Returns the link to the latest record version, when present.
    #[must_use]
    pub fn latest_url(&self) -> Option<&Url> {
        self.links.latest.as_ref()
    }

    /// Returns the record archive link, when present.
    #[must_use]
    pub fn archive_url(&self) -> Option<&Url> {
        self.links.archive.as_ref()
    }

    /// Finds a file by exact key.
    #[must_use]
    pub fn file_by_key(&self, key: &str) -> Option<&RecordFile> {
        self.files.iter().find(|file| file.key == key)
    }
}

/// Aggregated record details used by higher-level artifact helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    /// The originally requested record.
    pub record: Record,
    /// The latest resolved record in the same family.
    pub latest: Record,
    /// Latest record files indexed by key.
    pub files_by_key: BTreeMap<String, RecordFile>,
}

/// Result of a complete publish workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedRecord {
    /// Final deposition payload after publishing.
    pub deposition: Deposition,
    /// Published record fetched from the records API.
    pub record: Record,
}

#[cfg(test)]
mod tests {
    use super::{
        DepositState, Deposition, Record, RecordFile, RecordLinks, RecordPublicationStatus,
    };

    #[test]
    fn preserves_unknown_record_fields() {
        let record: Record = serde_json::from_value(serde_json::json!({
            "created": "2026-04-03T12:00:00+00:00",
            "updated": "2026-04-03T13:00:00+00:00",
            "id": 42,
            "recid": "42",
            "metadata": { "title": "artifact" },
            "files": [],
            "links": {},
            "parent": {
                "id": "parent-1",
                "communities": {
                    "default": "zenodo",
                    "ids": ["zenodo", "sandbox"]
                },
                "pids": {
                    "doi": {
                        "identifier": "10.5281/zenodo.42"
                    }
                }
            },
            "pids": {
                "doi": {
                    "identifier": "10.5281/zenodo.42"
                },
                "concept-doi": {
                    "identifier": "10.5281/zenodo.41"
                }
            },
            "mystery": "value"
        }))
        .unwrap();

        assert!(record.created.is_some());
        assert!(record.modified.is_some());
        assert_eq!(
            record
                .parent
                .as_ref()
                .and_then(|parent| parent.id.as_deref()),
            Some("parent-1")
        );
        assert_eq!(
            record
                .pids
                .as_ref()
                .and_then(|pids| pids.doi.as_ref())
                .map(|pid| pid.identifier.as_str()),
            Some("10.5281/zenodo.42")
        );
        assert_eq!(
            record.extra.get("mystery"),
            Some(&serde_json::Value::String("value".into()))
        );
    }

    #[test]
    fn follows_latest_draft_link() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 7,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": "https://zenodo.org/api/deposit/depositions/8"
            }
        }))
        .unwrap();

        assert_eq!(
            deposition.latest_draft_url().unwrap().as_str(),
            "https://zenodo.org/api/deposit/depositions/8"
        );
    }

    #[test]
    fn record_file_prefers_content_link() {
        let file: RecordFile = serde_json::from_value(serde_json::json!({
            "id": "f1",
            "key": "artifact.bin",
            "links": {
                "self": "https://zenodo.org/api/files/self",
                "content": "https://zenodo.org/api/files/content"
            }
        }))
        .unwrap();

        assert_eq!(
            file.download_url().unwrap().as_str(),
            "https://zenodo.org/api/files/content"
        );
    }

    #[test]
    fn record_and_deposition_helpers_expose_status_and_links() {
        let deposition: Deposition = serde_json::from_value(serde_json::json!({
            "id": 9,
            "submitted": false,
            "state": "mystery-state",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": "https://zenodo.org/api/files/bucket-9"
            }
        }))
        .unwrap();
        let record = Record {
            created: None,
            modified: None,
            id: crate::RecordId(10),
            recid: "10".into(),
            conceptrecid: None,
            doi: None,
            conceptdoi: None,
            metadata: crate::RecordMetadata::default(),
            files: Vec::new(),
            links: RecordLinks::default(),
            parent: None,
            pids: None,
            stats: None,
            status: RecordPublicationStatus::Draft,
            revision: None,
            extra: std::collections::BTreeMap::new(),
        };

        assert!(!deposition.is_published());
        assert!(deposition.bucket_url().is_some());
        assert!(matches!(deposition.status.state, DepositState::Unknown(_)));
        assert!(record.latest_url().is_none());
        assert!(record.archive_url().is_none());
    }
}
