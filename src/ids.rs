//! Small identifier newtypes used throughout the public API.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use url::Url;

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(
            /// Raw numeric identifier returned by Zenodo.
            pub u64
        );

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_u64(self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserialize_u64ish(deserializer).map(Self)
            }
        }
    };
}

fn deserialize_u64ish<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U64ish {
        Number(u64),
        String(String),
    }

    match U64ish::deserialize(deserializer)? {
        U64ish::Number(value) => Ok(value),
        U64ish::String(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

id_newtype!(
    /// Identifier for a deposition draft or published deposition.
    DepositionId
);
id_newtype!(
    /// Identifier for a public record version.
    RecordId
);
id_newtype!(
    /// Identifier shared across all versions in a record family.
    ConceptRecId
);

/// Identifier for a file attached to a deposition draft.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DepositionFileId(
    /// Raw file identifier returned by Zenodo.
    pub String,
);

impl fmt::Display for DepositionFileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for DepositionFileId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DepositionFileId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// DOI string wrapper used by record and deposition types.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Doi(
    /// Raw DOI value.
    pub String,
);

/// Errors raised while parsing or validating DOI selectors.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DoiError {
    /// The normalized DOI string was empty.
    #[error("DOI cannot be empty")]
    Empty,
    /// The DOI did not match the expected `10.<registrant>/<suffix>` shape.
    #[error("invalid DOI: {0}")]
    Invalid(String),
}

impl Doi {
    /// Creates a normalized DOI wrapper from a raw DOI-like input.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::Doi;
    ///
    /// let doi = Doi::new(" https://doi.org/10.5281/ZENODO.123 ")?;
    /// assert_eq!(doi.as_str(), "10.5281/zenodo.123");
    /// # Ok::<(), zenodo_rs::DoiError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the normalized value does not resemble a DOI.
    pub fn new(value: impl AsRef<str>) -> Result<Self, DoiError> {
        let normalized = normalize_doi(value.as_ref());
        validate_doi(&normalized)?;
        Ok(Self(normalized))
    }

    /// Returns the raw DOI string.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::Doi;
    ///
    /// let doi = Doi::new("doi:10.5281/ZENODO.456")?;
    /// assert_eq!(doi.as_str(), "10.5281/zenodo.456");
    /// # Ok::<(), zenodo_rs::DoiError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Doi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<String> for Doi {
    type Error = DoiError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Doi {
    type Error = DoiError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for Doi {
    type Err = DoiError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<'de> Deserialize<'de> for Doi {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Bucket upload URL returned by Zenodo for draft file uploads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BucketUrl(
    /// Raw bucket URL.
    pub Url,
);

impl From<Url> for BucketUrl {
    fn from(value: Url) -> Self {
        Self(value)
    }
}

impl AsRef<Url> for BucketUrl {
    fn as_ref(&self) -> &Url {
        &self.0
    }
}

fn normalize_doi(value: &str) -> String {
    let trimmed = value.trim();
    let without_prefix = trim_doi_prefix(trimmed);
    without_prefix.trim().to_ascii_lowercase()
}

fn trim_doi_prefix(value: &str) -> &str {
    const PREFIXES: [&str; 4] = [
        "doi:",
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
    ];

    for prefix in PREFIXES {
        if value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return &value[prefix.len()..];
        }
    }

    value
}

fn validate_doi(value: &str) -> Result<(), DoiError> {
    if value.is_empty() {
        return Err(DoiError::Empty);
    }

    let Some((registrant, suffix)) = value.split_once('/') else {
        return Err(DoiError::Invalid(value.to_owned()));
    };

    if registrant.len() <= 3 || !registrant.starts_with("10.") || suffix.is_empty() {
        return Err(DoiError::Invalid(value.to_owned()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BucketUrl, ConceptRecId, DepositionFileId, DepositionId, Doi, DoiError, RecordId};

    #[test]
    fn numeric_ids_deserialize_from_strings_and_numbers() {
        let deposition: DepositionId = serde_json::from_str("\"12\"").unwrap();
        let record: RecordId = serde_json::from_str("13").unwrap();
        let concept: ConceptRecId = serde_json::from_str("\"14\"").unwrap();

        assert_eq!(deposition.0, 12);
        assert_eq!(record.0, 13);
        assert_eq!(concept.0, 14);
    }

    #[test]
    fn doi_round_trips_through_display_and_parse() {
        let doi: Doi = "10.5281/zenodo.123".parse().unwrap();
        assert_eq!(doi.as_str(), "10.5281/zenodo.123");
        assert_eq!(doi.to_string(), "10.5281/zenodo.123");
    }

    #[test]
    fn doi_normalization_trims_prefixes_and_case() {
        assert_eq!(
            Doi::new("  HTTPS://DOI.ORG/10.5281/ZENODO.123  ")
                .unwrap()
                .as_str(),
            "10.5281/zenodo.123"
        );
        assert_eq!(
            Doi::new("doi:10.5281/ZENODO.456").unwrap().as_str(),
            "10.5281/zenodo.456"
        );
        assert_eq!(
            Doi::new("https://dx.doi.org/10.5281/ZENODO.789")
                .unwrap()
                .as_str(),
            "10.5281/zenodo.789"
        );
    }

    #[test]
    fn doi_deserialization_normalizes_values() {
        let doi: Doi = serde_json::from_str("\"HTTPS://DOI.ORG/10.5281/ZENODO.999\"").unwrap();
        assert_eq!(doi.as_str(), "10.5281/zenodo.999");
    }

    #[test]
    fn bucket_url_wraps_url() {
        let url = url::Url::parse("https://zenodo.org/api/files/abc").unwrap();
        let bucket = BucketUrl::from(url.clone());
        assert_eq!(bucket.as_ref(), &url);
    }

    #[test]
    fn string_wrappers_support_common_conversions() {
        let file_id = DepositionFileId::from("abc");
        let doi = Doi::try_from(String::from("10.5281/zenodo.456")).unwrap();
        let borrowed_doi = Doi::try_from("10.5281/zenodo.789").unwrap();
        let deposition = DepositionId::from(5_u64);
        let serialized = serde_json::to_string(&deposition).unwrap();
        let owned_file_id = DepositionFileId::from(String::from("xyz"));

        assert_eq!(file_id.to_string(), "abc");
        assert_eq!(doi.to_string(), "10.5281/zenodo.456");
        assert_eq!(borrowed_doi.to_string(), "10.5281/zenodo.789");
        assert_eq!(owned_file_id.to_string(), "xyz");
        assert_eq!(serialized, "5");
    }

    #[test]
    fn doi_validation_rejects_empty_or_invalid_values() {
        assert_eq!(Doi::new("  ").unwrap_err(), DoiError::Empty);
        assert!(matches!(
            Doi::new("zenodo.123").unwrap_err(),
            DoiError::Invalid(value) if value == "zenodo.123"
        ));
        assert!(matches!(
            Doi::new("10.5281/").unwrap_err(),
            DoiError::Invalid(value) if value == "10.5281/"
        ));
    }
}
