#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::items_after_statements,
    clippy::manual_let_else,
    clippy::needless_pass_by_value,
    clippy::needless_raw_string_hashes,
    clippy::panic,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

mod mock_support;

use client_uploader_traits::{
    CreatePublication, CreatePublicationRequest, DownloadNamedPublicFile, DraftResource,
    DraftWorkflow, ListResourceFiles, LookupByDoi, ReadPublicResource, ResolveLatestPublicResource,
    ResolveLatestPublicResourceByDoi, SearchPublicResources, UpdatePublication,
    UpdatePublicationRequest,
};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::str::FromStr;

use axum::http::{Method, StatusCode};
use serde_json::json;
use tempfile::{tempdir, Builder};
use zenodo_rs::{
    AccessRight, ArtifactSelector, Auth, Creator, DepositMetadataUpdate, DepositionId, Doi,
    Endpoint, FileReplacePolicy, PollOptions, RecordId, RecordSelector, UploadSpec, UploadType,
    ZenodoClient, ZenodoError,
};

use crate::mock_support::{MockZenodoServer, QueuedResponse};

fn creator() -> Creator {
    Creator {
        name: "Doe, Jane".into(),
        affiliation: Some("Zenodo".into()),
        orcid: None,
        gnd: None,
        extra: BTreeMap::new(),
    }
}

fn metadata_update() -> DepositMetadataUpdate {
    DepositMetadataUpdate::builder()
        .title("Example artifact")
        .upload_type(UploadType::Dataset)
        .description_html("<p>Example</p>")
        .creator(creator())
        .access_right(AccessRight::Open)
        .build()
        .expect("valid metadata")
}

fn deposition_json(
    server: &MockZenodoServer,
    id: u64,
    submitted: bool,
    state: &str,
) -> serde_json::Value {
    json!({
        "id": id,
        "submitted": submitted,
        "state": state,
        "metadata": {},
        "files": [],
        "links": {
            "self": server.url(&format!("deposit/depositions/{id}")),
            "bucket": server.url(&format!("files/bucket-{id}"))
        }
    })
}

fn deposition_with_files_json(
    server: &MockZenodoServer,
    id: u64,
    files: serde_json::Value,
) -> serde_json::Value {
    json!({
        "id": id,
        "submitted": false,
        "state": "inprogress",
        "metadata": {},
        "files": files,
        "links": {
            "self": server.url(&format!("deposit/depositions/{id}")),
            "bucket": server.url(&format!("files/bucket-{id}"))
        }
    })
}

fn record_json(server: &MockZenodoServer, id: u64) -> serde_json::Value {
    json!({
        "id": id,
        "recid": id,
        "doi": format!("10.5281/zenodo.{id}"),
        "metadata": { "title": format!("Record {id}") },
        "files": [
            {
                "id": format!("f-{id}"),
                "key": "artifact.bin",
                "size": 5,
                "checksum": "md5:5d41402abc4b2a76b9719d911017c592",
                "links": {
                    "self": server.url(&format!("download/{id}/artifact.bin"))
                }
            }
        ],
        "links": {
            "self": server.url(&format!("records/{id}")),
            "archive": server.url(&format!("download/{id}/archive.zip"))
        }
    })
}

async fn spawn_raw_server(response_bytes: &'static [u8]) -> url::Url {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind raw server");
    let addr = listener.local_addr().expect("raw server addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept raw request");
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).await;
        let _ = stream.write_all(response_bytes).await;
        let _ = stream.shutdown().await;
    });

    url::Url::parse(&format!("http://{addr}/api/")).expect("raw base url")
}

#[tokio::test]
async fn client_sends_expected_headers_and_metadata_payload() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions",
        StatusCode::CREATED,
        deposition_json(&server, 1, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/1",
        StatusCode::OK,
        deposition_json(&server, 1, false, "inprogress"),
    );

    let deposition = client.create_deposition().await.expect("create deposition");
    let updated = client
        .update_metadata(deposition.id, &metadata_update())
        .await
        .expect("update metadata");

    assert_eq!(updated.id, DepositionId(1));

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].body, br#"{}"#);
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer test-token")
    );
    assert_eq!(
        requests[0].headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        requests[0].headers.get("user-agent").map(String::as_str),
        Some("zenodo-rs-tests/0.1")
    );

    let body: serde_json::Value = serde_json::from_slice(&requests[1].body).expect("json body");
    assert_eq!(body["metadata"]["title"], "Example artifact");
    assert_eq!(body["metadata"]["upload_type"], "dataset");
    assert_eq!(body["metadata"]["access_right"], "open");
}

#[tokio::test]
async fn file_operations_use_expected_routes_and_headers() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/9/files",
        StatusCode::OK,
        json!([{
            "id": "a",
            "filename": "artifact.txt",
            "filesize": 4
        }]),
    );
    server.enqueue_text(
        Method::DELETE,
        "/api/deposit/depositions/9/files/a",
        StatusCode::NO_CONTENT,
        "",
    );

    let files = client
        .list_files(DepositionId(9))
        .await
        .expect("list files");
    assert_eq!(files.len(), 1);
    client
        .delete_file(DepositionId(9), "a".into())
        .await
        .expect("delete file");

    let requests = server.requests();
    assert_eq!(requests[0].path, "/api/deposit/depositions/9/files");
    assert_eq!(requests[1].path, "/api/deposit/depositions/9/files/a");
    assert_eq!(requests[1].method, Method::DELETE);
}

#[tokio::test]
async fn upload_path_and_reader_send_known_content_length() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let bucket: zenodo_rs::BucketUrl = server
        .url("files/bucket-1")
        .parse::<url::Url>()
        .expect("bucket url")
        .into();

    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-1/artifact.txt",
        StatusCode::OK,
        json!({ "key": "artifact.txt", "size": 4 }),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-1/reader.bin",
        StatusCode::OK,
        json!({ "key": "reader.bin", "size": 5 }),
    );

    let temp = Builder::new().suffix(".txt").tempfile().expect("temp file");
    std::fs::write(temp.path(), b"data").expect("write temp file");

    client
        .upload_path(&bucket, "artifact.txt", temp.path())
        .await
        .expect("upload path");

    client
        .upload_reader(
            &bucket,
            "reader.bin",
            Cursor::new(b"12345".to_vec()),
            5,
            mime::APPLICATION_OCTET_STREAM,
        )
        .await
        .expect("upload reader");

    let requests = server.requests();
    assert_eq!(requests[0].body, b"data");
    assert_eq!(
        requests[0]
            .headers
            .get("content-length")
            .map(String::as_str),
        Some("4")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/octet-stream")
    );
    assert_eq!(requests[1].body, b"12345");
    assert_eq!(
        requests[1]
            .headers
            .get("content-length")
            .map(String::as_str),
        Some("5")
    );
    assert_eq!(
        requests[1].headers.get("content-type").map(String::as_str),
        Some("application/octet-stream")
    );
}

