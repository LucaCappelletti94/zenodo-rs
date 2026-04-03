//! Published-record search, retrieval, and latest-version helpers.

use serde::Deserialize;
use url::Url;

use crate::client::ZenodoClient;
use crate::error::ZenodoError;
use crate::ids::{Doi, DoiError, RecordId};
use crate::model::{ArtifactInfo, Record, RecordFile};
use crate::pagination::Page;

/// Selector for a published record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordSelector {
    /// Select by Zenodo record ID.
    RecordId(
        /// Record identifier.
        RecordId,
    ),
    /// Select by DOI.
    Doi(
        /// DOI selector.
        Doi,
    ),
}

impl RecordSelector {
    /// Selects a record by record ID.
    #[must_use]
    pub fn record_id(id: RecordId) -> Self {
        Self::RecordId(id)
    }

    /// Selects a record by DOI string.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{RecordSelector, RecordId};
    ///
    /// assert_eq!(RecordSelector::record_id(RecordId(42)), RecordSelector::RecordId(RecordId(42)));
    /// assert!(matches!(
    ///     RecordSelector::doi("https://doi.org/10.5281/zenodo.42")?,
    ///     RecordSelector::Doi(_)
    /// ));
    /// # Ok::<(), zenodo_rs::DoiError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid.
    pub fn doi(value: impl AsRef<str>) -> Result<Self, DoiError> {
        Ok(Self::Doi(Doi::new(value)?))
    }
}

impl From<RecordId> for RecordSelector {
    fn from(value: RecordId) -> Self {
        Self::RecordId(value)
    }
}

impl From<Doi> for RecordSelector {
    fn from(value: Doi) -> Self {
        Self::Doi(value)
    }
}

impl From<&Doi> for RecordSelector {
    fn from(value: &Doi) -> Self {
        Self::Doi(value.clone())
    }
}

/// High-level selector for a downloadable artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactSelector {
    /// Select a named file from a record.
    FileByKey {
        /// Record or DOI selector.
        record: RecordSelector,
        /// Exact file key.
        key: String,
        /// Whether to resolve the latest version first.
        latest: bool,
    },
    /// Select the record archive.
    Archive {
        /// Record or DOI selector.
        record: RecordSelector,
        /// Whether to resolve the latest version first.
        latest: bool,
    },
}

impl ArtifactSelector {
    /// Selects a named file from a specific record or DOI.
    #[must_use]
    pub fn file(record: impl Into<RecordSelector>, key: impl Into<String>) -> Self {
        Self::FileByKey {
            record: record.into(),
            key: key.into(),
            latest: false,
        }
    }

    /// Selects a named file from the latest version of a record or DOI.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{ArtifactSelector, RecordId, RecordSelector};
    ///
    /// assert_eq!(
    ///     ArtifactSelector::latest_file(RecordId(42), "artifact.tar.gz"),
    ///     ArtifactSelector::FileByKey {
    ///         record: RecordSelector::RecordId(RecordId(42)),
    ///         key: "artifact.tar.gz".into(),
    ///         latest: true,
    ///     }
    /// );
    /// assert!(matches!(
    ///     ArtifactSelector::latest_file_by_doi("10.5281/zenodo.42", "artifact.tar.gz")?,
    ///     ArtifactSelector::FileByKey { latest: true, .. }
    /// ));
    /// # Ok::<(), zenodo_rs::DoiError>(())
    /// ```
    #[must_use]
    pub fn latest_file(record: impl Into<RecordSelector>, key: impl Into<String>) -> Self {
        Self::FileByKey {
            record: record.into(),
            key: key.into(),
            latest: true,
        }
    }

    /// Selects a named file from a DOI string.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid.
    pub fn file_by_doi(doi: impl AsRef<str>, key: impl Into<String>) -> Result<Self, DoiError> {
        Ok(Self::file(RecordSelector::doi(doi)?, key))
    }

    /// Selects a named file from the latest version resolved from a DOI string.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid.
    pub fn latest_file_by_doi(
        doi: impl AsRef<str>,
        key: impl Into<String>,
    ) -> Result<Self, DoiError> {
        Ok(Self::latest_file(RecordSelector::doi(doi)?, key))
    }

