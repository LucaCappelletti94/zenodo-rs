#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use zenodo_rs::{Auth, Endpoint, PollOptions, ZenodoClient};

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: Method,
    pub path: String,
    pub query: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct QueuedResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl QueuedResponse {
    pub fn json(status: StatusCode, body: Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&body).expect("json response serialization"),
        }
    }

    pub fn text(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: body.into().into_bytes(),
        }
    }

    pub fn bytes(status: StatusCode, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
}

#[derive(Default)]
struct MockState {
    responses: Mutex<HashMap<(Method, String), VecDeque<QueuedResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

pub struct MockZenodoServer {
    pub base_url: Url,
    state: Arc<MockState>,
    handle: JoinHandle<()>,
}

impl MockZenodoServer {
    pub async fn start() -> Self {
        let state = Arc::new(MockState::default());
        let app = Router::new()
            .fallback(any(handle_request))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock server");
        });

        Self {
            base_url: Url::parse(&format!("http://{addr}/api/")).expect("mock api base url"),
            state,
            handle,
        }
    }

    pub fn client(&self) -> ZenodoClient {
        ZenodoClient::builder(Auth::new("test-token"))
            .endpoint(Endpoint::Custom(self.base_url.clone()))
            .user_agent("zenodo-rs-tests/0.1")
            .poll_options(PollOptions {
                max_wait: Duration::from_millis(250),
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
            })
            .build()
            .expect("build test client")
    }

    pub fn url(&self, path: &str) -> String {
        self.base_url
            .join(path.trim_start_matches('/'))
            .expect("join mock url")
            .to_string()
    }

    pub fn enqueue(&self, method: Method, path: &str, response: QueuedResponse) {
        let mut responses = self.state.responses.lock().expect("lock responses");
        responses
            .entry((method, path.to_owned()))
            .or_default()
            .push_back(response);
    }

    pub fn enqueue_json(&self, method: Method, path: &str, status: StatusCode, body: Value) {
        self.enqueue(method, path, QueuedResponse::json(status, body));
    }

    pub fn enqueue_text(
        &self,
        method: Method,
        path: &str,
        status: StatusCode,
        body: impl Into<String>,
    ) {
        self.enqueue(method, path, QueuedResponse::text(status, body));
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.state.requests.lock().expect("lock requests").clone()
    }
}

impl Drop for MockZenodoServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_request(
    State(state): State<Arc<MockState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX).await.expect("read request body");

    let captured = CapturedRequest {
        method: parts.method.clone(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(str::to_owned),
        headers: normalize_headers(&parts.headers),
        body: body.to_vec(),
    };
    state
        .requests
        .lock()
        .expect("lock requests")
        .push(captured.clone());

    let response = state
        .responses
        .lock()
        .expect("lock responses")
        .get_mut(&(parts.method, parts.uri.path().to_owned()))
        .and_then(VecDeque::pop_front);

    match response {
        Some(response) => build_response(response),
        None => build_response(QueuedResponse::text(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "no queued response for {} {}{}",
                captured.method,
                captured.path,
                captured
                    .query
                    .as_deref()
                    .map(|value| format!("?{value}"))
                    .unwrap_or_default()
            ),
        )),
    }
}

fn normalize_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
        })
        .collect()
}

fn build_response(response: QueuedResponse) -> Response<Body> {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }

    builder
        .body(Body::from(response.body))
        .expect("build mock response")
}
