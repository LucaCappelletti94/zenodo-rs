//! Endpoint selection for production, sandbox, or custom Zenodo deployments.

use url::Url;

/// Base API endpoint used by the client.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Endpoint {
    /// The public Zenodo production service.
    #[default]
    Production,
    /// The Zenodo sandbox service for integration tests and dry runs.
    Sandbox,
    /// A fully custom Zenodo deployment root or API base URL.
    Custom(
        /// Deployment root or base API URL, normalized to end in `/api/`.
        Url,
    ),
}

impl Endpoint {
    /// Returns the API base URL for this endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured URL cannot be parsed into a valid
    /// base URL.
    pub fn base_url(&self) -> Result<Url, url::ParseError> {
        match self {
            Self::Production => Url::parse("https://zenodo.org/api/"),
            Self::Sandbox => Url::parse("https://sandbox.zenodo.org/api/"),
            Self::Custom(url) => Ok(normalize_base_url(url.clone())),
        }
    }
}

fn normalize_base_url(mut url: Url) -> Url {
    let path = url.path().trim_end_matches('/');
    let normalized = if path.is_empty() {
        "/api/".to_owned()
    } else if path.ends_with("/api") {
        format!("{path}/")
    } else {
        format!("{path}/api/")
    };
    url.set_path(&normalized);
    url
}

#[cfg(test)]
mod tests {
    use super::Endpoint;
    use url::Url;

    #[test]
    fn uses_expected_production_and_sandbox_urls() {
        assert_eq!(
            Endpoint::Production.base_url().unwrap().as_str(),
            "https://zenodo.org/api/"
        );
        assert_eq!(
            Endpoint::Sandbox.base_url().unwrap().as_str(),
            "https://sandbox.zenodo.org/api/"
        );
    }

    #[test]
    fn preserves_custom_base_url() {
        let url = Url::parse("http://localhost:1234/api/").unwrap();
        assert_eq!(Endpoint::Custom(url.clone()).base_url().unwrap(), url);
    }

    #[test]
    fn normalizes_custom_base_url_without_trailing_slash() {
        let normalized = Endpoint::Custom(Url::parse("http://localhost:1234/api").unwrap())
            .base_url()
            .unwrap();
        assert_eq!(normalized.as_str(), "http://localhost:1234/api/");
    }

    #[test]
    fn normalizes_custom_base_url_with_empty_path() {
        let normalized = Endpoint::Custom(Url::parse("http://localhost:1234").unwrap())
            .base_url()
            .unwrap();
        assert_eq!(normalized.as_str(), "http://localhost:1234/api/");
    }

    #[test]
    fn normalizes_custom_deployment_root_to_api_base() {
        let normalized = Endpoint::Custom(Url::parse("http://localhost:1234/zenodo").unwrap())
            .base_url()
            .unwrap();
        assert_eq!(normalized.as_str(), "http://localhost:1234/zenodo/api/");
    }
}
