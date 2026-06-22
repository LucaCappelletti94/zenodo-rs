//! Low-level typed Zenodo client operations.
//!
//! Use this module when you want direct access to Zenodo's deposition and
//! record endpoints without the higher-level safety logic from [`crate::workflow`].
//!
//! The main entrypoints here are:
//!
//! - [`ZenodoClient`] for authenticated API access
//! - [`ZenodoClientBuilder`] for endpoint, timeout, and polling configuration
//! - [`Auth`] for token loading from strings or environment variables
//!
//! If you want the crate to decide between draft reuse and `newversion`, or to
//! run a full publish workflow, prefer [`crate::workflow`].

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Method, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::fs::File;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;
use url::Url;

use crate::endpoint::Endpoint;
use crate::error::ZenodoError;
use crate::ids::{BucketUrl, DepositionFileId, DepositionId};
use crate::metadata::DepositMetadataUpdate;
use crate::model::{BucketObject, Deposition, DepositionFile};
use crate::poll::PollOptions;
use crate::progress::TransferProgress;
use crate::retry::RetryOptions;

/// Bearer-token authentication for Zenodo API requests.
#[derive(Clone)]
pub struct Auth {
    /// API token used for bearer authentication.
    pub token: SecretString,
}

impl Auth {
    /// Standard environment variable for a production Zenodo API token.
    pub const TOKEN_ENV_VAR: &'static str = "ZENODO_TOKEN";

    /// Standard environment variable for a Zenodo sandbox API token.
    pub const SANDBOX_TOKEN_ENV_VAR: &'static str = "ZENODO_SANDBOX_TOKEN";

    /// Creates a new authentication wrapper from a raw token string.
    ///
    /// # Examples
    ///
    /// ```
    /// use secrecy::ExposeSecret;
    /// use zenodo_rs::Auth;
    ///
    /// let auth = Auth::new("secret-token");
    /// assert_eq!(auth.token.expose_secret(), "secret-token");
    /// ```
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: SecretString::from(token.into()),
        }
    }

    /// Reads a production Zenodo API token from [`Self::TOKEN_ENV_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variable is missing or invalid.
    pub fn from_env() -> Result<Self, ZenodoError> {
        Self::from_env_var(Self::TOKEN_ENV_VAR)
    }

    /// Reads a sandbox Zenodo API token from [`Self::SANDBOX_TOKEN_ENV_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variable is missing or invalid.
    pub fn from_sandbox_env() -> Result<Self, ZenodoError> {
        Self::from_env_var(Self::SANDBOX_TOKEN_ENV_VAR)
    }

    /// Reads a Zenodo API token from a custom environment variable.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use zenodo_rs::Auth;
    ///
    /// let auth = Auth::from_env_var("ZENODO_TOKEN")?;
    /// # let _ = auth;
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variable is missing or invalid.
    pub fn from_env_var(name: &str) -> Result<Self, ZenodoError> {
        let token = std::env::var(name).map_err(|source| ZenodoError::EnvVar {
            name: name.to_owned(),
            source,
        })?;
        Ok(Self::new(token))
    }
}

