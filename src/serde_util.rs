use std::fmt;

use serde::de::{IntoDeserializer, Visitor};
use serde::Deserializer;

/// Deserializes an integer field while tolerating integer-like float and
/// numeric string wire shapes from Zenodo.
pub(crate) fn deserialize_u64ish<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    struct U64ishVisitor;

    impl Visitor<'_> for U64ishVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a non-negative integer, integer-like float, or numeric string")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(value).map_err(E::custom)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if !value.is_finite() || value.fract() != 0.0 || value < 0.0 {
                return Err(E::custom("expected an integer-like numeric value"));
            }

            value.to_string().parse::<u64>().map_err(E::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            value.parse::<u64>().map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(U64ishVisitor)
}

/// Deserializes an optional integer field while tolerating integer-like float
/// and numeric string wire shapes from Zenodo.
pub(crate) fn deserialize_option_u64ish<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalU64ishVisitor;

    impl<'de> Visitor<'de> for OptionalU64ishVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(
                "an optional non-negative integer, integer-like float, or numeric string",
            )
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_u64ish(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalU64ishVisitor)
}

/// Deserializes a string field that Zenodo sometimes emits as either a string
/// or an integer.
pub(crate) fn deserialize_stringish<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringishVisitor;

    impl Visitor<'_> for StringishVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string, integer, or integer-like float")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
            Ok(value.to_string())
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(value)
                .map(|value| value.to_string())
                .map_err(E::custom)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            deserialize_u64ish(value.into_deserializer()).map(|value| value.to_string())
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
            Ok(value)
        }
    }

    deserializer.deserialize_any(StringishVisitor)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{deserialize_option_u64ish, deserialize_stringish, deserialize_u64ish};

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct U64Holder {
        #[serde(deserialize_with = "deserialize_u64ish")]
        value: u64,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct OptionalU64Holder {
        #[serde(default, deserialize_with = "deserialize_option_u64ish")]
        value: Option<u64>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct StringHolder {
        #[serde(deserialize_with = "deserialize_stringish")]
        value: String,
    }

    #[test]
    fn u64ish_accepts_integer_like_values() {
        assert_eq!(
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": 12 }))
                .unwrap()
                .value,
            12
        );
        assert_eq!(
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": 13.0 }))
                .unwrap()
                .value,
            13
        );
        assert_eq!(
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": "14" }))
                .unwrap()
                .value,
            14
        );
        assert_eq!(
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": 15_i64 }))
                .unwrap()
                .value,
            15
        );
    }

    #[test]
    fn u64ish_rejects_non_integral_or_negative_values() {
        let fractional =
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": 14.5 })).unwrap_err();
        let negative =
            serde_json::from_value::<U64Holder>(serde_json::json!({ "value": -1 })).unwrap_err();

        assert!(fractional.to_string().contains("integer-like"));
        assert!(!negative.to_string().is_empty());
    }

    #[test]
    fn optional_u64ish_handles_none_and_values() {
        assert_eq!(
            serde_json::from_value::<OptionalU64Holder>(serde_json::json!({})).unwrap(),
            OptionalU64Holder { value: None }
        );
        assert_eq!(
            serde_json::from_value::<OptionalU64Holder>(serde_json::json!({ "value": null }))
                .unwrap(),
            OptionalU64Holder { value: None }
        );
        assert_eq!(
            serde_json::from_value::<OptionalU64Holder>(serde_json::json!({ "value": "15" }))
                .unwrap(),
            OptionalU64Holder { value: Some(15) }
        );
        assert_eq!(
            serde_json::from_value::<OptionalU64Holder>(serde_json::json!({ "value": 16.0 }))
                .unwrap(),
            OptionalU64Holder { value: Some(16) }
        );
    }

    #[test]
    fn stringish_accepts_strings_and_integer_like_numbers() {
        assert_eq!(
            serde_json::from_value::<StringHolder>(serde_json::json!({ "value": "abc" })).unwrap(),
            StringHolder {
                value: "abc".into()
            }
        );
        assert_eq!(
            serde_json::from_value::<StringHolder>(serde_json::json!({ "value": 16.0 })).unwrap(),
            StringHolder { value: "16".into() }
        );
        assert_eq!(
            serde_json::from_value::<StringHolder>(serde_json::json!({ "value": 17 })).unwrap(),
            StringHolder { value: "17".into() }
        );
    }

    #[test]
    fn stringish_rejects_negative_or_fractional_numbers() {
        let negative =
            serde_json::from_value::<StringHolder>(serde_json::json!({ "value": -1 })).unwrap_err();
        let fractional =
            serde_json::from_value::<StringHolder>(serde_json::json!({ "value": 1.5 }))
                .unwrap_err();

        assert!(!negative.to_string().is_empty());
        assert!(fractional.to_string().contains("integer-like"));
    }
}