#[tokio::test]
async fn search_and_version_helpers_follow_record_links() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 10,
                    "recid": 10,
                    "metadata": { "title": "Search result" },
                    "files": [],
                    "links": {}
                })],
                "total": { "value": 1 }
            },
            "links": {
                "next": server.url("records?page=2"),
                "prev": server.url("records?page=1")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/20",
        StatusCode::OK,
        json!({
            "id": 20,
            "recid": 20,
            "conceptrecid": 99,
            "metadata": { "title": "Versioned record" },
            "files": [],
            "links": {
                "latest": server.url("records/21"),
                "versions": server.url("records/20/versions")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/20",
        StatusCode::OK,
        json!({
            "id": 20,
            "recid": 20,
            "conceptrecid": 99,
            "metadata": { "title": "Versioned record" },
            "files": [],
            "links": {
                "latest": server.url("records/21"),
                "versions": server.url("records/20/versions")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/21",
        StatusCode::OK,
        json!({
            "id": 21,
            "recid": 21,
            "metadata": { "title": "Latest" },
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/20/versions",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [
                    { "id": 20, "recid": 20, "metadata": { "title": "v1" }, "files": [], "links": {} },
                    { "id": 21, "recid": 21, "metadata": { "title": "v2" }, "files": [], "links": {} }
                ],
                "total": 2
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/30",
        StatusCode::OK,
        json!({
            "id": 30,
            "recid": 30,
            "conceptrecid": 77,
            "metadata": { "title": "Fallback versions" },
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [{ "id": 30, "recid": 30, "metadata": { "title": "Fallback versions" }, "files": [], "links": {} }],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/31",
        StatusCode::OK,
        json!({
            "id": 31,
            "recid": 31,
            "metadata": { "title": "Singleton" },
            "files": [],
            "links": {}
        }),
    );

    let page = client
        .search_records(&zenodo_rs::RecordQuery {
            q: Some("title:test".into()),
            status: Some(zenodo_rs::RecordQueryStatus::Published),
            sort: Some(zenodo_rs::RecordSort::MostRecent),
            page: Some(2),
            size: Some(25),
            all_versions: true,
            communities: vec!["alpha".into()],
            resource_type: Some("dataset".into()),
            subtype: Some("figure".into()),
            custom: vec![("foo".into(), "bar".into())],
        })
        .await
        .expect("search records");
    assert_eq!(page.total, Some(1));
    assert_eq!(page.hits[0].id, RecordId(10));

    let latest = client
        .resolve_latest_version(RecordId(20))
        .await
        .expect("latest version");
    assert_eq!(latest.id, RecordId(21));

    let versions = client
        .list_record_versions(RecordId(20))
        .await
        .expect("versions via link");
    assert_eq!(versions.total, Some(2));

    let fallback = client
        .list_record_versions(RecordId(30))
        .await
        .expect("versions via conceptrecid search");
    assert_eq!(fallback.total, Some(1));

    let singleton = client
        .list_record_versions(RecordId(31))
        .await
        .expect("singleton versions");
    assert_eq!(singleton.total, Some(1));
    assert_eq!(singleton.hits[0].id, RecordId(31));

    let requests = server.requests();
    let search_request = requests
        .iter()
        .find(|request| {
            request.path == "/api/records"
                && request
                    .query
                    .as_deref()
                    .unwrap_or_default()
                    .contains("title%3Atest")
        })
        .expect("initial search request");
    let query = search_request.query.as_deref().expect("search query");
    assert!(query.contains("q=title%3Atest"));
    assert!(query.contains("status=published"));
    assert!(query.contains("sort=mostrecent"));
    assert!(query.contains("page=2"));
    assert!(query.contains("size=25"));
    assert!(query.contains("all_versions=true"));
    assert!(query.contains("communities=alpha"));
    assert!(query.contains("type=dataset"));
    assert!(query.contains("subtype=figure"));
    assert!(query.contains("foo=bar"));
}

#[tokio::test]
async fn doi_and_artifact_helpers_resolve_latest_and_download_files() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    let latest_path = dir.path().join("latest.bin");
    let archive_path = dir.path().join("archive.zip");

    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 40,
                    "recid": 40,
                    "doi": "10.5281/zenodo.40",
                    "metadata": { "title": "By DOI" },
                    "files": [],
                    "links": { "latest": server.url("records/41") }
                })],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/40",
        StatusCode::OK,
        record_json(&server, 40),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/40",
        StatusCode::OK,
        record_json(&server, 40),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/41",
        StatusCode::OK,
        json!({
            "id": 41,
            "recid": 41,
            "doi": "10.5281/zenodo.41",
            "metadata": { "title": "Latest DOI record" },
            "files": [{
                "id": "f-41",
                "key": "artifact.bin",
                "size": 5,
                "checksum": "md5:7d793037a0760186574b0282f2f435e7",
                "links": {
                    "self": server.url("download/41/artifact.bin")
                }
            }],
            "links": {
                "archive": server.url("download/41/archive.zip")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 40,
                    "recid": 40,
                    "doi": "10.5281/zenodo.40",
                    "metadata": { "title": "By DOI" },
                    "files": [],
                    "links": { "latest": server.url("records/41") }
                })],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/41",
        StatusCode::OK,
        json!({
            "id": 41,
            "recid": 41,
            "doi": "10.5281/zenodo.41",
            "metadata": { "title": "Latest DOI record" },
            "files": [{
                "id": "f-41",
                "key": "artifact.bin",
                "size": 5,
                "checksum": "md5:7d793037a0760186574b0282f2f435e7",
                "links": {
                    "self": server.url("download/41/artifact.bin")
                }
            }],
            "links": {
                "archive": server.url("download/41/archive.zip")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 40,
                    "recid": 40,
                    "doi": "10.5281/zenodo.40",
                    "metadata": { "title": "By DOI" },
                    "files": [],
                    "links": { "latest": server.url("records/41") }
                })],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/41",
        StatusCode::OK,
        json!({
            "id": 41,
            "recid": 41,
            "doi": "10.5281/zenodo.41",
            "metadata": { "title": "Latest DOI record" },
            "files": [{
                "id": "f-41",
                "key": "artifact.bin",
                "size": 5,
                "checksum": "md5:7d793037a0760186574b0282f2f435e7",
                "links": {
                    "self": server.url("download/41/artifact.bin")
                }
            }],
            "links": {
                "archive": server.url("download/41/archive.zip")
            }
        }),
    );
    server.enqueue(
        Method::GET,
        "/api/download/40/artifact.bin",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![
                ("content-type".into(), "application/octet-stream".into()),
                ("content-length".into(), "5".into()),
                (
                    "content-disposition".into(),
                    "attachment; filename=artifact.bin".into(),
                ),
            ],
            b"hello".to_vec(),
        ),
    );
    server.enqueue(
        Method::GET,
        "/api/download/41/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"world".to_vec()),
    );
    server.enqueue(
        Method::GET,
        "/api/download/41/archive.zip",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"zip!".to_vec()),
    );

    let info = client
        .get_artifact_info(RecordId(40))
        .await
        .expect("artifact info");
    assert_eq!(info.record.id, RecordId(40));

    let latest_by_doi = client
        .resolve_latest_by_doi(&Doi::new("10.5281/zenodo.40").expect("doi"))
        .await
        .expect("resolve latest by doi");
    assert_eq!(latest_by_doi.id, RecordId(41));

    let direct = client
        .download_record_file_by_key_to_path(RecordId(40), "artifact.bin", &path)
        .await
        .expect("download direct");
    assert_eq!(direct.resolved_record, RecordId(40));
    assert_eq!(
        std::fs::read(&path).expect("read downloaded file"),
        b"hello"
    );

    let doi = Doi::from_str("10.5281/zenodo.40").expect("doi");
    let resolved = client
        .download_file_by_doi_to_path(&doi, "artifact.bin", true, &latest_path)
        .await
        .expect("download latest by doi");
    assert_eq!(resolved.resolved_record, RecordId(41));
    assert_eq!(
        resolved.requested,
        ArtifactSelector::FileByKey {
            record: RecordSelector::Doi(doi.clone()),
            key: "artifact.bin".into(),
            latest: true,
        }
    );
    assert_eq!(
        std::fs::read(&latest_path).expect("read latest file"),
        b"world"
    );

    let archive = client
        .download_artifact(
            &ArtifactSelector::Archive {
                record: RecordSelector::Doi(doi),
                latest: true,
            },
            &archive_path,
        )
        .await
        .expect("download archive");
    assert_eq!(archive.bytes_written, 4);
    assert_eq!(std::fs::read(&archive_path).expect("read archive"), b"zip!");
}

#[tokio::test]
async fn open_artifact_exposes_response_headers() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/records/500",
        StatusCode::OK,
        json!({
            "id": 500,
            "recid": 500,
            "metadata": { "title": "Stream me" },
            "files": [{
                "id": "f-500",
                "key": "artifact.bin",
                "size": 3,
                "links": { "self": server.url("download/500/artifact.bin") }
            }],
            "links": {}
        }),
    );
    server.enqueue(
        Method::GET,
        "/api/download/500/artifact.bin",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![
                ("content-type".into(), "application/octet-stream".into()),
                ("content-length".into(), "3".into()),
                (
                    "content-disposition".into(),
                    "attachment; filename=file.bin".into(),
                ),
            ],
            b"abc".to_vec(),
        ),
    );

    let mut stream = client
        .open_artifact(&ArtifactSelector::FileByKey {
            record: RecordSelector::RecordId(RecordId(500)),
            key: "artifact.bin".into(),
            latest: false,
        })
        .await
        .expect("open artifact");

    let mut bytes = Vec::new();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.stream.next().await {
        bytes.extend_from_slice(&chunk.expect("chunk"));
    }

    assert_eq!(stream.content_length, Some(3));
    assert_eq!(
        stream.content_type.as_ref().map(mime::Mime::as_ref),
        Some("application/octet-stream")
    );
    assert_eq!(
        stream.content_disposition.as_deref(),
        Some("attachment; filename=file.bin")
    );
    assert_eq!(bytes, b"abc");

    let requests = server.requests();
    assert_eq!(
        requests[1].headers.get("authorization").map(String::as_str),
        Some("Bearer test-token")
    );
    assert_ne!(
        requests[1].headers.get("accept").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn cross_origin_downloads_do_not_forward_auth_or_json_accept() {
    let api_server = MockZenodoServer::start().await;
    let download_server = MockZenodoServer::start().await;
    let client = api_server.client();

    api_server.enqueue_json(
        Method::GET,
        "/api/records/501",
        StatusCode::OK,
        json!({
            "id": 501,
            "recid": 501,
            "metadata": { "title": "Cross origin" },
            "files": [{
                "id": "f-501",
                "key": "artifact.bin",
                "size": 3,
                "links": { "self": download_server.url("download/501/artifact.bin") }
            }],
            "links": {}
        }),
    );
    download_server.enqueue(
        Method::GET,
        "/api/download/501/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"abc".to_vec()),
    );

    let mut stream = client
        .open_artifact(&ArtifactSelector::FileByKey {
            record: RecordSelector::RecordId(RecordId(501)),
            key: "artifact.bin".into(),
            latest: false,
        })
        .await
        .expect("open artifact");

    use futures_util::StreamExt;
    while let Some(chunk) = stream.stream.next().await {
        let _ = chunk.expect("chunk");
    }

    let requests = download_server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].headers.get("authorization"), None);
    assert_ne!(
        requests[0].headers.get("accept").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn doi_resolution_requires_exact_match_and_can_follow_next_page() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let doi = Doi::from_str(" HTTPS://DOI.ORG/10.5281/ZENODO.600 ").expect("doi");

    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 599,
                    "recid": 599,
                    "doi": "10.5281/zenodo.599",
                    "metadata": { "title": "wrong match" },
                    "files": [],
                    "links": {}
                })],
                "total": 2
            },
            "links": {
                "next": server.url("records?page=2")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 600,
                    "recid": 600,
                    "conceptdoi": "10.5281/zenodo.600",
                    "metadata": { "title": "exact concept doi" },
                    "files": [],
                    "links": {}
                })],
                "total": 2
            },
            "links": {}
        }),
    );

    let record = client
        .get_record_by_doi(&doi)
        .await
        .expect("exact doi match");
    assert_eq!(record.id, RecordId(600));

    let requests = server.requests();
    assert!(requests[0]
        .query
        .as_deref()
        .unwrap_or_default()
        .contains("doi%3A%2210.5281%2Fzenodo.600%22"));
}