    /// Selects the archive for a specific record or DOI.
    #[must_use]
    pub fn archive(record: impl Into<RecordSelector>) -> Self {
        Self::Archive {
            record: record.into(),
            latest: false,
        }
    }

    /// Selects the archive for the latest version of a record or DOI.
    #[must_use]
    pub fn latest_archive(record: impl Into<RecordSelector>) -> Self {
        Self::Archive {
            record: record.into(),
            latest: true,
        }
    }

    /// Selects the archive for a DOI string.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid.
    pub fn archive_by_doi(doi: impl AsRef<str>) -> Result<Self, DoiError> {
        Ok(Self::archive(RecordSelector::doi(doi)?))
    }

    /// Selects the archive for the latest version resolved from a DOI string.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid.
    pub fn latest_archive_by_doi(doi: impl AsRef<str>) -> Result<Self, DoiError> {
        Ok(Self::latest_archive(RecordSelector::doi(doi)?))
    }
}

/// Typed query parameters for the records search API.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RecordQuery {
    /// Free-text query string.
    pub q: Option<String>,
    /// Record status filter.
    pub status: Option<RecordQueryStatus>,
    /// Sort order.
    pub sort: Option<RecordSort>,
    /// 1-based page number.
    pub page: Option<u32>,
    /// Page size.
    pub size: Option<u32>,
    /// Whether to include all versions in the search results.
    pub all_versions: bool,
    /// Community filters.
    pub communities: Vec<String>,
    /// Resource type filter.
    pub resource_type: Option<String>,
    /// Resource subtype filter.
    pub subtype: Option<String>,
    /// Extra raw query pairs for unsupported parameters.
    pub custom: Vec<(String, String)>,
}

impl RecordQuery {
    /// Starts building a typed record search query.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::RecordQuery;
    ///
    /// let query = RecordQuery::builder()
    ///     .query("doi:\"10.5281/zenodo.42\"")
    ///     .published()
    ///     .most_recent()
    ///     .size(10)
    ///     .all_versions()
    ///     .build();
    ///
    /// assert_eq!(query.q.as_deref(), Some("doi:\"10.5281/zenodo.42\""));
    /// assert!(query.all_versions);
    /// ```
    #[must_use]
    pub fn builder() -> RecordQueryBuilder {
        RecordQueryBuilder::default()
    }

    /// Serializes the query into Zenodo URL parameter pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{RecordQuery, RecordQueryStatus, RecordSort};
    ///
    /// let pairs = RecordQuery {
    ///     q: Some("doi:\"10.5281/zenodo.123\"".into()),
    ///     status: Some(RecordQueryStatus::Published),
    ///     sort: Some(RecordSort::MostRecent),
    ///     page: Some(2),
    ///     size: Some(25),
    ///     all_versions: true,
    ///     ..RecordQuery::default()
    /// }
    /// .into_pairs();
    ///
    /// assert!(pairs.contains(&("q".into(), "doi:\"10.5281/zenodo.123\"".into())));
    /// assert!(pairs.contains(&("status".into(), "published".into())));
    /// assert!(pairs.contains(&("sort".into(), "mostrecent".into())));
    /// assert!(pairs.contains(&("all_versions".into(), "true".into())));
    /// ```
    #[must_use]
    pub fn into_pairs(self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();

        if let Some(q) = self.q {
            pairs.push(("q".into(), q));
        }
        if let Some(status) = self.status {
            pairs.push(("status".into(), status.to_string()));
        }
        if let Some(sort) = self.sort {
            pairs.push(("sort".into(), sort.to_string()));
        }
        if let Some(page) = self.page {
            pairs.push(("page".into(), page.to_string()));
        }
        if let Some(size) = self.size {
            pairs.push(("size".into(), size.to_string()));
        }
        if self.all_versions {
            pairs.push(("all_versions".into(), "true".into()));
        }
        if !self.communities.is_empty() {
            pairs.push(("communities".into(), self.communities.join(",")));
        }
        if let Some(resource_type) = self.resource_type {
            pairs.push(("type".into(), resource_type));
        }
        if let Some(subtype) = self.subtype {
            pairs.push(("subtype".into(), subtype));
        }
        pairs.extend(self.custom);
        pairs
    }
}

