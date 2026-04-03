//! Helpers for resolving absolute and relative Zenodo links.

use url::Url;

/// Resolves a Zenodo link relation against a base URL.
///
/// # Errors
///
/// Returns an error if `href` is neither a valid absolute URL nor a valid
/// relative reference for `base`.
pub fn resolve_link(base: &Url, href: &str) -> Result<Url, url::ParseError> {
    match Url::parse(href) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) => base.join(href),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_link;
    use url::Url;

    #[test]
    fn resolves_absolute_links_unchanged() {
        let base = Url::parse("https://zenodo.org/api/records/1").unwrap();
        let resolved = resolve_link(&base, "https://example.com/path").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/path");
    }

    #[test]
    fn resolves_relative_links_against_base() {
        let base = Url::parse("https://zenodo.org/api/records/1").unwrap();
        let resolved = resolve_link(&base, "../records/2").unwrap();
        assert_eq!(resolved.as_str(), "https://zenodo.org/api/records/2");
    }

    #[test]
    fn invalid_links_return_parse_errors() {
        let base = Url::parse("https://zenodo.org/api/records/1").unwrap();
        let error = resolve_link(&base, "http://[::1").unwrap_err();
        assert!(matches!(error, url::ParseError::InvalidIpv6Address));
    }
}