impl From<SecretString> for Auth {
    fn from(token: SecretString) -> Self {
        Self { token }
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Builder for configuring a [`ZenodoClient`].
#[derive(Clone, Debug)]
pub struct ZenodoClientBuilder {
    auth: Option<Auth>,
    endpoint: Endpoint,
    poll: PollOptions,
    retry: RetryOptions,
    user_agent: Option<String>,
    request_timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
}

impl ZenodoClientBuilder {
    /// Overrides the API endpoint used by the client.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{Auth, Endpoint, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .endpoint(Endpoint::Sandbox)
    ///     .build()?;
    ///
    /// assert!(matches!(client.endpoint(), Endpoint::Sandbox));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Switches the client to the Zenodo sandbox endpoint.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{Auth, Endpoint, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .sandbox()
    ///     .build()?;
    ///
    /// assert!(matches!(client.endpoint(), Endpoint::Sandbox));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn sandbox(mut self) -> Self {
        self.endpoint = Endpoint::Sandbox;
        self
    }

    /// Overrides the `User-Agent` header sent on each request.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{Auth, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .user_agent("my-zenodo-tool/0.1")
    ///     .build()?;
    /// # let _ = client;
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Sets the overall HTTP request timeout used by the underlying client.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use zenodo_rs::{Auth, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .request_timeout(Duration::from_secs(30))
    ///     .build()?;
    ///
    /// assert_eq!(client.request_timeout(), Some(Duration::from_secs(30)));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// Sets the TCP connect timeout used by the underlying client.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use zenodo_rs::{Auth, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .connect_timeout(Duration::from_secs(5))
    ///     .build()?;
    ///
    /// assert_eq!(client.connect_timeout(), Some(Duration::from_secs(5)));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Overrides the polling policy used by workflow helpers.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use zenodo_rs::{Auth, PollOptions, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .poll_options(PollOptions {
    ///         max_wait: Duration::from_secs(10),
    ///         initial_delay: Duration::from_millis(250),
    ///         max_delay: Duration::from_secs(1),
    ///     })
    ///     .build()?;
    ///
    /// assert_eq!(client.poll_options().max_wait, Duration::from_secs(10));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn poll_options(mut self, poll: PollOptions) -> Self {
        self.poll = poll;
        self
    }

    /// Overrides the retry policy used for transient failures.
    ///
    /// Retries apply to idempotent operations: record reads, downloads, and
    /// path-based uploads. They are on by default. Pass
    /// [`RetryOptions::disabled`] to attempt each operation exactly once.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{Auth, RetryOptions, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .retry_options(RetryOptions::disabled())
    ///     .build()?;
    ///
    /// assert_eq!(client.retry_options().max_retries, 0);
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn retry_options(mut self, retry: RetryOptions) -> Self {
        self.retry = retry;
        self
    }

    /// Builds a configured [`ZenodoClient`].
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying `reqwest` client cannot be built.
    pub fn build(self) -> Result<ZenodoClient, ZenodoError> {
        ensure_rustls_provider();

        let user_agent = self
            .user_agent
            .unwrap_or_else(|| format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));

        let mut inner = reqwest::Client::builder().user_agent(&user_agent);
        if let Some(timeout) = self.request_timeout {
            inner = inner.timeout(timeout);
        }
        if let Some(timeout) = self.connect_timeout {
            inner = inner.connect_timeout(timeout);
        }
        let inner = inner.build()?;

        Ok(ZenodoClient {
            inner,
            auth: self.auth,
            endpoint: self.endpoint,
            poll: self.poll,
            retry: self.retry,
            request_timeout: self.request_timeout,
            connect_timeout: self.connect_timeout,
        })
    }
}

#[cfg(feature = "rustls-ring-tls")]
pub(crate) fn ensure_rustls_provider() {
    static INSTALL: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALL.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(not(feature = "rustls-ring-tls"))]
pub(crate) fn ensure_rustls_provider() {}

/// Typed async client for the core Zenodo REST API.
#[derive(Clone, Debug)]
pub struct ZenodoClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) auth: Option<Auth>,
    pub(crate) endpoint: Endpoint,
    pub(crate) poll: PollOptions,
    pub(crate) retry: RetryOptions,
    pub(crate) request_timeout: Option<Duration>,
    pub(crate) connect_timeout: Option<Duration>,
}

impl ZenodoClient {
    /// Starts building a new client from authentication settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::{Auth, Endpoint, ZenodoClient};
    ///
    /// let client = ZenodoClient::builder(Auth::new("token"))
    ///     .sandbox()
    ///     .build()?;
    ///
    /// assert!(matches!(client.endpoint(), Endpoint::Sandbox));
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn builder(auth: Auth) -> ZenodoClientBuilder {
        Self::builder_with_auth(Some(auth))
    }

    /// Starts building a token-free client for Zenodo's public read surface.
    ///
    /// Anonymous clients reach record search, record and DOI lookup,
    /// latest-version resolution, and public downloads without credentials.
    /// Token-only operations (deposition, upload, publish) fail with
    /// [`ZenodoError::MissingAuth`].
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::ZenodoClient;
    ///
    /// let client = ZenodoClient::anonymous_builder().sandbox().build()?;
    /// assert!(client.is_anonymous());
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    #[must_use]
    pub fn anonymous_builder() -> ZenodoClientBuilder {
        Self::builder_with_auth(None)
    }

    fn builder_with_auth(auth: Option<Auth>) -> ZenodoClientBuilder {
        ZenodoClientBuilder {
            auth,
            endpoint: Endpoint::default(),
            poll: PollOptions::default(),
            retry: RetryOptions::default(),
            user_agent: None,
            request_timeout: None,
            connect_timeout: None,
        }
    }

