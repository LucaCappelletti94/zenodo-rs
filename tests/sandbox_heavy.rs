#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

mod support;

use zenodo_rs::{ArtifactSelector, FileReplacePolicy};

use crate::support::{download_path, live_client, metadata, path_upload, unique_suffix};

#[tokio::test]
#[ignore = "requires ZENODO_SANDBOX_TOKEN and live sandbox publishing permissions"]
async fn weekly_sandbox_smoke_covers_publish_versioning_and_downloads() {
    let client = live_client();
    let run_suffix = unique_suffix("weekly");

    let initial_draft = client
        .create_deposition()
        .await
        .expect("create empty deposition");

    let (_v1_dir, v1_upload) = path_upload("payload.txt", format!("v1 {run_suffix}\n").as_bytes());
    let first_published = client
        .publish_dataset_with_policy(
            initial_draft.id,
            &metadata("zenodo-rs weekly publish smoke", Some("1.0.0")),
            FileReplacePolicy::ReplaceAll,
            vec![v1_upload],
        )
        .await
        .expect("publish initial version");

    assert!(first_published.deposition.is_published());
    let first_record = first_published.record.clone();
    let version_doi = first_record.doi.clone().expect("published DOI");

    let editable = client
        .enter_edit_mode(first_published.deposition.id)
        .await
        .expect("enter edit mode on published deposition");
    assert!(
        !editable.is_published(),
        "edit mode should expose an editable draft state"
    );
    let discarded = client
        .discard(editable.id)
        .await
        .expect("discard transient edit mode changes");
    assert!(discarded.is_published());

    let (_v2_dir, v2_upload) = path_upload("payload.txt", format!("v2 {run_suffix}\n").as_bytes());
    let second_published = client
        .publish_dataset_with_policy(
            first_published.deposition.id,
            &metadata("zenodo-rs weekly publish smoke", Some("2.0.0")),
            FileReplacePolicy::UpsertByFilename,
            vec![v2_upload],
        )
        .await
        .expect("publish second version");

    assert!(second_published.deposition.is_published());
    let latest_record = second_published.record.clone();
    assert_ne!(
        latest_record.id, first_record.id,
        "publishing a new version should yield a new record id"
    );

    let resolved_latest = client
        .resolve_latest_by_doi(&version_doi)
        .await
        .expect("resolve latest version from v1 DOI");
    assert_eq!(resolved_latest.id, latest_record.id);

    let versions = client
        .list_record_versions(latest_record.id)
        .await
        .expect("list versions for latest record family");
    assert!(
        versions.hits.len() >= 2,
        "weekly published lineage should expose at least two versions"
    );

    let latest_by_id = client
        .get_latest_record(first_record.id)
        .await
        .expect("resolve latest by original record id");
    assert_eq!(latest_by_id.id, latest_record.id);

    let latest_files = client
        .list_record_files(latest_record.id)
        .await
        .expect("list latest record files");
    assert!(
        latest_files.iter().any(|file| file.key == "payload.txt"),
        "latest record should expose the expected payload file"
    );

    let info = client
        .get_artifact_info(latest_record.id)
        .await
        .expect("artifact info for latest record");
    assert_eq!(info.latest.id, latest_record.id);
    assert!(info.files_by_key.contains_key("payload.txt"));

    let (_download_dir, payload_path) = download_path("payload.txt");
    let payload_download = client
        .download_latest_record_file_by_key_to_path(first_record.id, "payload.txt", &payload_path)
        .await
        .expect("download latest payload through record-id helper");
    assert_eq!(payload_download.resolved_record, latest_record.id);
    let payload = std::fs::read_to_string(&payload_path).expect("read payload download");
    assert_eq!(payload, format!("v2 {run_suffix}\n"));

    let (_selector_dir, selector_path) = download_path("selector.txt");
    let selector_download = client
        .download_artifact(
            &ArtifactSelector::latest_file_by_doi(version_doi.as_str(), "payload.txt")
                .expect("selector from DOI"),
            &selector_path,
        )
        .await
        .expect("download latest payload through DOI selector");
    assert_eq!(selector_download.resolved_record, latest_record.id);

    let (_archive_dir, archive_path) = download_path("latest.zip");
    let archive_download = client
        .download_record_archive_to_path(latest_record.id, &archive_path)
        .await
        .expect("download latest archive");
    assert_eq!(archive_download.resolved_record, latest_record.id);
    assert!(
        std::fs::metadata(&archive_path)
            .expect("archive metadata")
            .len()
            > 0
    );
}