#[tokio::test]
async fn doi_string_helpers_cover_success_paths() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 700,
                    "recid": 700.0,
                    "doi": "10.5281/zenodo.700",
                    "metadata": { "title": "v1" },
                    "files": [],
                    "links": {
                        "latest": server.url("records/701")
                    }
                })],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": {
                "hits": [json!({
                    "id": 700,
                    "recid": 700,
                    "doi": "10.5281/zenodo.700",
                    "metadata": { "title": "v1" },
                    "files": [],
                    "links": {
                        "latest": server.url("records/701")
                    }
                })],
                "total": 1
            },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/701",
        StatusCode::OK,
        json!({
            "id": 701,
            "recid": 701,
            "doi": "10.5281/zenodo.701",
            "metadata": { "title": "v2" },
            "files": [],
            "links": {}
        }),
    );

    let direct = client
        .get_record_by_doi_str("doi:10.5281/ZENODO.700")
        .await
        .expect("record from doi string");
    assert_eq!(direct.id, RecordId(700));

    let latest = client
        .resolve_latest_by_doi_str("https://doi.org/10.5281/zenodo.700")
        .await
        .expect("latest from doi string");
    assert_eq!(latest.id, RecordId(701));
}

#[tokio::test]
async fn failed_download_preserves_existing_destination_file() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    std::fs::write(&path, b"good").expect("seed destination");
    let raw_base = spawn_raw_server(
        b"HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: 10\r\n\r\nabc",
    )
    .await;
    let raw_url = raw_base.join("download/broken.bin").expect("raw url");

    server.enqueue_json(
        Method::GET,
        "/api/records/601",
        StatusCode::OK,
        json!({
            "id": 601,
            "recid": 601,
            "metadata": { "title": "broken" },
            "files": [{
                "id": "f-601",
                "key": "artifact.bin",
                "size": 10,
                "links": { "self": raw_url }
            }],
            "links": {}
        }),
    );

    let error = client
        .download_record_file_by_key_to_path(RecordId(601), "artifact.bin", &path)
        .await
        .expect_err("download should fail");
    assert!(matches!(error, ZenodoError::Transport(_)));
    assert_eq!(std::fs::read(&path).expect("read destination"), b"good");
}

#[cfg(feature = "checksums")]
#[tokio::test]
async fn checksum_mismatch_preserves_existing_destination_file() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    std::fs::write(&path, b"good").expect("seed destination");

    server.enqueue_json(
        Method::GET,
        "/api/records/602",
        StatusCode::OK,
        json!({
            "id": 602,
            "recid": 602,
            "metadata": { "title": "bad checksum" },
            "files": [{
                "id": "f-602",
                "key": "artifact.bin",
                "size": 3,
                "checksum": "md5:ffffffffffffffffffffffffffffffff",
                "links": { "self": server.url("download/602/artifact.bin") }
            }],
            "links": {}
        }),
    );
    server.enqueue(
        Method::GET,
        "/api/download/602/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"abc".to_vec()),
    );

    let error = client
        .download_record_file_by_key_to_path(RecordId(602), "artifact.bin", &path)
        .await
        .expect_err("download should fail");
    assert!(matches!(error, ZenodoError::ChecksumMismatch { .. }));
    assert_eq!(std::fs::read(&path).expect("read destination"), b"good");
}

