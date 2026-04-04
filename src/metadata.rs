//! Typed metadata models for deposit updates and published records.
//!
//! This module contains the typed request and response shapes that matter most
//! for Zenodo publishing and retrieval.
//!
//! The most important entrypoints are:
//!
//! - [`DepositMetadataUpdate::builder`] for draft metadata updates
//! - [`Creator::builder`] and the other small builders for nested metadata
//! - [`RecordMetadata`] for typed fields on published records
//!
//! Unknown Zenodo fields are still preserved through flattened `extra` maps so
//! the crate remains forward compatible with mild schema drift.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use thiserror::Error;
use url::Url;

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
    /// Zenodo upload-type vocabulary for deposit metadata.
    UploadType {
        /// Dataset upload type.
        Dataset => "dataset",
        /// Publication upload type.
        Publication => "publication",
        /// Poster upload type.
        Poster => "poster",
        /// Presentation upload type.
        Presentation => "presentation",
        /// Software upload type.
        Software => "software",
        /// Image upload type.
        Image => "image",
        /// Video upload type.
        Video => "video",
        /// Lesson upload type.
        Lesson => "lesson",
        /// Physical object upload type.
        PhysicalObject => "physicalobject",
        /// Other upload type.
        Other => "other"
    }
);

string_enum!(
    /// Zenodo access-right vocabulary for deposit metadata.
    AccessRight {
        /// Open access.
        Open => "open",
        /// Embargoed access.
        Embargoed => "embargoed",
        /// Restricted access.
        Restricted => "restricted",
        /// Closed access.
        Closed => "closed"
    }
);