    /// Builds a client with default endpoint and polling options.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialized.
    pub fn new(auth: Auth) -> Result<Self, ZenodoError> {
        Self::builder(auth).build()
    }

    /// Builds a client directly from a raw bearer token.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::ZenodoClient;
    ///
    /// let client = ZenodoClient::with_token("token")?;
    /// assert_eq!(client.endpoint().base_url()?.as_str(), "https://zenodo.org/api/");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialized.
    pub fn with_token(token: impl Into<String>) -> Result<Self, ZenodoError> {
        Self::new(Auth::new(token))
    }

    /// Builds a production client from [`Auth::TOKEN_ENV_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variable is missing or invalid, or
    /// if the underlying HTTP client cannot be initialized.
    pub fn from_env() -> Result<Self, ZenodoError> {
        Self::new(Auth::from_env()?)
    }

    /// Builds a sandbox client from [`Auth::SANDBOX_TOKEN_ENV_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variable is missing or invalid, or
    /// if the underlying HTTP client cannot be initialized.
    pub fn from_sandbox_env() -> Result<Self, ZenodoError> {
        Self::builder(Auth::from_sandbox_env()?).sandbox().build()
    }

    /// Builds an anonymous production client. See [`Self::anonymous_builder`].
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::ZenodoClient;
    ///
    /// let client = ZenodoClient::anonymous()?;
    /// assert!(client.is_anonymous());
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialized.
    pub fn anonymous() -> Result<Self, ZenodoError> {
        Self::anonymous_builder().build()
    }

    /// Builds an anonymous sandbox client. See [`Self::anonymous_builder`].
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialized.
    pub fn anonymous_sandbox() -> Result<Self, ZenodoError> {
        Self::anonymous_builder().sandbox().build()
    }