#[tokio::test]
async fn workflow_helpers_follow_new_version_delete_upload_and_publish() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("dataset.txt");
    std::fs::write(&artifact_path, b"data").expect("write upload file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/50",
        StatusCode::OK,
        json!({
            "id": 50,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/50/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 50,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/51")
            }
        }),
    );
    server.enqueue_text(
        Method::GET,
        "/api/deposit/depositions/51",
        StatusCode::CONFLICT,
        "still integrating",
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/51",
        StatusCode::OK,
        json!({
            "id": 51,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": server.url("files/bucket-51")
            }
        }),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/51",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            51,
            json!([
                { "id": "old-a", "filename": "old-a.txt", "filesize": 1 },
                { "id": "old-b", "filename": "old-b.txt", "filesize": 1 }
            ]),
        ),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/51",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            51,
            json!([
                { "id": "old-a", "filename": "old-a.txt", "filesize": 1 },
                { "id": "old-b", "filename": "old-b.txt", "filesize": 1 }
            ]),
        ),
    );
    server.enqueue_text(
        Method::DELETE,
        "/api/deposit/depositions/51/files/old-a",
        StatusCode::NO_CONTENT,
        "",
    );
    server.enqueue_text(
        Method::DELETE,
        "/api/deposit/depositions/51/files/old-b",
        StatusCode::NO_CONTENT,
        "",
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-51/dataset.txt",
        StatusCode::OK,
        json!({ "key": "dataset.txt", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/51/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 51,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": server.url("files/bucket-51")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/51",
        StatusCode::OK,
        json!({
            "id": 51,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": server.url("files/bucket-51")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/51",
        StatusCode::OK,
        json!({
            "id": 51,
            "submitted": true,
            "state": "done",
            "record_id": 60,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/60",
        StatusCode::OK,
        record_json(&server, 60),
    );

    let published = client
        .publish_dataset(
            DepositionId(50),
            &metadata_update(),
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        )
        .await
        .expect("publish dataset");

    assert_eq!(published.deposition.id, DepositionId(51));
    assert_eq!(published.record.id, RecordId(60));

    let requests = server.requests();
    let paths: Vec<_> = requests
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/50",
            "/api/deposit/depositions/50/actions/newversion",
            "/api/deposit/depositions/51",
            "/api/deposit/depositions/51",
            "/api/deposit/depositions/51",
            "/api/deposit/depositions/51",
            "/api/deposit/depositions/51/files/old-a",
            "/api/deposit/depositions/51/files/old-b",
            "/api/files/bucket-51/dataset.txt",
            "/api/deposit/depositions/51/actions/publish",
            "/api/deposit/depositions/51",
            "/api/deposit/depositions/51",
            "/api/records/60"
        ]
    );
}

#[tokio::test]
async fn publish_dataset_reports_missing_record_id_after_publish() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("dataset.txt");
    std::fs::write(&artifact_path, b"data").expect("write upload file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        deposition_json(&server, 98, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        deposition_json(&server, 98, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        deposition_json(&server, 98, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-98/dataset.txt",
        StatusCode::OK,
        json!({ "key": "dataset.txt", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/98/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 98,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        json!({
            "id": 98,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );

    let error = client
        .publish_dataset(
            DepositionId(98),
            &metadata_update(),
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        )
        .await
        .expect_err("missing record id should fail");

    assert!(matches!(
        error,
        ZenodoError::InvalidState(message) if message == "published deposition is missing record_id"
    ));
}

#[tokio::test]
async fn create_and_publish_dataset_creates_a_fresh_deposition_first() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("local-name.bin");
    std::fs::write(&artifact_path, b"data").expect("write upload file");

    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions",
        StatusCode::CREATED,
        deposition_json(&server, 72, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/72",
        StatusCode::OK,
        deposition_json(&server, 72, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/72",
        StatusCode::OK,
        deposition_json(&server, 72, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/72",
        StatusCode::OK,
        deposition_json(&server, 72, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-72/archive-name.bin",
        StatusCode::OK,
        json!({ "key": "archive-name.bin", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/72/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 72,
            "submitted": true,
            "state": "done",
            "record_id": 82,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/72",
        StatusCode::OK,
        json!({
            "id": 72,
            "submitted": true,
            "state": "done",
            "record_id": 82,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/82",
        StatusCode::OK,
        record_json(&server, 82),
    );

    let files = UploadSpec::from_named_paths([("archive-name.bin", artifact_path.as_path())])
        .expect("manifest upload specs");

    let published = client
        .create_and_publish_dataset(&metadata_update(), files)
        .await
        .expect("create and publish dataset");

    assert_eq!(published.deposition.id, DepositionId(72));
    assert_eq!(published.record.id, RecordId(82));

    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions",
            "/api/deposit/depositions/72",
            "/api/deposit/depositions/72",
            "/api/deposit/depositions/72",
            "/api/files/bucket-72/archive-name.bin",
            "/api/deposit/depositions/72/actions/publish",
            "/api/deposit/depositions/72",
            "/api/records/82"
        ]
    );
}

#[tokio::test]
async fn repository_client_trait_methods_delegate_to_record_apis() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    let doi = Doi::new("10.5281/zenodo.700").expect("doi");

    let record_700 = json!({
        "id": 700,
        "recid": 700,
        "doi": "10.5281/zenodo.700",
        "metadata": { "title": "Trait record" },
        "files": [{
            "id": "f-700",
            "key": "artifact.bin",
            "size": 5,
            "checksum": "md5:5d41402abc4b2a76b9719d911017c592",
            "links": {
                "self": server.url("download/700/artifact.bin")
            }
        }],
        "links": {
            "latest": server.url("records/701")
        }
    });
    let record_701 = json!({
        "id": 701,
        "recid": 701,
        "doi": "10.5281/zenodo.701",
        "metadata": { "title": "Latest trait record" },
        "files": [{
            "id": "f-701",
            "key": "artifact.bin",
            "size": 6,
            "links": {
                "self": server.url("download/701/artifact.bin")
            }
        }],
        "links": {}
    });

    server.enqueue_json(
        Method::GET,
        "/api/records/700",
        StatusCode::OK,
        record_700.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [record_700.clone()], "total": 1 },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/700",
        StatusCode::OK,
        record_700.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/700",
        StatusCode::OK,
        record_700.clone(),
    );
    server.enqueue(
        Method::GET,
        "/api/download/700/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"hello".to_vec()),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [record_700.clone()], "total": 1 },
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/700",
        StatusCode::OK,
        record_700.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/701",
        StatusCode::OK,
        record_701.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [record_700], "total": 1 },
            "links": {}
        }),
    );
    server.enqueue_json(Method::GET, "/api/records/701", StatusCode::OK, record_701);

    let public = <ZenodoClient as ReadPublicResource>::get_public_resource(&client, &RecordId(700))
        .await
        .expect("get public resource");
    assert_eq!(public.id, RecordId(700));

    let results = <ZenodoClient as SearchPublicResources>::search_public_resources(
        &client,
        &zenodo_rs::RecordQuery::builder()
            .query("title:\"Trait record\"")
            .build(),
    )
    .await
    .expect("search public resources");
    assert_eq!(results.hits.len(), 1);

    let files = <ZenodoClient as ListResourceFiles>::list_resource_files(&client, &RecordId(700))
        .await
        .expect("list resource files");
    assert_eq!(files.len(), 1);

    let download = <ZenodoClient as DownloadNamedPublicFile>::download_named_public_file_to_path(
        &client,
        &RecordId(700),
        "artifact.bin",
        &path,
    )
    .await
    .expect("download named public file");
    assert_eq!(download.resolved_record, RecordId(700));
    assert_eq!(std::fs::read(&path).expect("read download"), b"hello");

    let by_doi = <ZenodoClient as LookupByDoi>::get_public_resource_by_doi(&client, &doi)
        .await
        .expect("lookup by doi");
    assert_eq!(by_doi.id, RecordId(700));

    let latest = <ZenodoClient as ResolveLatestPublicResource>::resolve_latest_public_resource(
        &client,
        &RecordId(700),
    )
    .await
    .expect("resolve latest public resource");
    assert_eq!(latest.id, RecordId(701));

    let latest_by_doi =
        <ZenodoClient as ResolveLatestPublicResourceByDoi>::resolve_latest_public_resource_by_doi(
            &client, &doi,
        )
        .await
        .expect("resolve latest public resource by doi");
    assert_eq!(latest_by_doi.id, RecordId(701));
}

#[tokio::test]
async fn repository_client_trait_methods_delegate_to_publication_apis() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("artifact.bin");
    std::fs::write(&artifact_path, b"data").expect("write upload file");

    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions",
        StatusCode::CREATED,
        deposition_json(&server, 930, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/930",
        StatusCode::OK,
        deposition_json(&server, 930, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/930",
        StatusCode::OK,
        deposition_json(&server, 930, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/930",
        StatusCode::OK,
        deposition_json(&server, 930, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-930/artifact.bin",
        StatusCode::OK,
        json!({ "id": "bucket-930", "key": "artifact.bin", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/930/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 930,
            "submitted": true,
            "state": "done",
            "record_id": 931,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/930",
        StatusCode::OK,
        json!({
            "id": 930,
            "submitted": true,
            "state": "done",
            "record_id": 931,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/931",
        StatusCode::OK,
        record_json(&server, 931),
    );

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/940",
        StatusCode::OK,
        deposition_json(&server, 940, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/940",
        StatusCode::OK,
        deposition_json(&server, 940, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/940",
        StatusCode::OK,
        deposition_json(&server, 940, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-940/artifact.bin",
        StatusCode::OK,
        json!({ "id": "bucket-940", "key": "artifact.bin", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/940/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 940,
            "submitted": true,
            "state": "done",
            "record_id": 941,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/940",
        StatusCode::OK,
        json!({
            "id": 940,
            "submitted": true,
            "state": "done",
            "record_id": 941,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/941",
        StatusCode::OK,
        record_json(&server, 941),
    );

    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions",
        StatusCode::CREATED,
        deposition_json(&server, 950, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/950",
        StatusCode::OK,
        deposition_json(&server, 950, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/950",
        StatusCode::OK,
        deposition_json(&server, 950, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/950",
        StatusCode::OK,
        deposition_json(&server, 950, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-950/artifact.bin",
        StatusCode::OK,
        json!({ "id": "bucket-950", "key": "artifact.bin", "size": 4 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/950/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 950,
            "submitted": true,
            "state": "done",
            "record_id": 951,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );

    let created = <ZenodoClient as CreatePublication>::create_publication(
        &client,
        CreatePublicationRequest::untargeted(
            metadata_update(),
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        ),
    )
    .await
    .expect("create publication");
    assert_eq!(created.record.id, RecordId(931));

    let updated = <ZenodoClient as UpdatePublication>::update_publication(
        &client,
        UpdatePublicationRequest::new(
            DepositionId(940),
            metadata_update(),
            FileReplacePolicy::ReplaceAll,
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        ),
    )
    .await
    .expect("update publication");
    assert_eq!(updated.deposition.id, DepositionId(940));

    let draft = <ZenodoClient as DraftWorkflow>::create_draft(&client, &metadata_update())
        .await
        .expect("create draft");
    assert_eq!(draft.draft_id(), DepositionId(950));

    let updated_draft = <ZenodoClient as DraftWorkflow>::update_draft_metadata(
        &client,
        &DepositionId(950),
        &metadata_update(),
    )
    .await
    .expect("update draft metadata");
    assert_eq!(updated_draft.id, DepositionId(950));

    let uploaded = <ZenodoClient as DraftWorkflow>::reconcile_draft_files(
        &client,
        &draft,
        FileReplacePolicy::ReplaceAll,
        vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
    )
    .await
    .expect("reconcile draft files");
    assert_eq!(uploaded.len(), 1);
    assert_eq!(uploaded[0].key, "artifact.bin");

    let published = <ZenodoClient as DraftWorkflow>::publish_draft(&client, &DepositionId(950))
        .await
        .expect("publish draft");
    assert!(published.is_published());
}

#[tokio::test]
async fn publish_dataset_with_policy_uses_upsert_by_filename() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let match_path = dir.path().join("match.txt");
    let new_path = dir.path().join("new.txt");
    std::fs::write(&match_path, b"match").expect("write match upload");
    std::fs::write(&new_path, b"brand").expect("write new upload");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/55",
        StatusCode::OK,
        deposition_json(&server, 55, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/55",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            55,
            json!([
                { "id": "match-id", "filename": "match.txt", "filesize": 5 },
                { "id": "keep-id", "filename": "keep.txt", "filesize": 4 }
            ]),
        ),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/55",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            55,
            json!([
                { "id": "match-id", "filename": "match.txt", "filesize": 5 },
                { "id": "keep-id", "filename": "keep.txt", "filesize": 4 }
            ]),
        ),
    );
    server.enqueue_text(
        Method::DELETE,
        "/api/deposit/depositions/55/files/match-id",
        StatusCode::NO_CONTENT,
        "",
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-55/match.txt",
        StatusCode::OK,
        json!({ "key": "match.txt", "size": 5 }),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-55/new.txt",
        StatusCode::OK,
        json!({ "key": "new.txt", "size": 5 }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/55/actions/publish",
        StatusCode::ACCEPTED,
        json!({
            "id": 55,
            "submitted": true,
            "state": "done",
            "record_id": 65,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/55",
        StatusCode::OK,
        json!({
            "id": 55,
            "submitted": true,
            "state": "done",
            "record_id": 65,
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/65",
        StatusCode::OK,
        record_json(&server, 65),
    );

    let published = client
        .publish_dataset_with_policy(
            DepositionId(55),
            &metadata_update(),
            FileReplacePolicy::UpsertByFilename,
            vec![
                UploadSpec::from_path(&match_path).expect("match upload spec"),
                UploadSpec::from_path(&new_path).expect("new upload spec"),
            ],
        )
        .await
        .expect("publish dataset with upsert");

    assert_eq!(published.deposition.id, DepositionId(55));
    assert_eq!(published.record.id, RecordId(65));

    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/55",
            "/api/deposit/depositions/55",
            "/api/deposit/depositions/55",
            "/api/deposit/depositions/55/files/match-id",
            "/api/files/bucket-55/match.txt",
            "/api/files/bucket-55/new.txt",
            "/api/deposit/depositions/55/actions/publish",
            "/api/deposit/depositions/55",
            "/api/records/65"
        ]
    );
}

#[tokio::test]
async fn ensure_editable_draft_reuses_unpublished_deposition() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/70",
        StatusCode::OK,
        deposition_json(&server, 70, false, "inprogress"),
    );

    let draft = client
        .ensure_editable_draft(DepositionId(70))
        .await
        .expect("ensure draft");

    assert_eq!(draft.id, DepositionId(70));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/deposit/depositions/70");
}

#[tokio::test]
async fn ensure_editable_draft_resolves_latest_published_deposition_before_newversion() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/71",
        StatusCode::OK,
        json!({
            "id": 71,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/71"),
                "latest": server.url("deposit/depositions/72")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/72",
        StatusCode::OK,
        json!({
            "id": 72,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/72"),
                "latest": server.url("deposit/depositions/72")
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/72/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 72,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/73")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/73",
        StatusCode::OK,
        json!({
            "id": 73,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/73"),
                "bucket": server.url("files/bucket-73")
            }
        }),
    );

    let draft = client
        .ensure_editable_draft(DepositionId(71))
        .await
        .expect("ensure latest draft from stale published deposition");

    assert_eq!(draft.id, DepositionId(73));

    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/71",
            "/api/deposit/depositions/72",
            "/api/deposit/depositions/72/actions/newversion",
            "/api/deposit/depositions/73",
        ]
    );
}

#[tokio::test]
async fn ensure_editable_draft_falls_back_to_latest_record_when_deposition_has_no_latest_link() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/81",
        StatusCode::OK,
        json!({
            "id": 81,
            "submitted": true,
            "state": "done",
            "record_id": 81,
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/81")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/81",
        StatusCode::OK,
        json!({
            "id": 81,
            "recid": 81,
            "metadata": { "title": "Record 81" },
            "files": [],
            "links": {
                "self": server.url("records/81"),
                "latest": server.url("records/82")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/82",
        StatusCode::OK,
        json!({
            "id": 82,
            "recid": 82,
            "metadata": { "title": "Record 82" },
            "files": [],
            "links": {
                "self": server.url("records/82")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/82",
        StatusCode::OK,
        json!({
            "id": 82,
            "submitted": true,
            "state": "done",
            "record_id": 82,
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/82")
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/82/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 82,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/83")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/83",
        StatusCode::OK,
        json!({
            "id": 83,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/83"),
                "bucket": server.url("files/bucket-83")
            }
        }),
    );

    let draft = client
        .ensure_editable_draft(DepositionId(81))
        .await
        .expect("ensure latest draft from record fallback");

    assert_eq!(draft.id, DepositionId(83));

    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/81",
            "/api/records/81",
            "/api/records/82",
            "/api/deposit/depositions/82",
            "/api/deposit/depositions/82/actions/newversion",
            "/api/deposit/depositions/83",
        ]
    );
}

#[tokio::test]
async fn http_errors_surface_through_public_api() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/80",
        StatusCode::BAD_REQUEST,
        json!({
            "message": "bad request",
            "errors": [
                { "field": "metadata.title", "message": "required" }
            ]
        }),
    );

    let error = client
        .get_deposition(DepositionId(80))
        .await
        .expect_err("expected error");

    match error {
        ZenodoError::Http {
            status,
            message,
            field_errors,
            ..
        } => {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(message.as_deref(), Some("bad request"));
            assert_eq!(field_errors.len(), 1);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn replace_all_files_deletes_all_existing_files() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("fresh.txt");
    std::fs::write(&artifact_path, b"fresh").expect("write file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/90",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            90,
            json!([{ "id": "stale", "filename": "stale.txt", "filesize": 5 }]),
        ),
    );
    server.enqueue_json(
        Method::DELETE,
        "/api/deposit/depositions/90/files/stale",
        StatusCode::OK,
        json!({}),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-90/fresh.txt",
        StatusCode::OK,
        json!({ "key": "fresh.txt", "size": 5 }),
    );

    let uploaded = client
        .replace_all_files(
            &serde_json::from_value(deposition_with_files_json(
                &server,
                90,
                json!([{ "id": "stale", "filename": "stale.txt", "filesize": 5 }]),
            ))
            .expect("draft"),
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        )
        .await
        .expect("replace all files");

    assert_eq!(uploaded.len(), 1);
    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/90",
            "/api/deposit/depositions/90/files/stale",
            "/api/files/bucket-90/fresh.txt"
        ]
    );
}

#[tokio::test]
async fn reconcile_files_upsert_by_filename_only_deletes_matching_files() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let match_path = dir.path().join("match.txt");
    let new_path = dir.path().join("new.txt");
    std::fs::write(&match_path, b"match").expect("write match file");
    std::fs::write(&new_path, b"brand").expect("write new file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/91",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            91,
            json!([
                { "id": "match-id", "filename": "match.txt", "filesize": 5 },
                { "id": "keep-id", "filename": "keep.txt", "filesize": 4 }
            ]),
        ),
    );
    server.enqueue_text(
        Method::DELETE,
        "/api/deposit/depositions/91/files/match-id",
        StatusCode::NO_CONTENT,
        "",
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-91/match.txt",
        StatusCode::OK,
        json!({ "key": "match.txt", "size": 5 }),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-91/new.txt",
        StatusCode::OK,
        json!({ "key": "new.txt", "size": 5 }),
    );

    let uploaded = client
        .reconcile_files(
            &serde_json::from_value(deposition_with_files_json(
                &server,
                91,
                json!([
                    { "id": "match-id", "filename": "match.txt", "filesize": 5 },
                    { "id": "keep-id", "filename": "keep.txt", "filesize": 4 }
                ]),
            ))
            .expect("draft"),
            FileReplacePolicy::UpsertByFilename,
            vec![
                UploadSpec::from_path(&match_path).expect("match upload spec"),
                UploadSpec::from_path(&new_path).expect("new upload spec"),
            ],
        )
        .await
        .expect("reconcile by filename");

    assert_eq!(uploaded.len(), 2);
    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/91",
            "/api/deposit/depositions/91/files/match-id",
            "/api/files/bucket-91/match.txt",
            "/api/files/bucket-91/new.txt"
        ]
    );
}

#[tokio::test]
async fn reconcile_files_keep_existing_adds_without_deleting() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("fresh.txt");
    std::fs::write(&artifact_path, b"fresh").expect("write file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/92",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            92,
            json!([{ "id": "stale", "filename": "stale.txt", "filesize": 5 }]),
        ),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-92/fresh.txt",
        StatusCode::OK,
        json!({ "key": "fresh.txt", "size": 5 }),
    );

    let uploaded = client
        .reconcile_files(
            &serde_json::from_value(deposition_with_files_json(
                &server,
                92,
                json!([{ "id": "stale", "filename": "stale.txt", "filesize": 5 }]),
            ))
            .expect("draft"),
            FileReplacePolicy::KeepExistingAndAdd,
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        )
        .await
        .expect("keep existing and add");

    assert_eq!(uploaded.len(), 1);
    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/92",
            "/api/files/bucket-92/fresh.txt"
        ]
    );
}

#[tokio::test]
async fn reconcile_files_keep_existing_rejects_filename_collisions() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");
    let artifact_path = dir.path().join("fresh.txt");
    std::fs::write(&artifact_path, b"fresh").expect("write file");

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/93",
        StatusCode::OK,
        deposition_with_files_json(
            &server,
            93,
            json!([{ "id": "existing", "filename": "fresh.txt", "filesize": 5 }]),
        ),
    );

    let error = client
        .reconcile_files(
            &serde_json::from_value(deposition_with_files_json(
                &server,
                93,
                json!([{ "id": "existing", "filename": "fresh.txt", "filesize": 5 }]),
            ))
            .expect("draft"),
            FileReplacePolicy::KeepExistingAndAdd,
            vec![UploadSpec::from_path(&artifact_path).expect("upload spec")],
        )
        .await
        .expect_err("keep-existing conflict should fail");

    assert!(matches!(
        error,
        ZenodoError::ConflictingDraftFile { filename } if filename == "fresh.txt"
    ));
    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(paths, vec!["/api/deposit/depositions/93"]);
}

#[tokio::test]
async fn reconcile_files_rejects_duplicate_uploaded_filenames() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/94",
        StatusCode::OK,
        deposition_json(&server, 94, false, "inprogress"),
    );

    let error = client
        .reconcile_files(
            &serde_json::from_value(deposition_json(&server, 94, false, "inprogress"))
                .expect("draft"),
            FileReplacePolicy::ReplaceAll,
            vec![
                UploadSpec::from_reader(
                    "artifact.bin",
                    Cursor::new(vec![1_u8]),
                    1,
                    mime::APPLICATION_OCTET_STREAM,
                ),
                UploadSpec::from_reader(
                    "artifact.bin",
                    Cursor::new(vec![2_u8]),
                    1,
                    mime::APPLICATION_OCTET_STREAM,
                ),
            ],
        )
        .await
        .expect_err("duplicate uploads should fail");

    assert!(matches!(
        error,
        ZenodoError::DuplicateUploadFilename { filename } if filename == "artifact.bin"
    ));
    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(paths, vec!["/api/deposit/depositions/94"]);
}

#[tokio::test]
async fn enter_edit_mode_waits_for_current_record_to_become_editable() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/95",
        StatusCode::OK,
        json!({
            "id": 95,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/95/actions/edit",
        StatusCode::ACCEPTED,
        json!({
            "id": 95,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/95",
        StatusCode::OK,
        json!({
            "id": 95,
            "submitted": true,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );

    let draft = client
        .enter_edit_mode(DepositionId(95))
        .await
        .expect("edit mode");
    assert_eq!(draft.id, DepositionId(95));
    assert!(draft.is_published());
    assert_eq!(draft.status.state, zenodo_rs::DepositState::InProgress);
}

#[tokio::test]
async fn enter_edit_mode_reuses_unpublished_or_immediately_editable_depositions() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/96",
        StatusCode::OK,
        json!({
            "id": 96,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": server.url("files/bucket-96")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/97",
        StatusCode::OK,
        json!({
            "id": 97,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/97/actions/edit",
        StatusCode::OK,
        json!({
            "id": 97,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "bucket": server.url("files/bucket-97")
            }
        }),
    );

    let reused = client
        .enter_edit_mode(DepositionId(96))
        .await
        .expect("reused draft");
    assert_eq!(reused.id, DepositionId(96));
    assert!(!reused.is_published());

    let edited = client
        .enter_edit_mode(DepositionId(97))
        .await
        .expect("immediately editable");
    assert_eq!(edited.id, DepositionId(97));
    assert!(!edited.is_published());
}

#[tokio::test]
async fn enter_edit_mode_can_follow_latest_draft_link() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        json!({
            "id": 98,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/98")
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/98/actions/edit",
        StatusCode::CREATED,
        json!({
            "id": 98,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/98"),
                "latest_draft": server.url("deposit/depositions/99")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/98",
        StatusCode::OK,
        json!({
            "id": 98,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/98"),
                "latest_draft": server.url("deposit/depositions/99")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/99",
        StatusCode::OK,
        json!({
            "id": 99,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {
                "self": server.url("deposit/depositions/99"),
                "bucket": server.url("files/bucket-99")
            }
        }),
    );

    let draft = client
        .enter_edit_mode(DepositionId(98))
        .await
        .expect("latest draft edit mode");
    assert_eq!(draft.id, DepositionId(99));
    assert!(!draft.is_published());
}

#[tokio::test]
async fn action_methods_and_record_helpers_cover_remaining_public_branches() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/5/actions/edit",
        StatusCode::CREATED,
        deposition_json(&server, 5, false, "inprogress"),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/5/actions/discard",
        StatusCode::CREATED,
        deposition_json(&server, 5, false, "inprogress"),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/5/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 5,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/6")
            }
        }),
    );

    let record_100 = json!({
        "id": 100,
        "recid": 100,
        "doi": "10.5281/zenodo.100",
        "metadata": { "title": "Record 100" },
        "files": [{
            "id": "f-100",
            "key": "artifact.bin",
            "size": 5,
            "links": { "self": server.url("download/100/artifact.bin") }
        }],
        "links": {
            "latest": server.url("records/101")
        }
    });
    let record_101 = json!({
        "id": 101,
        "recid": 101,
        "doi": "10.5281/zenodo.101",
        "metadata": { "title": "Record 101" },
        "files": [{
            "id": "f-101",
            "key": "artifact.bin",
            "size": 6,
            "links": { "self": server.url("download/101/artifact.bin") }
        }],
        "links": {}
    });

    server.enqueue_json(
        Method::GET,
        "/api/records/100",
        StatusCode::OK,
        record_100.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/101",
        StatusCode::OK,
        record_101.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/100",
        StatusCode::OK,
        record_100.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [record_100.clone()], "total": 1 },
            "links": {}
        }),
    );
    server.enqueue_json(Method::GET, "/api/records/100", StatusCode::OK, record_100);
    server.enqueue_json(Method::GET, "/api/records/101", StatusCode::OK, record_101);

    client.edit(DepositionId(5)).await.expect("edit");
    client.discard(DepositionId(5)).await.expect("discard");
    let new_version = client
        .new_version(DepositionId(5))
        .await
        .expect("new version");
    assert!(new_version.latest_draft_url().is_some());

    let latest = client
        .get_latest_record(RecordId(100))
        .await
        .expect("get latest record");
    assert_eq!(latest.id, RecordId(101));

    let files = client
        .list_record_files(RecordId(100))
        .await
        .expect("list record files");
    assert_eq!(files.len(), 1);

    let doi = Doi::new("10.5281/zenodo.100").expect("doi");
    let info = client
        .get_artifact_info_by_doi(&doi)
        .await
        .expect("artifact info by doi");
    assert_eq!(info.record.id, RecordId(100));
    assert_eq!(info.latest.id, RecordId(101));
    assert!(info.files_by_key.contains_key("artifact.bin"));
}

#[tokio::test]
async fn action_methods_tolerate_empty_success_bodies_by_refreshing_depositions() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue(
        Method::POST,
        "/api/deposit/depositions/205/actions/discard",
        QueuedResponse::bytes(StatusCode::NO_CONTENT, vec![], vec![]),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/205",
        StatusCode::OK,
        json!({
            "id": 205,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );

    let deposition = client
        .discard(DepositionId(205))
        .await
        .expect("discard refresh");
    assert_eq!(deposition.id, DepositionId(205));
    assert!(deposition.is_published());

    let paths: Vec<_> = server
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/deposit/depositions/205/actions/discard",
            "/api/deposit/depositions/205"
        ]
    );
}

#[tokio::test]
async fn download_helpers_cover_remaining_branches() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");

    let record_110 = json!({
        "id": 110,
        "recid": 110,
        "doi": "10.5281/zenodo.110",
        "metadata": { "title": "Record 110" },
        "files": [{
            "id": "f-110",
            "key": "artifact.bin",
            "size": 5,
            "links": { "self": server.url("download/110/artifact.bin") }
        }],
        "links": {
            "latest": server.url("records/111"),
            "archive": server.url("download/110/archive.zip")
        }
    });
    let record_111 = json!({
        "id": 111,
        "recid": 111,
        "doi": "10.5281/zenodo.111",
        "metadata": { "title": "Record 111" },
        "files": [{
            "id": "f-111",
            "key": "artifact.bin",
            "size": 7,
            "links": { "self": server.url("download/111/artifact.bin") }
        }],
        "links": {
            "archive": server.url("download/111/archive.zip")
        }
    });

    server.enqueue_json(
        Method::GET,
        "/api/records/110",
        StatusCode::OK,
        record_110.clone(),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/111",
        StatusCode::OK,
        record_111.clone(),
    );
    server.enqueue(
        Method::GET,
        "/api/download/111/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"latest!".to_vec()),
    );

    server.enqueue_json(
        Method::GET,
        "/api/records/110",
        StatusCode::OK,
        record_110.clone(),
    );
    server.enqueue(
        Method::GET,
        "/api/download/110/archive.zip",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"archive".to_vec()),
    );

    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [record_110.clone()], "total": 1 },
            "links": {}
        }),
    );
    server.enqueue(
        Method::GET,
        "/api/download/110/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"doi-v1".to_vec()),
    );

    server.enqueue_json(Method::GET, "/api/records/110", StatusCode::OK, record_110);
    server.enqueue(
        Method::GET,
        "/api/download/110/artifact.bin",
        QueuedResponse::bytes(StatusCode::OK, vec![], b"selector".to_vec()),
    );

    let latest_path = dir.path().join("latest.bin");
    let archive_path = dir.path().join("archive.zip");
    let doi_path = dir.path().join("doi-v1.bin");
    let selector_path = dir.path().join("selector.bin");

    let latest = client
        .download_latest_record_file_by_key_to_path(RecordId(110), "artifact.bin", &latest_path)
        .await
        .expect("latest by record id");
    assert_eq!(latest.resolved_record, RecordId(111));
    assert_eq!(
        std::fs::read(&latest_path).expect("latest file"),
        b"latest!"
    );

    let archive = client
        .download_record_archive_to_path(RecordId(110), &archive_path)
        .await
        .expect("record archive");
    assert_eq!(archive.resolved_record, RecordId(110));
    assert_eq!(std::fs::read(&archive_path).expect("archive"), b"archive");

    let doi = Doi::new("10.5281/zenodo.110").expect("doi");
    let resolved = client
        .download_file_by_doi_to_path(&doi, "artifact.bin", false, &doi_path)
        .await
        .expect("doi latest false");
    assert_eq!(resolved.resolved_record, RecordId(110));
    assert_eq!(std::fs::read(&doi_path).expect("doi file"), b"doi-v1");

    let selected = client
        .download_artifact(
            &ArtifactSelector::FileByKey {
                record: RecordSelector::RecordId(RecordId(110)),
                key: "artifact.bin".into(),
                latest: false,
            },
            &selector_path,
        )
        .await
        .expect("selector download");
    assert_eq!(selected.resolved_key.as_deref(), Some("artifact.bin"));
    assert_eq!(
        std::fs::read(&selector_path).expect("selector file"),
        b"selector"
    );
}

#[tokio::test]
async fn workflow_error_paths_and_reader_uploads_are_exercised() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/200",
        StatusCode::OK,
        json!({
            "id": 200,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/200/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 200,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );

    let missing_latest = client.ensure_editable_draft(DepositionId(200)).await;
    assert!(matches!(
        missing_latest,
        Err(ZenodoError::MissingLink("latest_draft"))
    ));

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/201",
        StatusCode::OK,
        json!({
            "id": 201,
            "submitted": false,
            "state": "inprogress",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    let no_bucket = client
        .replace_all_files(
            &serde_json::from_value(json!({
                "id": 201,
                "submitted": false,
                "state": "inprogress",
                "metadata": {},
                "files": [],
                "links": {}
            }))
            .unwrap(),
            Vec::<UploadSpec>::new(),
        )
        .await;
    assert!(matches!(no_bucket, Err(ZenodoError::MissingLink("bucket"))));

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/202",
        StatusCode::OK,
        deposition_json(&server, 202, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/files/bucket-202/reader.bin",
        StatusCode::OK,
        json!({ "key": "reader.bin", "size": 6 }),
    );
    let uploaded = client
        .replace_all_files(
            &serde_json::from_value(deposition_json(&server, 202, false, "inprogress")).unwrap(),
            vec![UploadSpec::from_reader(
                "reader.bin",
                Cursor::new(b"reader".to_vec()),
                6,
                mime::APPLICATION_OCTET_STREAM,
            )],
        )
        .await
        .expect("reader upload");
    assert_eq!(uploaded.len(), 1);

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/203",
        StatusCode::OK,
        deposition_json(&server, 203, false, "inprogress"),
    );
    server.enqueue_json(
        Method::PUT,
        "/api/deposit/depositions/203",
        StatusCode::OK,
        deposition_json(&server, 203, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/203",
        StatusCode::OK,
        deposition_json(&server, 203, false, "inprogress"),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/203/actions/publish",
        StatusCode::ACCEPTED,
        deposition_json(&server, 203, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/203",
        StatusCode::OK,
        deposition_json(&server, 203, false, "inprogress"),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/203",
        StatusCode::OK,
        json!({
            "id": 203,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    let missing_record_id = client
        .publish_dataset(
            DepositionId(203),
            &metadata_update(),
            Vec::<UploadSpec>::new(),
        )
        .await;
    assert!(matches!(
        missing_record_id,
        Err(ZenodoError::InvalidState(_))
    ));
}

#[tokio::test]
async fn invalid_json_responses_surface_as_json_errors() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue(
        Method::GET,
        "/api/records/300",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![("content-type".into(), "application/json".into())],
            b"{not-json".to_vec(),
        ),
    );
    server.enqueue_json(
        Method::GET,
        "/api/records",
        StatusCode::OK,
        json!({
            "hits": { "hits": [], "total": 0 },
            "links": {}
        }),
    );

    let invalid_json = client.get_record(RecordId(300)).await;
    assert!(matches!(invalid_json, Err(ZenodoError::Json(_))));

    let missing = client
        .get_record_by_doi(&Doi::new("10.5281/zenodo.missing").expect("doi"))
        .await;
    assert!(matches!(missing, Err(ZenodoError::UnsupportedSelector(_))));
}

#[tokio::test]
async fn mocked_http_error_cases_propagate_through_delete_and_download() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::DELETE,
        "/api/deposit/depositions/400/files/bad",
        StatusCode::CONFLICT,
        json!({ "message": "cannot delete" }),
    );
    server.enqueue_text(
        Method::GET,
        "/api/download/401/artifact.bin",
        StatusCode::NOT_FOUND,
        "missing",
    );
    server.enqueue_json(
        Method::GET,
        "/api/records/401",
        StatusCode::OK,
        json!({
            "id": 401,
            "recid": 401,
            "metadata": { "title": "Broken file" },
            "files": [{
                "id": "f-401",
                "key": "artifact.bin",
                "size": 1,
                "links": { "self": server.url("download/401/artifact.bin") }
            }],
            "links": {}
        }),
    );

    let delete = client
        .delete_file(DepositionId(400), "bad".into())
        .await
        .expect_err("delete should fail");
    assert!(matches!(
        delete,
        ZenodoError::Http {
            status: StatusCode::CONFLICT,
            ..
        }
    ));

    let open = match client
        .open_artifact(&ArtifactSelector::FileByKey {
            record: RecordSelector::RecordId(RecordId(401)),
            key: "artifact.bin".into(),
            latest: false,
        })
        .await
    {
        Ok(_) => panic!("open artifact should fail"),
        Err(error) => error,
    };
    assert!(matches!(
        open,
        ZenodoError::Http {
            status: StatusCode::NOT_FOUND,
            ..
        }
    ));
}

#[tokio::test]
async fn mocked_download_errors_cover_missing_file_and_missing_links() {
    let server = MockZenodoServer::start().await;
    let client = server.client();
    let dir = tempdir().expect("tempdir");

    server.enqueue_json(
        Method::GET,
        "/api/records/410",
        StatusCode::OK,
        json!({
            "id": 410,
            "recid": 410,
            "metadata": { "title": "No file" },
            "files": [],
            "links": {}
        }),
    );
    let missing_file = client
        .download_record_file_by_key_to_path(
            RecordId(410),
            "artifact.bin",
            &dir.path().join("missing.bin"),
        )
        .await
        .expect_err("missing file should fail");
    assert!(matches!(missing_file, ZenodoError::MissingFile { .. }));

    server.enqueue_json(
        Method::GET,
        "/api/records/411",
        StatusCode::OK,
        json!({
            "id": 411,
            "recid": 411,
            "metadata": { "title": "No archive" },
            "files": [],
            "links": {}
        }),
    );
    let missing_archive = client
        .download_record_archive_to_path(RecordId(411), &dir.path().join("archive.zip"))
        .await
        .expect_err("missing archive should fail");
    assert!(matches!(
        missing_archive,
        ZenodoError::MissingLink("archive")
    ));
}

#[tokio::test]
async fn mocked_workflow_errors_cover_non_retryable_and_timeout_paths() {
    let server = MockZenodoServer::start().await;
    let client = server.client();

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/420",
        StatusCode::OK,
        json!({
            "id": 420,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/420/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 420,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/421")
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/421",
        StatusCode::BAD_REQUEST,
        json!({ "message": "bad draft lookup" }),
    );
    let non_retryable = client
        .ensure_editable_draft(DepositionId(420))
        .await
        .expect_err("non-retryable error expected");
    assert!(matches!(
        non_retryable,
        ZenodoError::Http {
            status: StatusCode::BAD_REQUEST,
            ..
        }
    ));

    server.enqueue_json(
        Method::GET,
        "/api/deposit/depositions/422",
        StatusCode::OK,
        json!({
            "id": 422,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/api/deposit/depositions/422/actions/newversion",
        StatusCode::CREATED,
        json!({
            "id": 422,
            "submitted": true,
            "state": "done",
            "metadata": {},
            "files": [],
            "links": {
                "latest_draft": server.url("deposit/depositions/423")
            }
        }),
    );
    for _ in 0..32 {
        server.enqueue_text(
            Method::GET,
            "/api/deposit/depositions/423",
            StatusCode::TOO_MANY_REQUESTS,
            "slow down",
        );
    }
    let timeout = client
        .ensure_editable_draft(DepositionId(422))
        .await
        .expect_err("timeout expected");
    assert!(matches!(timeout, ZenodoError::Timeout("latest draft")));
}

#[tokio::test]
async fn truncated_error_bodies_become_transport_errors() {
    let base = spawn_raw_server(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 10\r\nContent-Type: text/plain\r\n\r\nabc",
    )
    .await;
    let client = ZenodoClient::builder(Auth::new("token"))
        .endpoint(Endpoint::Custom(base))
        .poll_options(PollOptions {
            max_wait: std::time::Duration::from_millis(20),
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(2),
        })
        .build()
        .expect("build raw client");

    let error = client
        .get_deposition(DepositionId(430))
        .await
        .expect_err("truncated body should fail");
    assert!(matches!(error, ZenodoError::Transport(_)));
}