/// Builder for [`RecordQuery`] values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RecordQueryBuilder {
    query: RecordQuery,
}

impl RecordQueryBuilder {
    /// Sets the free-text query string.
    #[must_use]
    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.query.q = Some(query.into());
        self
    }

    /// Sets the records API status filter.
    #[must_use]
    pub fn status(mut self, status: RecordQueryStatus) -> Self {
        self.query.status = Some(status);
        self
    }

    /// Filters to published records.
    #[must_use]
    pub fn published(mut self) -> Self {
        self.query.status = Some(RecordQueryStatus::Published);
        self
    }

    /// Filters to draft records.
    #[must_use]
    pub fn draft(mut self) -> Self {
        self.query.status = Some(RecordQueryStatus::Draft);
        self
    }

    /// Sets the records API sort order.
    #[must_use]
    pub fn sort(mut self, sort: RecordSort) -> Self {
        self.query.sort = Some(sort);
        self
    }

    /// Sorts by most recent first.
    #[must_use]
    pub fn most_recent(mut self) -> Self {
        self.query.sort = Some(RecordSort::MostRecent);
        self
    }

    /// Sets the 1-based page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.query.page = Some(page);
        self
    }

    /// Sets the page size.
    #[must_use]
    pub fn size(mut self, size: u32) -> Self {
        self.query.size = Some(size);
        self
    }

    /// Includes all versions in the search results.
    #[must_use]
    pub fn all_versions(mut self) -> Self {
        self.query.all_versions = true;
        self
    }

    /// Replaces the full community filter list.
    #[must_use]
    pub fn communities(mut self, communities: Vec<String>) -> Self {
        self.query.communities = communities;
        self
    }

    /// Adds one community filter.
    #[must_use]
    pub fn community(mut self, community: impl Into<String>) -> Self {
        self.query.communities.push(community.into());
        self
    }

    /// Sets the top-level resource type filter.
    #[must_use]
    pub fn resource_type(mut self, resource_type: impl Into<String>) -> Self {
        self.query.resource_type = Some(resource_type.into());
        self
    }

    /// Sets the resource subtype filter.
    #[must_use]
    pub fn subtype(mut self, subtype: impl Into<String>) -> Self {
        self.query.subtype = Some(subtype.into());
        self
    }

    /// Adds one unsupported raw query pair.
    #[must_use]
    pub fn custom(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.custom.push((key.into(), value.into()));
        self
    }

    /// Builds the query value.
    #[must_use]
    pub fn build(self) -> RecordQuery {
        self.query
    }
}

/// Filter values for the records `status` query parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordQueryStatus {
    /// Draft records.
    Draft,
    /// Published records.
    Published,
    /// Arbitrary server value not modeled directly by the crate.
    Custom(
        /// Raw server value.
        String,
    ),
}

impl std::fmt::Display for RecordQueryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Published => write!(f, "published"),
            Self::Custom(value) => value.fmt(f),
        }
    }
}

/// Sort values for the records `sort` query parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordSort {
    /// Relevance descending.
    BestMatch,
    /// Most recent first.
    MostRecent,
    /// Relevance ascending.
    AscBestMatch,
    /// Oldest first.
    AscMostRecent,
    /// Arbitrary server value not modeled directly by the crate.
    Custom(
        /// Raw server value.
        String,
    ),
}