    /// Returns `true` when the client was built without an API token.
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.auth.is_none()
    }

    /// Returns the configured API endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Returns the configured polling behavior.
    #[must_use]
    pub fn poll_options(&self) -> &PollOptions {
        &self.poll
    }

    /// Returns the configured retry behavior for transient failures.
    #[must_use]
    pub fn retry_options(&self) -> &RetryOptions {
        &self.retry
    }

    /// Returns the configured overall HTTP request timeout.
    #[must_use]
    pub fn request_timeout(&self) -> Option<Duration> {
        self.request_timeout
    }

    /// Returns the configured TCP connect timeout.
    #[must_use]
    pub fn connect_timeout(&self) -> Option<Duration> {
        self.connect_timeout
    }

    pub(crate) fn request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, ZenodoError> {
        let url = self.endpoint.base_url()?.join(path)?;
        self.request_url(method, url)
    }

    pub(crate) fn request_url(
        &self,
        method: Method,
        url: Url,
    ) -> Result<RequestBuilder, ZenodoError> {
        if !self.is_trusted_url(&url)? {
            return Err(ZenodoError::InvalidState(format!(
                "refusing API request to different origin: {url}"
            )));
        }

        let mut request = self
            .inner
            .request(method, url)
            .header(ACCEPT, "application/json");
        if let Some(token) = self.bearer_token() {
            request = request.bearer_auth(token);
        }

        Ok(request)
    }

    /// Builds a request to a relative API path that requires authentication.
    pub(crate) fn authenticated_request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, ZenodoError> {
        self.require_auth()?;
        self.request(method, path)
    }

    /// Builds a request to an absolute API URL that requires authentication.
    pub(crate) fn authenticated_request_url(
        &self,
        method: Method,
        url: Url,
    ) -> Result<RequestBuilder, ZenodoError> {
        self.require_auth()?;
        self.request_url(method, url)
    }

    pub(crate) fn download_request_url(
        &self,
        method: Method,
        url: Url,
    ) -> Result<RequestBuilder, ZenodoError> {
        let trusted = self.is_trusted_url(&url)?;
        let mut request = self.inner.request(method, url);
        if trusted {
            if let Some(token) = self.bearer_token() {
                request = request.bearer_auth(token);
            }
        }

        Ok(request)
    }

    fn bearer_token(&self) -> Option<&str> {
        self.auth.as_ref().map(|auth| auth.token.expose_secret())
    }

    fn require_auth(&self) -> Result<(), ZenodoError> {
        if self.auth.is_some() {
            Ok(())
        } else {
            Err(ZenodoError::MissingAuth)
        }
    }

    fn is_trusted_url(&self, url: &Url) -> Result<bool, ZenodoError> {
        Ok(self.endpoint.base_url()?.origin() == url.origin())
    }

    pub(crate) async fn execute_json<T>(&self, request: RequestBuilder) -> Result<T, ZenodoError>
    where
        T: DeserializeOwned,
    {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(ZenodoError::from_response(response).await);
        }

        let bytes = response.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Sends a request built by `build`, decoding JSON, with transient-failure
    /// retries.
    ///
    /// `build` is invoked on each attempt so the single-use [`RequestBuilder`]
    /// is reconstructed before every send. Use this only for idempotent
    /// operations, since a retried request is sent more than once.
    pub(crate) async fn execute_json_with_retries<T, F>(&self, build: F) -> Result<T, ZenodoError>
    where
        T: DeserializeOwned,
        F: Fn() -> Result<RequestBuilder, ZenodoError>,
    {
        self.retry(|| async { self.execute_json(build()?).await })
            .await
    }

    pub(crate) async fn execute_json_or_else<T, F, Fut>(
        &self,
        request: RequestBuilder,
        on_empty: F,
    ) -> Result<T, ZenodoError>
    where
        T: DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, ZenodoError>>,
    {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(ZenodoError::from_response(response).await);
        }

        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return on_empty().await;
        }

        Ok(serde_json::from_slice(&bytes)?)
    }

    pub(crate) async fn execute_unit(&self, request: RequestBuilder) -> Result<(), ZenodoError> {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(ZenodoError::from_response(response).await);
        }

        Ok(())
    }

    pub(crate) async fn execute_response(
        &self,
        request: RequestBuilder,
    ) -> Result<reqwest::Response, ZenodoError> {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(ZenodoError::from_response(response).await);
        }

        Ok(response)
    }

    pub(crate) async fn get_deposition_by_url(&self, url: &Url) -> Result<Deposition, ZenodoError> {
        self.execute_json_with_retries(|| self.authenticated_request_url(Method::GET, url.clone()))
            .await
    }

    pub(crate) async fn get_record_by_url(
        &self,
        url: &Url,
    ) -> Result<crate::model::Record, ZenodoError> {
        self.execute_json_with_retries(|| self.request_url(Method::GET, url.clone()))
            .await
    }

    /// Creates a new empty deposition draft.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo returns a non-success
    /// response.
    pub async fn create_deposition(&self) -> Result<Deposition, ZenodoError> {
        self.execute_json(
            self.authenticated_request(Method::POST, "deposit/depositions")?
                .json(&serde_json::json!({})),
        )
        .await
    }

    /// Fetches a single deposition by deposition ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo returns a non-success
    /// response.
    pub async fn get_deposition(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        self.execute_json_with_retries(|| {
            self.authenticated_request(Method::GET, &format!("deposit/depositions/{id}"))
        })
        .await
    }

    /// Replaces the draft metadata for a deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the metadata.
    pub async fn update_metadata(
        &self,
        id: DepositionId,
        metadata: &DepositMetadataUpdate,
    ) -> Result<Deposition, ZenodoError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            metadata: &'a DepositMetadataUpdate,
        }

        self.execute_json(
            self.authenticated_request(Method::PUT, &format!("deposit/depositions/{id}"))?
                .json(&Payload { metadata }),
        )
        .await
    }

    /// Lists the files currently attached to a draft deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo returns a non-success
    /// response.
    pub async fn list_files(&self, id: DepositionId) -> Result<Vec<DepositionFile>, ZenodoError> {
        self.execute_json_with_retries(|| {
            self.authenticated_request(Method::GET, &format!("deposit/depositions/{id}/files"))
        })
        .await
    }

    /// Deletes a file from a draft deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the delete.
    pub async fn delete_file(
        &self,
        id: DepositionId,
        file_id: DepositionFileId,
    ) -> Result<(), ZenodoError> {
        self.execute_unit(self.authenticated_request(
            Method::DELETE,
            &format!("deposit/depositions/{id}/files/{file_id}"),
        )?)
        .await
    }

    /// Uploads a local file to a Zenodo bucket using a fixed content length.
    ///
    /// # Errors
    ///
    /// Returns an error if the local file cannot be read, if the upload URL
    /// cannot be formed, or if Zenodo rejects the upload.
    pub async fn upload_path(
        &self,
        bucket: &BucketUrl,
        filename: &str,
        path: &Path,
    ) -> Result<BucketObject, ZenodoError> {
        self.upload_path_with_progress(bucket, filename, path, ())
            .await
    }

    /// Uploads a local file to a Zenodo bucket while reporting progress.
    ///
    /// The supplied progress sink receives the fixed upload size before the
    /// transfer starts and one `advance` event per streamed chunk.
    ///
    /// # Errors
    ///
    /// Returns an error if the local file cannot be read, if the upload URL
    /// cannot be formed, or if Zenodo rejects the upload.
    pub async fn upload_path_with_progress<P>(
        &self,
        bucket: &BucketUrl,
        filename: &str,
        path: &Path,
        progress: P,
    ) -> Result<BucketObject, ZenodoError>
    where
        P: TransferProgress + 'static,
    {
        self.upload_path_with_content_type_and_progress(
            bucket,
            filename,
            path,
            mime::APPLICATION_OCTET_STREAM,
            progress,
        )
        .await
    }

    pub(crate) async fn upload_path_with_content_type_and_progress<P>(
        &self,
        bucket: &BucketUrl,
        filename: &str,
        path: &Path,
        content_type: mime::Mime,
        progress: P,
    ) -> Result<BucketObject, ZenodoError>
    where
        P: TransferProgress + 'static,
    {
        let upload_url = bucket_upload_url(bucket, filename)?;
        let progress = Arc::new(progress);

        // Each retry re-opens the file and rebuilds the single-use body. The
        // progress sink is begun per attempt, which resets a bar to zero, the
        // correct visual for a from-scratch retry.
        let uploaded = self
            .retry(|| async {
                let file = File::open(path).await?;
                let length = file.metadata().await?.len();
                progress.begin(Some(length));
                let body_progress = Arc::clone(&progress);
                let body = reqwest::Body::wrap_stream(ReaderStream::new(file).map(move |item| {
                    if let Ok(bytes) = &item {
                        body_progress.advance(bytes.len() as u64);
                    }
                    item
                }));

                self.execute_json(
                    self.authenticated_request_url(Method::PUT, upload_url.clone())?
                        .header(CONTENT_LENGTH, length)
                        .header(CONTENT_TYPE, content_type.as_ref())
                        .body(body),
                )
                .await
            })
            .await?;
        progress.finish();
        Ok(uploaded)
    }

    /// Uploads data from a blocking reader to a Zenodo bucket.
    ///
    /// The caller must provide the exact content length. Unlike path-based
    /// uploads, reader uploads are not retried on transient failures, because a
    /// consumed reader cannot be replayed.
    ///
    /// # Errors
    ///
    /// Returns an error if the upload URL cannot be formed, if the reader
    /// fails, or if Zenodo rejects the upload.
    pub async fn upload_reader<R>(
        &self,
        bucket: &BucketUrl,
        filename: &str,
        reader: R,
        content_length: u64,
        content_type: mime::Mime,
    ) -> Result<BucketObject, ZenodoError>
    where
        R: Read + Send + 'static,
    {
        self.upload_reader_with_progress(bucket, filename, reader, content_length, content_type, ())
            .await
    }

    /// Uploads data from a blocking reader to a Zenodo bucket while reporting progress.
    ///
    /// The caller must provide the exact content length. The supplied progress
    /// sink receives the fixed upload size before the transfer starts and one
    /// `advance` event per chunk that is accepted by the request body stream.
    /// Reader uploads are not retried on transient failures, because a consumed
    /// reader cannot be replayed.
    ///
    /// # Errors
    ///
    /// Returns an error if the upload URL cannot be formed, if the reader
    /// fails, or if Zenodo rejects the upload.
    pub async fn upload_reader_with_progress<R, P>(
        &self,
        bucket: &BucketUrl,
        filename: &str,
        reader: R,
        content_length: u64,
        content_type: mime::Mime,
        progress: P,
    ) -> Result<BucketObject, ZenodoError>
    where
        R: Read + Send + 'static,
        P: TransferProgress + 'static,
    {
        let progress = Arc::new(progress);
        progress.begin(Some(content_length));
        let body = sized_body_from_reader(reader, content_length, Arc::clone(&progress));

        let uploaded = self
            .execute_json(
                self.authenticated_request_url(Method::PUT, bucket_upload_url(bucket, filename)?)?
                    .header(CONTENT_LENGTH, content_length)
                    .header(CONTENT_TYPE, content_type.as_ref())
                    .body(body),
            )
            .await?;
        progress.finish();
        Ok(uploaded)
    }

    /// Publishes a draft deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the publish
    /// action.
    pub async fn publish(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        self.execute_json_or_else(
            self.authenticated_request(
                Method::POST,
                &format!("deposit/depositions/{id}/actions/publish"),
            )?,
            || async move { self.get_deposition(id).await },
        )
        .await
    }

    /// Enters edit mode for a published deposition draft.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the edit
    /// action.
    pub async fn edit(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        self.execute_json_or_else(
            self.authenticated_request(
                Method::POST,
                &format!("deposit/depositions/{id}/actions/edit"),
            )?,
            || async move { self.get_deposition(id).await },
        )
        .await
    }

    /// Discards the current draft changes for a deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the discard
    /// action.
    pub async fn discard(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        self.execute_json_or_else(
            self.authenticated_request(
                Method::POST,
                &format!("deposit/depositions/{id}/actions/discard"),
            )?,
            || async move { self.get_deposition(id).await },
        )
        .await
    }

    /// Creates a new draft version from a published deposition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Zenodo rejects the versioning
    /// action.
    pub async fn new_version(&self, id: DepositionId) -> Result<Deposition, ZenodoError> {
        self.execute_json_or_else(
            self.authenticated_request(
                Method::POST,
                &format!("deposit/depositions/{id}/actions/newversion"),
            )?,
            || async move { self.get_deposition(id).await },
        )
        .await
    }
}