/// Creator entry used by Zenodo metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Creator {
    /// Full creator name in Zenodo's expected display form.
    pub name: String,
    /// Affiliation for the creator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
    /// ORCID identifier for the creator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orcid: Option<String>,
    /// GND identifier for the creator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gnd: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Creator {
    /// Starts building a creator entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::Creator;
    ///
    /// let creator = Creator::builder()
    ///     .name("Doe, Jane")
    ///     .affiliation("Zenodo")
    ///     .build()?;
    ///
    /// assert_eq!(creator.name, "Doe, Jane");
    /// assert_eq!(creator.affiliation.as_deref(), Some("Zenodo"));
    /// # Ok::<(), zenodo_rs::MetadataEntryBuildError>(())
    /// ```
    #[must_use]
    pub fn builder() -> CreatorBuilder {
        CreatorBuilder::default()
    }

    /// Creates a creator entry with only the required name field.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::Creator;
    ///
    /// let creator = Creator::named("Doe, Jane");
    /// assert_eq!(creator.name, "Doe, Jane");
    /// assert!(creator.affiliation.is_none());
    /// ```
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`Creator`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CreatorBuilder {
    name: Option<String>,
    affiliation: Option<String>,
    orcid: Option<String>,
    gnd: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl CreatorBuilder {
    /// Sets the creator name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the creator affiliation.
    #[must_use]
    pub fn affiliation(mut self, affiliation: impl Into<String>) -> Self {
        self.affiliation = Some(affiliation.into());
        self
    }

    /// Sets the creator ORCID.
    #[must_use]
    pub fn orcid(mut self, orcid: impl Into<String>) -> Self {
        self.orcid = Some(orcid.into());
        self
    }

    /// Sets the creator GND identifier.
    #[must_use]
    pub fn gnd(mut self, gnd: impl Into<String>) -> Self {
        self.gnd = Some(gnd.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the creator entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `name` field is missing.
    pub fn build(self) -> Result<Creator, MetadataEntryBuildError> {
        Ok(Creator {
            name: required_entry_field(self.name, "Creator", "name")?,
            affiliation: self.affiliation,
            orcid: self.orcid,
            gnd: self.gnd,
            extra: self.extra,
        })
    }
}

/// Contributor entry used by published-record metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Contributor {
    /// Full contributor name in Zenodo's expected display form.
    pub name: String,
    /// Contributor role label as reported by Zenodo.
    #[serde(rename = "type")]
    pub type_: String,
    /// Affiliation for the contributor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
    /// ORCID identifier for the contributor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orcid: Option<String>,
    /// GND identifier for the contributor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gnd: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Contributor {
    /// Starts building a contributor entry.
    #[must_use]
    pub fn builder() -> ContributorBuilder {
        ContributorBuilder::default()
    }

    /// Creates a contributor entry with the required name and role fields.
    #[must_use]
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_: role.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`Contributor`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ContributorBuilder {
    name: Option<String>,
    role: Option<String>,
    affiliation: Option<String>,
    orcid: Option<String>,
    gnd: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl ContributorBuilder {
    /// Sets the contributor name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the contributor role.
    #[must_use]
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// Sets the contributor affiliation.
    #[must_use]
    pub fn affiliation(mut self, affiliation: impl Into<String>) -> Self {
        self.affiliation = Some(affiliation.into());
        self
    }

    /// Sets the contributor ORCID.
    #[must_use]
    pub fn orcid(mut self, orcid: impl Into<String>) -> Self {
        self.orcid = Some(orcid.into());
        self
    }

    /// Sets the contributor GND identifier.
    #[must_use]
    pub fn gnd(mut self, gnd: impl Into<String>) -> Self {
        self.gnd = Some(gnd.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the contributor entry.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` or `role` is missing.
    pub fn build(self) -> Result<Contributor, MetadataEntryBuildError> {
        Ok(Contributor {
            name: required_entry_field(self.name, "Contributor", "name")?,
            type_: required_entry_field(self.role, "Contributor", "role")?,
            affiliation: self.affiliation,
            orcid: self.orcid,
            gnd: self.gnd,
            extra: self.extra,
        })
    }
}

/// Subject classification attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Subject {
    /// Subject term or label.
    pub term: String,
    /// Subject identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Subject classification scheme, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Subject {
    /// Starts building a subject entry.
    #[must_use]
    pub fn builder() -> SubjectBuilder {
        SubjectBuilder::default()
    }

    /// Creates a subject entry with only the required term field.
    #[must_use]
    pub fn new(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`Subject`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SubjectBuilder {
    term: Option<String>,
    identifier: Option<String>,
    scheme: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl SubjectBuilder {
    /// Sets the subject term.
    #[must_use]
    pub fn term(mut self, term: impl Into<String>) -> Self {
        self.term = Some(term.into());
        self
    }

    /// Sets the subject identifier.
    #[must_use]
    pub fn identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }

    /// Sets the subject scheme.
    #[must_use]
    pub fn scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = Some(scheme.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the subject entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `term` field is missing.
    pub fn build(self) -> Result<Subject, MetadataEntryBuildError> {
        Ok(Subject {
            term: required_entry_field(self.term, "Subject", "term")?,
            identifier: self.identifier,
            scheme: self.scheme,
            extra: self.extra,
        })
    }
}

/// Additional identifier attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordIdentifier {
    /// Identifier value.
    pub identifier: String,
    /// Identifier scheme, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl RecordIdentifier {
    /// Starts building a record identifier entry.
    #[must_use]
    pub fn builder() -> RecordIdentifierBuilder {
        RecordIdentifierBuilder::default()
    }

    /// Creates an identifier entry with only the required identifier field.
    #[must_use]
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`RecordIdentifier`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RecordIdentifierBuilder {
    identifier: Option<String>,
    scheme: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl RecordIdentifierBuilder {
    /// Sets the identifier value.
    #[must_use]
    pub fn identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }

    /// Sets the identifier scheme.
    #[must_use]
    pub fn scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = Some(scheme.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the identifier entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `identifier` field is missing.
    pub fn build(self) -> Result<RecordIdentifier, MetadataEntryBuildError> {
        Ok(RecordIdentifier {
            identifier: required_entry_field(self.identifier, "RecordIdentifier", "identifier")?,
            scheme: self.scheme,
            extra: self.extra,
        })
    }
}

/// Date entry attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordDate {
    /// Date value exactly as reported by Zenodo.
    pub date: String,
    /// Date type label, when present.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    /// Human-readable date description, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl RecordDate {
    /// Starts building a record date entry.
    #[must_use]
    pub fn builder() -> RecordDateBuilder {
        RecordDateBuilder::default()
    }

    /// Creates a record date entry with only the required date field.
    #[must_use]
    pub fn new(date: impl Into<String>) -> Self {
        Self {
            date: date.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`RecordDate`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RecordDateBuilder {
    date: Option<String>,
    date_type: Option<String>,
    description: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl RecordDateBuilder {
    /// Sets the raw date value.
    #[must_use]
    pub fn date(mut self, date: impl Into<String>) -> Self {
        self.date = Some(date.into());
        self
    }

    /// Sets the Zenodo date type label.
    #[must_use]
    pub fn date_type(mut self, date_type: impl Into<String>) -> Self {
        self.date_type = Some(date_type.into());
        self
    }

    /// Sets the human-readable date description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the date entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `date` field is missing.
    pub fn build(self) -> Result<RecordDate, MetadataEntryBuildError> {
        Ok(RecordDate {
            date: required_entry_field(self.date, "RecordDate", "date")?,
            type_: self.date_type,
            description: self.description,
            extra: self.extra,
        })
    }
}

/// Version-relation details reported for a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordVersionRelation {
    /// Zero-based index of the current version, when Zenodo reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    /// Total number of known versions, when Zenodo reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Whether this version is the latest known version, when Zenodo reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_last: Option<bool>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Relation blocks attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordRelations {
    /// Version-ordering relation details, when Zenodo reports them.
    #[serde(default)]
    pub version: Vec<RecordVersionRelation>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Related identifier entry in Zenodo metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RelatedIdentifier {
    /// Related identifier value.
    pub identifier: String,
    /// Relation type string used by Zenodo.
    pub relation: String,
    /// Identifier scheme, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Related resource type, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl RelatedIdentifier {
    /// Starts building a related identifier entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::RelatedIdentifier;
    ///
    /// let related = RelatedIdentifier::builder()
    ///     .identifier("10.5281/zenodo.42")
    ///     .relation("isSupplementTo")
    ///     .scheme("doi")
    ///     .build()?;
    ///
    /// assert_eq!(related.relation, "isSupplementTo");
    /// # Ok::<(), zenodo_rs::MetadataEntryBuildError>(())
    /// ```
    #[must_use]
    pub fn builder() -> RelatedIdentifierBuilder {
        RelatedIdentifierBuilder::default()
    }

    /// Creates a related identifier entry with required identifier and relation fields.
    #[must_use]
    pub fn new(identifier: impl Into<String>, relation: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            relation: relation.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`RelatedIdentifier`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RelatedIdentifierBuilder {
    identifier: Option<String>,
    relation: Option<String>,
    scheme: Option<String>,
    resource_type: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl RelatedIdentifierBuilder {
    /// Sets the identifier value.
    #[must_use]
    pub fn identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }

    /// Sets the relation type.
    #[must_use]
    pub fn relation(mut self, relation: impl Into<String>) -> Self {
        self.relation = Some(relation.into());
        self
    }

    /// Sets the identifier scheme.
    #[must_use]
    pub fn scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = Some(scheme.into());
        self
    }

    /// Sets the related resource type.
    #[must_use]
    pub fn resource_type(mut self, resource_type: impl Into<String>) -> Self {
        self.resource_type = Some(resource_type.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the related identifier entry.
    ///
    /// # Errors
    ///
    /// Returns an error if `identifier` or `relation` is missing.
    pub fn build(self) -> Result<RelatedIdentifier, MetadataEntryBuildError> {
        Ok(RelatedIdentifier {
            identifier: required_entry_field(self.identifier, "RelatedIdentifier", "identifier")?,
            relation: required_entry_field(self.relation, "RelatedIdentifier", "relation")?,
            scheme: self.scheme,
            resource_type: self.resource_type,
            extra: self.extra,
        })
    }
}

/// Reference to a Zenodo community.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommunityRef {
    /// Community identifier.
    pub identifier: String,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl CommunityRef {
    /// Starts building a community reference.
    #[must_use]
    pub fn builder() -> CommunityRefBuilder {
        CommunityRefBuilder::default()
    }

    /// Creates a community reference from its identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::CommunityRef;
    ///
    /// let community = CommunityRef::new("zenodo");
    /// assert_eq!(community.identifier, "zenodo");
    /// ```
    #[must_use]
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`CommunityRef`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommunityRefBuilder {
    identifier: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl CommunityRefBuilder {
    /// Sets the community identifier.
    #[must_use]
    pub fn identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the community reference.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `identifier` field is missing.
    pub fn build(self) -> Result<CommunityRef, MetadataEntryBuildError> {
        Ok(CommunityRef {
            identifier: required_entry_field(self.identifier, "CommunityRef", "identifier")?,
            extra: self.extra,
        })
    }
}

/// Reference to a Zenodo grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GrantRef {
    /// Grant identifier.
    pub id: String,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl GrantRef {
    /// Starts building a grant reference.
    #[must_use]
    pub fn builder() -> GrantRefBuilder {
        GrantRefBuilder::default()
    }

    /// Creates a grant reference from its identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::GrantRef;
    ///
    /// let grant = GrantRef::new("grant-1");
    /// assert_eq!(grant.id, "grant-1");
    /// ```
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

/// Builder for [`GrantRef`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GrantRefBuilder {
    id: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl GrantRefBuilder {
    /// Sets the grant identifier.
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the grant reference.
    ///
    /// # Errors
    ///
    /// Returns an error if the required `id` field is missing.
    pub fn build(self) -> Result<GrantRef, MetadataEntryBuildError> {
        Ok(GrantRef {
            id: required_entry_field(self.id, "GrantRef", "id")?,
            extra: self.extra,
        })
    }
}

/// License metadata attached to a published record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LicenseRef {
    /// License identifier, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Human-readable license title, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl LicenseRef {
    /// Starts building a license reference.
    #[must_use]
    pub fn builder() -> LicenseRefBuilder {
        LicenseRefBuilder::default()
    }

    /// Creates a license reference from a license identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            ..Self::default()
        }
    }
}

/// Builder for [`LicenseRef`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LicenseRefBuilder {
    id: Option<String>,
    title: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl LicenseRefBuilder {
    /// Sets the license identifier.
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the human-readable license title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the license reference.
    #[must_use]
    pub fn build(self) -> LicenseRef {
        LicenseRef {
            id: self.id,
            title: self.title,
            extra: self.extra,
        }
    }
}

/// Resource type details on published records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceType {
    /// Top-level Zenodo resource type.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    /// Zenodo resource subtype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    /// Human-readable resource type title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl ResourceType {
    /// Starts building a resource type entry.
    #[must_use]
    pub fn builder() -> ResourceTypeBuilder {
        ResourceTypeBuilder::default()
    }

    /// Creates a resource type entry from a top-level type string.
    #[must_use]
    pub fn new(type_: impl Into<String>) -> Self {
        Self {
            type_: Some(type_.into()),
            ..Self::default()
        }
    }
}

/// Builder for [`ResourceType`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ResourceTypeBuilder {
    type_: Option<String>,
    subtype: Option<String>,
    title: Option<String>,
    extra: BTreeMap<String, Value>,
}

impl ResourceTypeBuilder {
    /// Sets the top-level resource type.
    #[must_use]
    pub fn type_(mut self, type_: impl Into<String>) -> Self {
        self.type_ = Some(type_.into());
        self
    }

    /// Sets the resource subtype.
    #[must_use]
    pub fn subtype(mut self, subtype: impl Into<String>) -> Self {
        self.subtype = Some(subtype.into());
        self
    }

    /// Sets the human-readable resource type title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the resource type entry.
    #[must_use]
    pub fn build(self) -> ResourceType {
        ResourceType {
            type_: self.type_,
            subtype: self.subtype,
            title: self.title,
            extra: self.extra,
        }
    }
}

/// Errors raised while building nested metadata entry values.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MetadataEntryBuildError {
    /// A required field was not provided to a metadata entry builder.
    #[error("missing required {entry} field: {field}")]
    MissingField {
        /// Name of the entry being built.
        entry: &'static str,
        /// Name of the missing field.
        field: &'static str,
    },
}

fn required_entry_field<T>(
    value: Option<T>,
    entry: &'static str,
    field: &'static str,
) -> Result<T, MetadataEntryBuildError> {
    value.ok_or(MetadataEntryBuildError::MissingField { entry, field })
}

/// Typed metadata payload used for deposition updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositMetadataUpdate {
    /// Record title.
    pub title: String,
    /// Zenodo upload type.
    pub upload_type: UploadType,
    /// Publication date, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<NaiveDate>,
    /// HTML description body sent as Zenodo's `description` field.
    #[serde(rename = "description")]
    pub description_html: String,
    /// Creator list.
    #[serde(default)]
    pub creators: Vec<Creator>,
    /// Access-right setting.
    pub access_right: AccessRight,
    /// License identifier for open-access deposits, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Free-form keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Related identifier entries.
    #[serde(default)]
    pub related_identifiers: Vec<RelatedIdentifier>,
    /// Free-form notes field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Version string for the deposit, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Target communities for the deposit.
    #[serde(default)]
    pub communities: Vec<CommunityRef>,
    /// Grant references for the deposit.
    #[serde(default)]
    pub grants: Vec<GrantRef>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// Errors raised while building [`DepositMetadataUpdate`] values.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DepositMetadataBuildError {
    /// A required metadata field was not provided to the builder.
    #[error("missing required deposit metadata field: {field}")]
    MissingField {
        /// Name of the missing field.
        field: &'static str,
    },
}

/// Builder for [`DepositMetadataUpdate`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DepositMetadataUpdateBuilder {
    title: Option<String>,
    upload_type: Option<UploadType>,
    publication_date: Option<NaiveDate>,
    description_html: Option<String>,
    creators: Vec<Creator>,
    access_right: Option<AccessRight>,
    license: Option<String>,
    keywords: Vec<String>,
    related_identifiers: Vec<RelatedIdentifier>,
    notes: Option<String>,
    version: Option<String>,
    communities: Vec<CommunityRef>,
    grants: Vec<GrantRef>,
    extra: BTreeMap<String, Value>,
}

impl DepositMetadataUpdate {
    /// Starts building a deposit metadata update payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{AccessRight, Creator, DepositMetadataUpdate, UploadType};
    ///
    /// let metadata = DepositMetadataUpdate::builder()
    ///     .title("Example dataset")
    ///     .upload_type(UploadType::Dataset)
    ///     .description_html("<p>Example upload</p>")
    ///     .creator(
    ///         Creator::builder()
    ///             .name("Doe, Jane")
    ///             .affiliation("Zenodo")
    ///             .build()?,
    ///     )
    ///     .access_right(AccessRight::Open)
    ///     .build()?;
    ///
    /// assert_eq!(metadata.title, "Example dataset");
    /// assert_eq!(metadata.creators.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn builder() -> DepositMetadataUpdateBuilder {
        DepositMetadataUpdateBuilder::default()
    }
}

impl DepositMetadataUpdateBuilder {
    /// Sets the record title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the Zenodo upload type.
    #[must_use]
    pub fn upload_type(mut self, upload_type: UploadType) -> Self {
        self.upload_type = Some(upload_type);
        self
    }

    /// Sets the publication date.
    #[must_use]
    pub fn publication_date(mut self, publication_date: NaiveDate) -> Self {
        self.publication_date = Some(publication_date);
        self
    }

    /// Sets the HTML description body.
    #[must_use]
    pub fn description_html(mut self, description_html: impl Into<String>) -> Self {
        self.description_html = Some(description_html.into());
        self
    }

    /// Replaces the full creator list.
    #[must_use]
    pub fn creators(mut self, creators: Vec<Creator>) -> Self {
        self.creators = creators;
        self
    }

    /// Adds one creator entry.
    #[must_use]
    pub fn creator(mut self, creator: Creator) -> Self {
        self.creators.push(creator);
        self
    }

    /// Adds one creator entry by name only.
    #[must_use]
    pub fn creator_named(mut self, name: impl Into<String>) -> Self {
        self.creators.push(Creator::named(name));
        self
    }

    /// Sets the access-right policy.
    #[must_use]
    pub fn access_right(mut self, access_right: AccessRight) -> Self {
        self.access_right = Some(access_right);
        self
    }

    /// Sets the license identifier.
    #[must_use]
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Replaces the keyword list.
    #[must_use]
    pub fn keywords(mut self, keywords: Vec<String>) -> Self {
        self.keywords = keywords;
        self
    }

    /// Adds one keyword.
    #[must_use]
    pub fn keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    /// Replaces the related identifier list.
    #[must_use]
    pub fn related_identifiers(mut self, related_identifiers: Vec<RelatedIdentifier>) -> Self {
        self.related_identifiers = related_identifiers;
        self
    }

    /// Adds one related identifier.
    #[must_use]
    pub fn related_identifier(mut self, related_identifier: RelatedIdentifier) -> Self {
        self.related_identifiers.push(related_identifier);
        self
    }

    /// Sets free-form notes.
    #[must_use]
    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Sets the version string.
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Replaces the community list.
    #[must_use]
    pub fn communities(mut self, communities: Vec<CommunityRef>) -> Self {
        self.communities = communities;
        self
    }

    /// Adds one community reference.
    #[must_use]
    pub fn community(mut self, community: CommunityRef) -> Self {
        self.communities.push(community);
        self
    }

    /// Adds one community reference by identifier only.
    #[must_use]
    pub fn community_identifier(mut self, identifier: impl Into<String>) -> Self {
        self.communities.push(CommunityRef::new(identifier));
        self
    }

    /// Replaces the grant list.
    #[must_use]
    pub fn grants(mut self, grants: Vec<GrantRef>) -> Self {
        self.grants = grants;
        self
    }

    /// Adds one grant reference.
    #[must_use]
    pub fn grant(mut self, grant: GrantRef) -> Self {
        self.grants.push(grant);
        self
    }

    /// Adds one grant reference by identifier only.
    #[must_use]
    pub fn grant_id(mut self, id: impl Into<String>) -> Self {
        self.grants.push(GrantRef::new(id));
        self
    }

    /// Replaces all extra untyped fields.
    #[must_use]
    pub fn extra(mut self, extra: BTreeMap<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    /// Adds one extra untyped field.
    #[must_use]
    pub fn extra_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Builds the metadata payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{AccessRight, Creator, DepositMetadataUpdate, UploadType};
    ///
    /// let metadata = DepositMetadataUpdate::builder()
    ///     .title("Dataset")
    ///     .upload_type(UploadType::Dataset)
    ///     .description_html("<p>Ready for upload</p>")
    ///     .creator(Creator::builder().name("Doe, Jane").build()?)
    ///     .access_right(AccessRight::Open)
    ///     .keyword("rust")
    ///     .build()?;
    ///
    /// assert_eq!(metadata.keywords, vec!["rust"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any required Zenodo metadata field is still
    /// missing.
    pub fn build(self) -> Result<DepositMetadataUpdate, DepositMetadataBuildError> {
        Ok(DepositMetadataUpdate {
            title: required(self.title, "title")?,
            upload_type: required(self.upload_type, "upload_type")?,
            publication_date: self.publication_date,
            description_html: required(self.description_html, "description_html")?,
            creators: required_non_empty(self.creators, "creators")?,
            access_right: required(self.access_right, "access_right")?,
            license: self.license,
            keywords: self.keywords,
            related_identifiers: self.related_identifiers,
            notes: self.notes,
            version: self.version,
            communities: self.communities,
            grants: self.grants,
            extra: self.extra,
        })
    }
}

fn required<T>(value: Option<T>, field: &'static str) -> Result<T, DepositMetadataBuildError> {
    value.ok_or(DepositMetadataBuildError::MissingField { field })
}

fn required_non_empty<T>(
    value: Vec<T>,
    field: &'static str,
) -> Result<Vec<T>, DepositMetadataBuildError> {
    if value.is_empty() {
        return Err(DepositMetadataBuildError::MissingField { field });
    }

    Ok(value)
}

/// Typed metadata returned on published records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordMetadata {
    /// Record title.
    pub title: String,
    /// Publication date, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<NaiveDate>,
    /// HTML description body, when present.
    #[serde(
        rename = "description",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub description_html: Option<String>,
    /// Creator list.
    #[serde(default)]
    pub creators: Vec<Creator>,
    /// Contributor list.
    #[serde(default)]
    pub contributors: Vec<Contributor>,
    /// Free-form keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Free-form reference strings.
    #[serde(default)]
    pub references: Vec<String>,
    /// Community references attached to the record.
    #[serde(default)]
    pub communities: Vec<CommunityRef>,
    /// Grant references attached to the record.
    #[serde(default)]
    pub grants: Vec<GrantRef>,
    /// Subject classifications attached to the record.
    #[serde(default)]
    pub subjects: Vec<Subject>,
    /// Additional identifier entries.
    #[serde(default)]
    pub identifiers: Vec<RecordIdentifier>,
    /// Alternate identifier entries.
    #[serde(default)]
    pub alternate_identifiers: Vec<RecordIdentifier>,
    /// Additional date entries.
    #[serde(default)]
    pub dates: Vec<RecordDate>,
    /// Related identifier entries.
    #[serde(default)]
    pub related_identifiers: Vec<RelatedIdentifier>,
    /// Resource type details, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<ResourceType>,
    /// Access-right details, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_right: Option<AccessRight>,
    /// Access conditions for restricted records, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_conditions: Option<String>,
    /// Embargo date for embargoed records, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embargo_date: Option<NaiveDate>,
    /// License details, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<LicenseRef>,
    /// Publisher string, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// Primary language string, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Size labels attached to the record.
    #[serde(default)]
    pub sizes: Vec<String>,
    /// Format labels attached to the record.
    #[serde(default)]
    pub formats: Vec<String>,
    /// Free-form notes field, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Version string, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Journal title, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_title: Option<String>,
    /// Journal volume, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_volume: Option<String>,
    /// Journal issue, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_issue: Option<String>,
    /// Journal pages, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_pages: Option<String>,
    /// Conference title, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_title: Option<String>,
    /// Conference acronym, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_acronym: Option<String>,
    /// Conference dates, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_dates: Option<String>,
    /// Conference place, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_place: Option<String>,
    /// Conference URL, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_url: Option<Url>,
    /// Conference session, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_session: Option<String>,
    /// Conference session part, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_session_part: Option<String>,
    /// Imprint publisher, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imprint_publisher: Option<String>,
    /// Imprint ISBN, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imprint_isbn: Option<String>,
    /// Imprint place, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imprint_place: Option<String>,
    /// Container title for book chapters, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partof_title: Option<String>,
    /// Container page range for book chapters, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partof_pages: Option<String>,
    /// Thesis supervisors, when present.
    #[serde(default)]
    pub thesis_supervisors: Vec<Creator>,
    /// Awarding university for a thesis, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thesis_university: Option<String>,
    /// Relation blocks reported on the record, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relations: Option<RecordRelations>,
    /// Additional untyped fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::{
        AccessRight, CommunityRef, Contributor, Creator, DepositMetadataBuildError,
        DepositMetadataUpdate, GrantRef, LicenseRef, MetadataEntryBuildError, RecordDate,
        RecordIdentifier, RecordMetadata, RecordRelations, RecordVersionRelation,
        RelatedIdentifier, ResourceType, Subject, UploadType,
    };
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn metadata_builders_start_with_empty_optional_collections() {
        let metadata = DepositMetadataUpdate::builder()
            .title("Title")
            .upload_type(UploadType::Dataset)
            .description_html("<p>Desc</p>")
            .creator(Creator::builder().name("Doe, Jane").build().unwrap())
            .access_right(AccessRight::Open)
            .build()
            .unwrap();

        assert!(metadata.keywords.is_empty());
        assert!(metadata.related_identifiers.is_empty());
        assert!(metadata.communities.is_empty());
        assert!(metadata.grants.is_empty());
    }

    #[test]
    fn metadata_builder_requires_missing_fields() {
        let error = DepositMetadataUpdate::builder()
            .title("Title")
            .upload_type(UploadType::Dataset)
            .access_right(AccessRight::Open)
            .build()
            .unwrap_err();

        assert_eq!(
            error,
            DepositMetadataBuildError::MissingField {
                field: "description_html",
            }
        );
    }

    #[test]
    fn metadata_builder_requires_at_least_one_creator() {
        let error = DepositMetadataUpdate::builder()
            .title("Title")
            .upload_type(UploadType::Dataset)
            .description_html("<p>Desc</p>")
            .access_right(AccessRight::Open)
            .build()
            .unwrap_err();

        assert_eq!(
            error,
            DepositMetadataBuildError::MissingField { field: "creators" }
        );
    }

    #[test]
    fn metadata_builder_supports_full_optional_surface() {
        let publication_date = NaiveDate::from_ymd_opt(2026, 4, 3).unwrap();
        let primary_creator = Creator::builder()
            .name("Doe, Jane")
            .affiliation("Zenodo")
            .orcid("0000-0000-0000-0001")
            .build()
            .unwrap();
        let secondary_creator = Creator::builder()
            .name("Doe, John")
            .gnd("123456")
            .build()
            .unwrap();
        let related_identifier = RelatedIdentifier::builder()
            .identifier("10.5281/zenodo.42")
            .relation("isSupplementTo")
            .scheme("doi")
            .resource_type("dataset")
            .build()
            .unwrap();
        let related_identifier_extra = RelatedIdentifier::builder()
            .identifier("https://example.org")
            .relation("references")
            .build()
            .unwrap();
        let community = CommunityRef::new("zenodo");
        let extra_community = CommunityRef::new("sandbox");
        let grant = GrantRef::new("grant-1");
        let extra_grant = GrantRef::new("grant-2");
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("language".into(), json!("en"));

        let metadata = DepositMetadataUpdate::builder()
            .title("Complete")
            .upload_type(UploadType::Dataset)
            .publication_date(publication_date)
            .description_html("<p>Complete</p>")
            .creators(vec![primary_creator.clone()])
            .creator(secondary_creator.clone())
            .access_right(AccessRight::Embargoed)
            .license("cc-by-4.0")
            .keywords(vec!["rust".into()])
            .keyword("zenodo")
            .related_identifiers(vec![related_identifier.clone()])
            .related_identifier(related_identifier_extra.clone())
            .notes("generated in tests")
            .version("1.2.3")
            .communities(vec![community.clone()])
            .community(extra_community.clone())
            .grants(vec![grant.clone()])
            .grant(extra_grant.clone())
            .extra(extra)
            .extra_field("source", json!("tarpaulin"))
            .build()
            .unwrap();

        assert_eq!(metadata.publication_date, Some(publication_date));
        assert_eq!(metadata.creators, vec![primary_creator, secondary_creator]);
        assert_eq!(metadata.access_right, AccessRight::Embargoed);
        assert_eq!(metadata.license.as_deref(), Some("cc-by-4.0"));
        assert_eq!(metadata.keywords, vec!["rust", "zenodo"]);
        assert_eq!(
            metadata.related_identifiers,
            vec![related_identifier, related_identifier_extra]
        );
        assert_eq!(metadata.notes.as_deref(), Some("generated in tests"));
        assert_eq!(metadata.version.as_deref(), Some("1.2.3"));
        assert_eq!(metadata.communities, vec![community, extra_community]);
        assert_eq!(metadata.grants, vec![grant, extra_grant]);
        assert_eq!(metadata.extra.get("language"), Some(&json!("en")));
        assert_eq!(metadata.extra.get("source"), Some(&json!("tarpaulin")));
    }

    #[test]
    fn nested_metadata_entry_builders_cover_common_construction_paths() {
        let creator = Creator::builder()
            .name("Doe, Jane")
            .affiliation("Zenodo")
            .orcid("0000-0000-0000-0001")
            .gnd("98765")
            .extra_field("department", json!("Research"))
            .build()
            .unwrap();
        let contributor = Contributor::builder()
            .name("Doe, John")
            .role("DataManager")
            .affiliation("Zenodo")
            .build()
            .unwrap();
        let subject = Subject::builder()
            .term("chemistry")
            .identifier("123")
            .scheme("custom")
            .build()
            .unwrap();
        let identifier = RecordIdentifier::builder()
            .identifier("10.5281/zenodo.42")
            .scheme("doi")
            .build()
            .unwrap();
        let date = RecordDate::builder()
            .date("2026-04-03")
            .date_type("Collected")
            .description("Sampling day")
            .build()
            .unwrap();
        let related = RelatedIdentifier::builder()
            .identifier("10.5281/zenodo.41")
            .relation("isVersionOf")
            .scheme("doi")
            .resource_type("dataset")
            .build()
            .unwrap();
        let community = CommunityRef::builder()
            .identifier("zenodo")
            .build()
            .unwrap();
        let grant = GrantRef::builder().id("grant-1").build().unwrap();
        let license = LicenseRef::builder()
            .id("cc-by-4.0")
            .title("CC BY 4.0")
            .build();
        let resource_type = ResourceType::builder()
            .type_("dataset")
            .subtype("image")
            .title("Dataset")
            .build();

        assert_eq!(creator.name, "Doe, Jane");
        assert_eq!(contributor.type_, "DataManager");
        assert_eq!(subject.term, "chemistry");
        assert_eq!(identifier.scheme.as_deref(), Some("doi"));
        assert_eq!(date.type_.as_deref(), Some("Collected"));
        assert_eq!(related.resource_type.as_deref(), Some("dataset"));
        assert_eq!(community.identifier, "zenodo");
        assert_eq!(grant.id, "grant-1");
        assert_eq!(license.id.as_deref(), Some("cc-by-4.0"));
        assert_eq!(resource_type.type_.as_deref(), Some("dataset"));
    }

    #[test]
    fn person_and_relation_metadata_builders_cover_full_surface() {
        let mut creator_extra = std::collections::BTreeMap::new();
        creator_extra.insert("department".into(), json!("Research"));
        let creator = Creator::builder()
            .name("Doe, Jane")
            .affiliation("Zenodo")
            .orcid("0000-0000-0000-0001")
            .gnd("98765")
            .extra(creator_extra.clone())
            .extra_field("lab", json!("Core"))
            .build()
            .unwrap();
        assert_eq!(Creator::named("Named Only").name, "Named Only");
        assert_eq!(creator.extra.get("department"), Some(&json!("Research")));
        assert_eq!(creator.extra.get("lab"), Some(&json!("Core")));

        let mut contributor_extra = std::collections::BTreeMap::new();
        contributor_extra.insert("x".into(), json!(1));
        let contributor = Contributor::builder()
            .name("Doe, John")
            .role("Editor")
            .affiliation("Zenodo")
            .orcid("0000-0000-0000-0002")
            .gnd("12345")
            .extra(contributor_extra)
            .extra_field("y", json!(2))
            .build()
            .unwrap();
        assert_eq!(Contributor::new("Ada", "Supervisor").type_, "Supervisor");
        assert_eq!(contributor.extra.get("x"), Some(&json!(1)));
        assert_eq!(contributor.extra.get("y"), Some(&json!(2)));

        let mut related_extra = std::collections::BTreeMap::new();
        related_extra.insert("strength".into(), json!("primary"));
        let related = RelatedIdentifier::builder()
            .identifier("10.5281/zenodo.1")
            .relation("isVersionOf")
            .scheme("doi")
            .resource_type("dataset")
            .extra(related_extra)
            .extra_field("note", json!("important"))
            .build()
            .unwrap();
        assert_eq!(
            RelatedIdentifier::new("10.5281/zenodo.2", "references").relation,
            "references"
        );
        assert_eq!(related.extra.get("strength"), Some(&json!("primary")));
        assert_eq!(related.extra.get("note"), Some(&json!("important")));
    }

    #[test]
    fn classification_metadata_builders_cover_full_surface() {
        let mut subject_extra = std::collections::BTreeMap::new();
        subject_extra.insert("priority".into(), json!("high"));
        let subject = Subject::builder()
            .term("chemistry")
            .identifier("123")
            .scheme("custom")
            .extra(subject_extra)
            .extra_field("group", json!("A"))
            .build()
            .unwrap();
        assert_eq!(Subject::new("physics").term, "physics");
        assert_eq!(subject.extra.get("priority"), Some(&json!("high")));
        assert_eq!(subject.extra.get("group"), Some(&json!("A")));

        let mut identifier_extra = std::collections::BTreeMap::new();
        identifier_extra.insert("kind".into(), json!("alternate"));
        let identifier = RecordIdentifier::builder()
            .identifier("10.5281/zenodo.42")
            .scheme("doi")
            .extra(identifier_extra)
            .extra_field("source", json!("Zenodo"))
            .build()
            .unwrap();
        assert_eq!(RecordIdentifier::new("ark:/123").identifier, "ark:/123");
        assert_eq!(identifier.extra.get("kind"), Some(&json!("alternate")));
        assert_eq!(identifier.extra.get("source"), Some(&json!("Zenodo")));

        let mut date_extra = std::collections::BTreeMap::new();
        date_extra.insert("certainty".into(), json!("exact"));
        let date = RecordDate::builder()
            .date("2026-04-03")
            .date_type("Collected")
            .description("Sampling day")
            .extra(date_extra)
            .extra_field("timezone", json!("UTC"))
            .build()
            .unwrap();
        assert_eq!(RecordDate::new("2026-04-04").date, "2026-04-04");
        assert_eq!(date.extra.get("certainty"), Some(&json!("exact")));
        assert_eq!(date.extra.get("timezone"), Some(&json!("UTC")));
    }

    #[test]
    fn reference_metadata_builders_cover_full_surface() {
        let mut community_extra = std::collections::BTreeMap::new();
        community_extra.insert("owner".into(), json!("zenodo"));
        let community = CommunityRef::builder()
            .identifier("zenodo")
            .extra(community_extra)
            .extra_field("scope", json!("public"))
            .build()
            .unwrap();
        assert_eq!(CommunityRef::new("sandbox").identifier, "sandbox");
        assert_eq!(community.extra.get("owner"), Some(&json!("zenodo")));
        assert_eq!(community.extra.get("scope"), Some(&json!("public")));

        let mut grant_extra = std::collections::BTreeMap::new();
        grant_extra.insert("agency".into(), json!("EU"));
        let grant = GrantRef::builder()
            .id("grant-1")
            .extra(grant_extra)
            .extra_field("call", json!("Horizon"))
            .build()
            .unwrap();
        assert_eq!(GrantRef::new("grant-2").id, "grant-2");
        assert_eq!(grant.extra.get("agency"), Some(&json!("EU")));
        assert_eq!(grant.extra.get("call"), Some(&json!("Horizon")));

        let mut license_extra = std::collections::BTreeMap::new();
        license_extra.insert("jurisdiction".into(), json!("EU"));
        let license = LicenseRef::builder()
            .id("cc-by-4.0")
            .title("CC BY 4.0")
            .extra(license_extra)
            .extra_field("version", json!("4.0"))
            .build();
        assert_eq!(LicenseRef::new("mit").id.as_deref(), Some("mit"));
        assert_eq!(license.extra.get("jurisdiction"), Some(&json!("EU")));
        assert_eq!(license.extra.get("version"), Some(&json!("4.0")));

        let mut resource_type_extra = std::collections::BTreeMap::new();
        resource_type_extra.insert("family".into(), json!("research-data"));
        let resource_type = ResourceType::builder()
            .type_("dataset")
            .subtype("image")
            .title("Dataset")
            .extra(resource_type_extra)
            .extra_field("display", json!("Data set"))
            .build();
        assert_eq!(
            ResourceType::new("software").type_.as_deref(),
            Some("software")
        );
        assert_eq!(
            resource_type.extra.get("family"),
            Some(&json!("research-data"))
        );
        assert_eq!(resource_type.extra.get("display"), Some(&json!("Data set")));
    }

    #[test]
    fn deposit_metadata_builder_shortcuts_are_exercised() {
        let metadata = DepositMetadataUpdate::builder()
            .title("Example")
            .upload_type(UploadType::Software)
            .description_html("<p>Example</p>")
            .creator_named("Doe, Jane")
            .access_right(AccessRight::Open)
            .community_identifier("zenodo")
            .grant_id("grant-1")
            .build()
            .unwrap();

        assert_eq!(metadata.creators[0].name, "Doe, Jane");
        assert_eq!(metadata.communities[0].identifier, "zenodo");
        assert_eq!(metadata.grants[0].id, "grant-1");
    }

    #[test]
    fn nested_metadata_entry_builders_require_mandatory_fields() {
        assert_eq!(
            Creator::builder().build().unwrap_err(),
            MetadataEntryBuildError::MissingField {
                entry: "Creator",
                field: "name",
            }
        );
        assert_eq!(
            Contributor::builder().name("Doe").build().unwrap_err(),
            MetadataEntryBuildError::MissingField {
                entry: "Contributor",
                field: "role",
            }
        );
        assert_eq!(
            RelatedIdentifier::builder()
                .identifier("10.5281/zenodo.1")
                .build()
                .unwrap_err(),
            MetadataEntryBuildError::MissingField {
                entry: "RelatedIdentifier",
                field: "relation",
            }
        );
    }

    #[test]
    fn enums_preserve_unknown_values() {
        let upload: UploadType = serde_json::from_str("\"posterish\"").unwrap();
        let access: AccessRight = serde_json::from_str("\"members-only\"").unwrap();

        assert_eq!(serde_json::to_string(&upload).unwrap(), "\"posterish\"");
        assert_eq!(serde_json::to_string(&access).unwrap(), "\"members-only\"");
    }

    #[test]
    fn known_enum_values_round_trip() {
        let upload = serde_json::to_string(&UploadType::Dataset).unwrap();
        let access = serde_json::to_string(&AccessRight::Open).unwrap();
        let metadata = RecordMetadata::default();

        assert_eq!(upload, "\"dataset\"");
        assert_eq!(access, "\"open\"");
        assert!(metadata.creators.is_empty());
    }

    #[test]
    fn published_record_metadata_deserializes_richer_typed_fields() {
        let metadata: RecordMetadata = serde_json::from_str(
            r#"{
                "title": "Rich record",
                "publication_date": "2026-04-03",
                "description": "<p>Rich</p>",
                "creators": [{ "name": "Doe, Jane" }],
                "contributors": [{ "name": "Doe, John", "type": "Editor" }],
                "keywords": ["rust", "zenodo"],
                "references": ["Doe J. Example."],
                "communities": [{ "identifier": "zenodo" }],
                "grants": [{ "id": "777541" }],
                "subjects": [{ "term": "Metadata", "scheme": "custom" }],
                "identifiers": [{ "identifier": "sha256:abc", "scheme": "sha256" }],
                "alternate_identifiers": [{ "identifier": "arXiv:1234.5678", "scheme": "arxiv" }],
                "dates": [{ "date": "2026-04-03", "type": "Collected" }],
                "related_identifiers": [{ "identifier": "10.5281/zenodo.42", "relation": "isSupplementTo" }],
                "resource_type": { "type": "dataset", "title": "Dataset" },
                "access_right": "open",
                "access_conditions": "By request",
                "license": { "id": "cc-by-4.0", "title": "Creative Commons Attribution 4.0 International" },
                "publisher": "Zenodo",
                "language": "eng",
                "sizes": ["1 file"],
                "formats": ["application/gzip"],
                "notes": "Some notes",
                "version": "1.2.3",
                "journal_title": "Journal of Rust",
                "journal_volume": "12",
                "journal_issue": "3",
                "journal_pages": "10-20",
                "conference_title": "RustConf",
                "conference_acronym": "RC",
                "conference_dates": "2026-04-03",
                "conference_place": "Rome, Italy",
                "conference_url": "https://example.org/conf",
                "conference_session": "A",
                "conference_session_part": "1",
                "imprint_publisher": "Example Press",
                "imprint_isbn": "978-1-234",
                "imprint_place": "Rome, Italy",
                "partof_title": "Collected Works",
                "partof_pages": "44-50",
                "thesis_supervisors": [{ "name": "Professor, Ada" }],
                "thesis_university": "Example University",
                "relations": {
                    "version": [{ "index": 2, "count": 4, "is_last": false }]
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            metadata.contributors,
            vec![Contributor {
                name: "Doe, John".into(),
                type_: "Editor".into(),
                affiliation: None,
                orcid: None,
                gnd: None,
                extra: std::collections::BTreeMap::default(),
            }]
        );
        assert_eq!(
            metadata.subjects,
            vec![Subject {
                term: "Metadata".into(),
                identifier: None,
                scheme: Some("custom".into()),
                extra: std::collections::BTreeMap::default(),
            }]
        );
        assert_eq!(metadata.access_right, Some(AccessRight::Open));
        assert_eq!(metadata.publisher.as_deref(), Some("Zenodo"));
        assert_eq!(
            metadata.relations,
            Some(RecordRelations {
                version: vec![RecordVersionRelation {
                    index: Some(2),
                    count: Some(4),
                    is_last: Some(false),
                    extra: std::collections::BTreeMap::default(),
                }],
                extra: std::collections::BTreeMap::default(),
            })
        );
        assert_eq!(
            metadata.thesis_university.as_deref(),
            Some("Example University")
        );
    }
}