impl std::fmt::Display for RecordSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BestMatch => write!(f, "bestmatch"),
            Self::MostRecent => write!(f, "mostrecent"),
            Self::AscBestMatch => write!(f, "-bestmatch"),
            Self::AscMostRecent => write!(f, "-mostrecent"),
            Self::Custom(value) => value.fmt(f),
        }
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct SearchEnvelope<T> {
    hits: SearchHits<T>,
    #[serde(default)]
    links: SearchLinks,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct SearchHits<T> {
    #[serde(default)]
    hits: Vec<T>,
    #[serde(default)]
    total: Option<SearchTotal>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SearchTotal {
    Number(u64),
    Object { value: u64 },
}

impl SearchTotal {
    fn into_u64(self) -> u64 {
        match self {
            Self::Number(value) | Self::Object { value } => value,
        }
    }
}

#[derive(Default, Deserialize)]
struct SearchLinks {
    #[serde(default)]
    next: Option<Url>,
    #[serde(default)]
    prev: Option<Url>,
}

impl<T> From<SearchEnvelope<T>> for Page<T> {
    fn from(value: SearchEnvelope<T>) -> Self {
        Self {
            hits: value.hits.hits,
            total: value.hits.total.map(SearchTotal::into_u64),
            next: value.links.next,
            prev: value.links.prev,
        }
    }
}

impl ZenodoClient {
    /// Searches published records using Zenodo's records API.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, RecordQuery, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let page = client
    ///         .search_records(
    ///             &RecordQuery::builder()
    ///                 .query("doi:\"10.5281/zenodo.123\"")
    ///                 .published()
    ///                 .most_recent()
    ///                 .size(10)
    ///                 .build(),
    ///         )
    ///         .await?;
    ///     let _ = page.hits;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo returns malformed
    /// search data.
    pub async fn search_records(&self, query: &RecordQuery) -> Result<Page<Record>, ZenodoError> {
        let pairs = query.clone().into_pairs();
        self.execute_json::<SearchEnvelope<Record>>(
            self.request(reqwest::Method::GET, "records")?.query(&pairs),
        )
        .await
        .map(Into::into)
    }

    /// Fetches a published record by record ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo returns a non-success
    /// response.
    pub async fn get_record(&self, id: RecordId) -> Result<Record, ZenodoError> {
        self.execute_json(self.request(reqwest::Method::GET, &format!("records/{id}"))?)
            .await
    }

    /// Resolves a DOI to a published record.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let record = client
    ///         .get_record_by_doi_str("https://doi.org/10.5281/zenodo.123")
    ///         .await?;
    ///     let _ = record.id;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the search fails or no record matches the DOI.
    pub async fn get_record_by_doi(&self, doi: &Doi) -> Result<Record, ZenodoError> {
        let mut page = self
            .search_records(
                &RecordQuery::builder()
                    .query(format!("doi:\"{doi}\" OR conceptdoi:\"{doi}\""))
                    .size(25)
                    .all_versions()
                    .build(),
            )
            .await?;

        loop {
            if let Some(record) = page
                .hits
                .into_iter()
                .find(|record| record_matches_doi(record, doi))
            {
                return Ok(record);
            }

            let Some(next) = page.next else {
                break;
            };
            page = self
                .execute_json::<SearchEnvelope<Record>>(
                    self.request_url(reqwest::Method::GET, next)?,
                )
                .await?
                .into();
        }

        Err(ZenodoError::UnsupportedSelector(format!(
            "no exact record found for DOI {doi}"
        )))
    }

    /// Parses a DOI string and resolves it to a published record.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid, if the search fails, or
    /// if no record matches the DOI.
    pub async fn get_record_by_doi_str(&self, doi: impl AsRef<str>) -> Result<Record, ZenodoError> {
        let doi = Doi::new(doi).map_err(|error| {
            ZenodoError::UnsupportedSelector(format!("invalid DOI selector: {error}"))
        })?;
        self.get_record_by_doi(&doi).await
    }

    /// Resolves a DOI and then follows the latest-version link when present.
    ///
    /// # Errors
    ///
    /// Returns an error if DOI resolution fails or the latest record cannot be
    /// fetched.
    pub async fn resolve_latest_by_doi(&self, doi: &Doi) -> Result<Record, ZenodoError> {
        let record = self.get_record_by_doi(doi).await?;
        self.resolve_latest_from_record(record).await
    }

    /// Parses a DOI string and resolves the latest version in that record family.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOI string is invalid, if DOI resolution fails,
    /// or if the latest record cannot be fetched.
    pub async fn resolve_latest_by_doi_str(
        &self,
        doi: impl AsRef<str>,
    ) -> Result<Record, ZenodoError> {
        let doi = Doi::new(doi).map_err(|error| {
            ZenodoError::UnsupportedSelector(format!("invalid DOI selector: {error}"))
        })?;
        self.resolve_latest_by_doi(&doi).await
    }

    /// Fetches the latest record version for a record family.
    ///
    /// # Errors
    ///
    /// Returns an error if record lookup fails or the latest record cannot be
    /// fetched.
    pub async fn get_latest_record(&self, id: RecordId) -> Result<Record, ZenodoError> {
        self.resolve_latest_version(id).await
    }

    /// Resolves the latest record version starting from a record ID.
    ///
    /// # Errors
    ///
    /// Returns an error if record lookup fails or the latest record cannot be
    /// fetched.
    pub async fn resolve_latest_version(&self, id: RecordId) -> Result<Record, ZenodoError> {
        let record = self.get_record(id).await?;
        self.resolve_latest_from_record(record).await
    }

    /// Lists the versions associated with a record family.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails or the versions query cannot
    /// be completed.
    pub async fn list_record_versions(&self, id: RecordId) -> Result<Page<Record>, ZenodoError> {
        let record = self.get_record(id).await?;
        if let Some(versions_url) = record.links.versions.clone() {
            return self
                .execute_json::<SearchEnvelope<Record>>(
                    self.request_url(reqwest::Method::GET, versions_url)?,
                )
                .await
                .map(Into::into);
        }

        if let Some(conceptrecid) = record.conceptrecid {
            return self
                .search_records(
                    &RecordQuery::builder()
                        .query(format!("conceptrecid:{}", conceptrecid.0))
                        .all_versions()
                        .most_recent()
                        .build(),
                )
                .await;
        }

        Ok(Page {
            hits: vec![record],
            total: Some(1),
            next: None,
            prev: None,
        })
    }

    /// Lists files attached to a specific record.
    ///
    /// # Errors
    ///
    /// Returns an error if the record lookup fails.
    pub async fn list_record_files(&self, id: RecordId) -> Result<Vec<RecordFile>, ZenodoError> {
        Ok(self.get_record(id).await?.files)
    }

    /// Returns a record together with its latest version and keyed files map.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::{Auth, RecordId, ZenodoClient};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = ZenodoClient::new(Auth::new("token"))?;
    ///     let info = client.get_artifact_info(RecordId(123)).await?;
    ///     let _ = info.files_by_key;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if record lookup or latest-version resolution fails.
    pub async fn get_artifact_info(&self, id: RecordId) -> Result<ArtifactInfo, ZenodoError> {
        let record = self.get_record(id).await?;
        let latest = self.resolve_latest_from_record(record.clone()).await?;
        let files_by_key = latest
            .files
            .iter()
            .cloned()
            .map(|file| (file.key.clone(), file))
            .collect();

        Ok(ArtifactInfo {
            record,
            latest,
            files_by_key,
        })
    }

    /// Resolves artifact information starting from a DOI.
    ///
    /// # Errors
    ///
    /// Returns an error if DOI resolution fails or latest-version resolution
    /// fails.
    pub async fn get_artifact_info_by_doi(&self, doi: &Doi) -> Result<ArtifactInfo, ZenodoError> {
        let record = self.get_record_by_doi(doi).await?;
        self.get_artifact_info(record.id).await
    }

    pub(crate) async fn resolve_record_selector(
        &self,
        selector: &RecordSelector,
    ) -> Result<Record, ZenodoError> {
        match selector {
            RecordSelector::RecordId(id) => self.get_record(*id).await,
            RecordSelector::Doi(doi) => self.get_record_by_doi(doi).await,
        }
    }

    pub(crate) async fn resolve_latest_from_record(
        &self,
        record: Record,
    ) -> Result<Record, ZenodoError> {
        match record.latest_url() {
            Some(latest_url) => self.get_record_by_url(latest_url).await,
            None => Ok(record),
        }
    }
}

fn record_matches_doi(record: &Record, doi: &Doi) -> bool {
    record.doi.as_ref() == Some(doi) || record.conceptdoi.as_ref() == Some(doi)
}

#[cfg(test)]
mod tests {
    use super::{
        record_matches_doi, ArtifactSelector, RecordQuery, RecordQueryStatus, RecordSelector,
        RecordSort,
    };
    use crate::{Doi, Record, RecordId};

    #[test]
    fn query_serialization_uses_zenodo_parameter_names() {
        let pairs = RecordQuery {
            q: Some("title:test".into()),
            page: Some(2),
            size: Some(50),
            all_versions: true,
            communities: vec!["alpha".into(), "beta".into()],
            resource_type: Some("dataset".into()),
            subtype: Some("image".into()),
            custom: vec![("foo".into(), "bar".into())],
            ..RecordQuery::default()
        }
        .into_pairs();

        assert!(pairs.contains(&("q".into(), "title:test".into())));
        assert!(pairs.contains(&("page".into(), "2".into())));
        assert!(pairs.contains(&("size".into(), "50".into())));
        assert!(pairs.contains(&("all_versions".into(), "true".into())));
        assert!(pairs.contains(&("communities".into(), "alpha,beta".into())));
        assert!(pairs.contains(&("type".into(), "dataset".into())));
        assert!(pairs.contains(&("subtype".into(), "image".into())));
        assert!(pairs.contains(&("foo".into(), "bar".into())));
    }

    #[test]
    fn query_builder_covers_common_search_configuration() {
        let pairs = RecordQuery::builder()
            .query("doi:\"10.5281/zenodo.1\"")
            .published()
            .most_recent()
            .page(2)
            .size(25)
            .all_versions()
            .community("zenodo")
            .resource_type("dataset")
            .subtype("image")
            .custom("foo", "bar")
            .build()
            .into_pairs();

        assert!(pairs.contains(&("q".into(), "doi:\"10.5281/zenodo.1\"".into())));
        assert!(pairs.contains(&("status".into(), "published".into())));
        assert!(pairs.contains(&("sort".into(), "mostrecent".into())));
        assert!(pairs.contains(&("page".into(), "2".into())));
        assert!(pairs.contains(&("size".into(), "25".into())));
        assert!(pairs.contains(&("all_versions".into(), "true".into())));
        assert!(pairs.contains(&("communities".into(), "zenodo".into())));
        assert!(pairs.contains(&("type".into(), "dataset".into())));
        assert!(pairs.contains(&("subtype".into(), "image".into())));
        assert!(pairs.contains(&("foo".into(), "bar".into())));
    }

    #[test]
    fn selector_and_display_helpers_cover_custom_variants() {
        let doi = Doi::new("10.5281/zenodo.1").unwrap();
        assert!(matches!(
            RecordSelector::from(RecordId(1)),
            RecordSelector::RecordId(_)
        ));
        assert!(matches!(
            RecordSelector::from(doi.clone()),
            RecordSelector::Doi(_)
        ));
        assert!(matches!(RecordSelector::from(&doi), RecordSelector::Doi(_)));
        assert_eq!(RecordSort::BestMatch.to_string(), "bestmatch");
        assert_eq!(RecordQueryStatus::Draft.to_string(), "draft");
        assert_eq!(RecordQueryStatus::Custom("mine".into()).to_string(), "mine");
        assert_eq!(RecordSort::AscBestMatch.to_string(), "-bestmatch");
        assert_eq!(RecordSort::AscMostRecent.to_string(), "-mostrecent");
        assert_eq!(RecordSort::Custom("rank".into()).to_string(), "rank");
        assert!(matches!(
            RecordSelector::record_id(RecordId(1)),
            RecordSelector::RecordId(_)
        ));
        assert!(matches!(
            RecordSelector::doi("10.5281/zenodo.1").unwrap(),
            RecordSelector::Doi(_)
        ));
        assert_eq!(
            ArtifactSelector::file(RecordId(1), "artifact.bin"),
            ArtifactSelector::FileByKey {
                record: RecordSelector::RecordId(RecordId(1)),
                key: "artifact.bin".into(),
                latest: false,
            }
        );
        assert_eq!(
            ArtifactSelector::latest_archive_by_doi("10.5281/zenodo.1").unwrap(),
            ArtifactSelector::Archive {
                record: RecordSelector::Doi(Doi::new("10.5281/zenodo.1").unwrap()),
                latest: true,
            }
        );
        assert_eq!(
            ArtifactSelector::latest_file_by_doi("10.5281/zenodo.1", "artifact.bin").unwrap(),
            ArtifactSelector::FileByKey {
                record: RecordSelector::Doi(Doi::new("10.5281/zenodo.1").unwrap()),
                key: "artifact.bin".into(),
                latest: true,
            }
        );
        assert_eq!(
            ArtifactSelector::file_by_doi("10.5281/zenodo.1", "artifact.bin").unwrap(),
            ArtifactSelector::FileByKey {
                record: RecordSelector::Doi(Doi::new("10.5281/zenodo.1").unwrap()),
                key: "artifact.bin".into(),
                latest: false,
            }
        );
        assert_eq!(
            ArtifactSelector::archive(RecordId(9)),
            ArtifactSelector::Archive {
                record: RecordSelector::RecordId(RecordId(9)),
                latest: false,
            }
        );
        assert_eq!(
            ArtifactSelector::latest_archive(RecordId(9)),
            ArtifactSelector::Archive {
                record: RecordSelector::RecordId(RecordId(9)),
                latest: true,
            }
        );
        assert_eq!(
            ArtifactSelector::archive_by_doi("10.5281/zenodo.1").unwrap(),
            ArtifactSelector::Archive {
                record: RecordSelector::Doi(Doi::new("10.5281/zenodo.1").unwrap()),
                latest: false,
            }
        );
    }

    #[test]
    fn query_builder_exercises_remaining_methods() {
        let query = RecordQuery::builder()
            .query("title:test")
            .status(RecordQueryStatus::Custom("custom".into()))
            .sort(RecordSort::AscMostRecent)
            .draft()
            .page(3)
            .size(15)
            .communities(vec!["alpha".into(), "beta".into()])
            .community("gamma")
            .resource_type("software")
            .subtype("source-code")
            .build();

        assert_eq!(query.q.as_deref(), Some("title:test"));
        assert_eq!(query.status, Some(RecordQueryStatus::Draft));
        assert_eq!(query.sort, Some(RecordSort::AscMostRecent));
        assert_eq!(query.page, Some(3));
        assert_eq!(query.size, Some(15));
        assert_eq!(query.communities, vec!["alpha", "beta", "gamma"]);
        assert_eq!(query.resource_type.as_deref(), Some("software"));
        assert_eq!(query.subtype.as_deref(), Some("source-code"));
    }

    #[test]
    fn doi_matching_accepts_record_and_concept_doi_only() {
        let doi = Doi::new("https://doi.org/10.5281/ZENODO.1").unwrap();
        let record: Record = serde_json::from_value(serde_json::json!({
            "id": 1,
            "recid": 1,
            "doi": "10.5281/zenodo.1",
            "conceptdoi": "10.5281/zenodo.2",
            "metadata": { "title": "artifact" },
            "files": [],
            "links": {}
        }))
        .unwrap();
        let concept_only: Record = serde_json::from_value(serde_json::json!({
            "id": 2,
            "recid": 2,
            "conceptdoi": "10.5281/zenodo.1",
            "metadata": { "title": "artifact" },
            "files": [],
            "links": {}
        }))
        .unwrap();
        let mismatch: Record = serde_json::from_value(serde_json::json!({
            "id": 3,
            "recid": 3,
            "doi": "10.5281/zenodo.999",
            "metadata": { "title": "artifact" },
            "files": [],
            "links": {}
        }))
        .unwrap();

        assert!(record_matches_doi(&record, &doi));
        assert!(record_matches_doi(&concept_only, &doi));
        assert!(!record_matches_doi(&mismatch, &doi));
    }
}