fn bucket_upload_url(bucket: &BucketUrl, filename: &str) -> Result<Url, ZenodoError> {
    let mut url = bucket.0.clone();
    let mut segments = url.path_segments_mut().map_err(|()| {
        ZenodoError::InvalidState("bucket URL cannot accept filename segments".to_owned())
    })?;
    segments.pop_if_empty();
    segments.push(filename);
    drop(segments);
    Ok(url)
}

fn sized_body_from_reader<R, P>(reader: R, content_length: u64, progress: Arc<P>) -> reqwest::Body
where
    R: Read + Send + 'static,
    P: TransferProgress + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);

    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut remaining = content_length;

        while remaining > 0 {
            let mut buf = vec![0_u8; remaining.min(64 * 1024) as usize];
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.blocking_send(Err(std::io::Error::new(
                        ErrorKind::UnexpectedEof,
                        "reader ended before declared content_length bytes were produced",
                    )));
                    return;
                }
                Ok(read) => {
                    buf.truncate(read);
                    remaining -= read as u64;
                    if tx.blocking_send(Ok(Bytes::from(buf))).is_err() {
                        return;
                    }
                    progress.advance(read as u64);
                }
                Err(error) => {
                    let _ = tx.blocking_send(Err(error));
                    return;
                }
            }
        }
    });

    reqwest::Body::wrap_stream(ReceiverStream::new(rx))
}

