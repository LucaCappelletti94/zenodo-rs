//! Pagination types for Zenodo list and search responses.

use serde::{Deserialize, Serialize};
use url::Url;

/// Generic page of hits returned by Zenodo search endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Page<T> {
    /// Items on the current page.
    #[serde(default)]
    pub hits: Vec<T>,
    /// Reported total hit count, when Zenodo provides one.
    #[serde(default)]
    pub total: Option<u64>,
    /// URL for the next page, when present.
    #[serde(default)]
    pub next: Option<Url>,
    /// URL for the previous page, when present.
    #[serde(default)]
    pub prev: Option<Url>,
}