#[cfg(test)]
mod tests {
    use std::env::VarError;
    use std::io::{self, Cursor, Read};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::{bucket_upload_url, Auth, ZenodoClient};
    use crate::ids::BucketUrl;
    use crate::{Endpoint, PollOptions, RecordId, ZenodoError};
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};
    use http_body_util::BodyExt;
    use reqwest::Method;
    use secrecy::{ExposeSecret, SecretString};
    use serde_json::json;
    use tokio::net::TcpListener;
    use url::Url;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: Option<&str>) -> Self {
            let previous = std::env::var(name).ok();
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
            Self { name, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn bucket_upload_preserves_path_and_encodes_filename() {
        let bucket = BucketUrl(Url::parse("https://zenodo.org/api/files/bucket-id").unwrap());
        let url = bucket_upload_url(&bucket, "artifact v1.tar.gz").unwrap();
        assert_eq!(
            url.as_str(),
            "https://zenodo.org/api/files/bucket-id/artifact%20v1.tar.gz"
        );
    }

    #[test]
    fn auth_debug_redacts_tokens_and_builders_preserve_configuration() {
        let auth = Auth::from(SecretString::from("secret"));
        assert!(format!("{auth:?}").contains("<redacted>"));
        assert_eq!(auth.token.expose_secret(), "secret");

        let poll = PollOptions {
            max_wait: Duration::from_secs(3),
            initial_delay: Duration::from_millis(2),
            max_delay: Duration::from_millis(4),
        };
        let endpoint = Endpoint::Custom(Url::parse("http://localhost:9999/api/").unwrap());
        let client = ZenodoClient::builder(Auth::new("token"))
            .endpoint(endpoint.clone())
            .user_agent("custom-agent/1.0")
            .request_timeout(Duration::from_secs(7))
            .connect_timeout(Duration::from_secs(2))
            .poll_options(poll.clone())
            .build()
            .unwrap();

        assert_eq!(client.endpoint(), &endpoint);
        assert_eq!(client.poll_options(), &poll);
        assert_eq!(client.request_timeout(), Some(Duration::from_secs(7)));
        assert_eq!(client.connect_timeout(), Some(Duration::from_secs(2)));
        assert!(matches!(
            ZenodoClient::builder(Auth::new("token"))
                .sandbox()
                .build()
                .unwrap()
                .endpoint(),
            Endpoint::Sandbox
        ));
        assert!(ZenodoClient::new(Auth::new("token")).is_ok());
        assert!(ZenodoClient::with_token("token").is_ok());
    }

    #[test]
    fn env_helpers_read_expected_token_variables() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _prod_guard = EnvVarGuard::set(Auth::TOKEN_ENV_VAR, Some("prod-token"));
        let _sandbox_guard = EnvVarGuard::set(Auth::SANDBOX_TOKEN_ENV_VAR, Some("sandbox-token"));
        let _custom_guard = EnvVarGuard::set("CUSTOM_ZENODO_TOKEN", Some("custom-token"));

        assert_eq!(
            Auth::from_env().unwrap().token.expose_secret(),
            "prod-token"
        );
        assert_eq!(
            Auth::from_sandbox_env().unwrap().token.expose_secret(),
            "sandbox-token"
        );
        assert_eq!(
            Auth::from_env_var("CUSTOM_ZENODO_TOKEN")
                .unwrap()
                .token
                .expose_secret(),
            "custom-token"
        );
        assert!(matches!(
            ZenodoClient::from_sandbox_env().unwrap().endpoint(),
            Endpoint::Sandbox
        ));
        assert!(matches!(
            ZenodoClient::from_env().unwrap().endpoint(),
            Endpoint::Production
        ));
    }

    #[test]
    fn env_helpers_report_missing_variables() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _prod_guard = EnvVarGuard::set(Auth::TOKEN_ENV_VAR, None);
        let _sandbox_guard = EnvVarGuard::set(Auth::SANDBOX_TOKEN_ENV_VAR, None);

        match Auth::from_env().unwrap_err() {
            ZenodoError::EnvVar { name, source } => {
                assert_eq!(name, Auth::TOKEN_ENV_VAR);
                assert!(matches!(source, VarError::NotPresent));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        match ZenodoClient::from_sandbox_env().unwrap_err() {
            ZenodoError::EnvVar { name, source } => {
                assert_eq!(name, Auth::SANDBOX_TOKEN_ENV_VAR);
                assert!(matches!(source, VarError::NotPresent));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bucket_upload_rejects_urls_without_path_segments() {
        let bucket = BucketUrl(Url::parse("mailto:test@example.com").unwrap());
        let error = bucket_upload_url(&bucket, "artifact.bin").unwrap_err();
        assert!(matches!(error, crate::ZenodoError::InvalidState(_)));
    }

    #[test]
    fn request_url_rejects_cross_origin_api_requests() {
        let client = ZenodoClient::builder(Auth::new("token"))
            .endpoint(Endpoint::Custom(
                Url::parse("http://localhost:1234/api/").unwrap(),
            ))
            .build()
            .unwrap();

        let error = client
            .request_url(
                Method::GET,
                Url::parse("http://example.com/api/records/1").unwrap(),
            )
            .unwrap_err();
        assert!(matches!(error, ZenodoError::InvalidState(_)));
    }

    #[tokio::test]
    async fn sized_body_from_reader_reports_short_reads() {
        let body =
            super::sized_body_from_reader(Cursor::new(b"ab".to_vec()), 5, std::sync::Arc::new(()));
        let error = body.collect().await.unwrap_err();
        assert!(error.is_body());
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }
    }

    #[tokio::test]
    async fn sized_body_from_reader_reports_reader_errors() {
        let body = super::sized_body_from_reader(BrokenReader, 5, std::sync::Arc::new(()));
        let error = body.collect().await.unwrap_err();
        assert!(error.is_body());
    }

    #[tokio::test]
    async fn sized_body_from_reader_tolerates_dropped_receiver() {
        let body =
            super::sized_body_from_reader(Cursor::new(b"abc".to_vec()), 3, std::sync::Arc::new(()));
        drop(body);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[test]
    fn anonymous_clients_omit_bearer_auth_on_requests() {
        use reqwest::header::AUTHORIZATION;

        let client = ZenodoClient::anonymous().unwrap();
        assert!(client.is_anonymous());
        assert!(matches!(client.endpoint(), Endpoint::Production));

        let request = client
            .request(Method::GET, "records/1")
            .unwrap()
            .build()
            .unwrap();
        assert!(!request.headers().contains_key(AUTHORIZATION));

        let download = client
            .download_request_url(
                Method::GET,
                Url::parse("https://zenodo.org/api/files/bucket/file.bin").unwrap(),
            )
            .unwrap()
            .build()
            .unwrap();
        assert!(!download.headers().contains_key(AUTHORIZATION));
    }

    #[test]
    fn authenticated_clients_attach_bearer_auth_on_requests() {
        use reqwest::header::AUTHORIZATION;

        let client = ZenodoClient::with_token("secret-token").unwrap();
        assert!(!client.is_anonymous());

        let request = client
            .request(Method::GET, "records/1")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer secret-token"
        );

        let download = client
            .download_request_url(
                Method::GET,
                Url::parse("https://zenodo.org/api/files/bucket/file.bin").unwrap(),
            )
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            download.headers().get(AUTHORIZATION).unwrap(),
            "Bearer secret-token"
        );
    }

    #[test]
    fn anonymous_builder_preserves_endpoint_selection() {
        let client = ZenodoClient::anonymous_builder().sandbox().build().unwrap();
        assert!(client.is_anonymous());
        assert!(matches!(client.endpoint(), Endpoint::Sandbox));

        let client = ZenodoClient::anonymous_sandbox().unwrap();
        assert!(client.is_anonymous());
        assert!(matches!(client.endpoint(), Endpoint::Sandbox));
    }

    #[tokio::test]
    async fn authenticated_operations_reject_anonymous_clients() {
        let client = ZenodoClient::anonymous().unwrap();

        assert!(matches!(
            client.create_deposition().await.unwrap_err(),
            ZenodoError::MissingAuth
        ));
        assert!(matches!(
            client
                .get_deposition(crate::DepositionId(1))
                .await
                .unwrap_err(),
            ZenodoError::MissingAuth
        ));
        let upload = tempfile::NamedTempFile::new().unwrap();
        assert!(matches!(
            client
                .upload_path(
                    &BucketUrl(Url::parse("https://zenodo.org/api/files/bucket").unwrap()),
                    "artifact.bin",
                    upload.path(),
                )
                .await
                .unwrap_err(),
            ZenodoError::MissingAuth
        ));
    }

    #[tokio::test]
    async fn request_timeout_is_enforced_for_http_calls() {
        #[derive(Clone)]
        struct DelayState {
            delay: Duration,
        }

        async fn delayed_record(
            State(state): State<Arc<DelayState>>,
        ) -> (StatusCode, Json<serde_json::Value>) {
            tokio::time::sleep(state.delay).await;
            (
                StatusCode::OK,
                Json(json!({
                    "id": 1,
                    "recid": 1,
                    "metadata": { "title": "slow" },
                    "files": [],
                    "links": {}
                })),
            )
        }

        let state = Arc::new(DelayState {
            delay: Duration::from_millis(50),
        });
        let app = Router::new()
            .route("/api/records/1", get(delayed_record))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = ZenodoClient::builder(Auth::new("token"))
            .endpoint(Endpoint::Custom(
                Url::parse(&format!("http://{addr}/api/")).unwrap(),
            ))
            .request_timeout(Duration::from_millis(10))
            .build()
            .unwrap();

        let error = client.get_record(RecordId(1)).await.unwrap_err();
        match error {
            ZenodoError::Transport(source) => assert!(source.is_timeout()),
            other => panic!("unexpected error: {other:?}"),
        }

        server.abort();
    }
}
