#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

mod support;

use std::io::Cursor;

use zenodo_rs::{ArtifactSelector, DepositState, FileReplacePolicy, UploadSpec, ZenodoError};

use crate::support::{
    download_path, live_client, metadata, path_upload, reader_upload, unique_suffix,
    wait_for_latest_by_doi,
};

#[tokio::test]
#[ignore = "requires ZENODO_SANDBOX_TOKEN and live sandbox publishing permissions"]
async fn daily_sandbox_smoke_covers_full_live_api_surface() {
    let client = live_client();
    let run_suffix = unique_suffix("daily");

    let initial_draft = client
        .create_deposition()
        .await
        .expect("create empty deposition");
    let draft_id = initial_draft.id;

    let original_draft = client
        .get_deposition(draft_id)
        .await
        .expect("fetch newly created draft");
    assert!(
        !original_draft.is_published(),
        "newly created depositions must start unpublished"
    );

    let unique_a = format!("daily-{run_suffix}-a.txt");
    let unique_b = format!("daily-{run_suffix}-b.bin");
    let (_a_dir, upload_a) = path_upload(&unique_a, b"draft smoke v1");
    let upload_a_replacement = UploadSpec::from_reader(
        unique_a.clone(),
        Cursor::new(b"draft smoke v2".to_vec()),
        14,
        mime::APPLICATION_OCTET_STREAM,
    );
    let upload_b = reader_upload(&unique_b, b"draft smoke extra");

    let refreshed_draft = client
        .update_metadata(
            draft_id,
            &metadata("zenodo-rs daily draft smoke", Some("0.0.1")),
        )
        .await
        .expect("update draft metadata");

    client
        .reconcile_files(
            &refreshed_draft,
            FileReplacePolicy::ReplaceAll,
            vec![upload_a],
        )
        .await
        .expect("replace all draft files");

    let files_after_replace = client
        .list_files(draft_id)
        .await
        .expect("list files after replace-all");
    assert_eq!(files_after_replace.len(), 1);
    assert_eq!(files_after_replace[0].filename, unique_a);

    client
        .reconcile_files(
            &refreshed_draft,
            FileReplacePolicy::UpsertByFilename,
            vec![upload_a_replacement],
        )
        .await
        .expect("upsert one existing filename");

    client
        .reconcile_files(
            &refreshed_draft,
            FileReplacePolicy::KeepExistingAndAdd,
            vec![upload_b],
        )
        .await
        .expect("add alongside existing draft files");

    let files_after_add = client
        .list_files(draft_id)
        .await
        .expect("list files after keep-existing-and-add");
    assert!(files_after_add.iter().any(|file| file.filename == unique_a));
    let extra_file = files_after_add
        .iter()
        .find(|file| file.filename == unique_b)
        .expect("extra uploaded file");

    client
        .delete_file(draft_id, extra_file.id.clone())
        .await
        .expect("delete extra draft file");

    let files_after_delete = client
        .list_files(draft_id)
        .await
        .expect("list files after delete");
    assert_eq!(files_after_delete.len(), 1);
    assert_eq!(files_after_delete[0].filename, unique_a);

    let (_v1_dir, v1_upload) = path_upload("payload.txt", format!("v1 {run_suffix}\n").as_bytes());
    let first_published = client
        .publish_dataset_with_policy(
            draft_id,
            &metadata("zenodo-rs daily live smoke", Some("1.0.0")),
            FileReplacePolicy::ReplaceAll,
            vec![v1_upload],
        )
        .await
        .expect("publish initial version");

    assert!(first_published.deposition.is_published());
    let first_record = first_published.record.clone();
    let version_doi = first_record.doi.clone().expect("published DOI");

    let record_by_id = client
        .get_record(first_record.id)
        .await
        .expect("fetch first published record by id");
    assert_eq!(record_by_id.id, first_record.id);

    let editable = client
        .enter_edit_mode(first_published.deposition.id)
        .await
        .expect("enter edit mode on published deposition");
    assert!(
        editable.status.state == DepositState::InProgress,
        "edit mode should expose an editable deposition state"
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
            &metadata("zenodo-rs daily live smoke", Some("2.0.0")),
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

    let latest_by_doi = wait_for_latest_by_doi(&client, &version_doi, latest_record.id).await;
    let latest_by_id = client
        .get_latest_record(first_record.id)
        .await
        .expect("resolve latest by original record id");
    assert_eq!(latest_by_doi.id, latest_record.id);
    assert_eq!(latest_by_id.id, latest_record.id);

    let record_by_latest_doi = client
        .get_record_by_doi_str(version_doi.as_str())
        .await
        .expect("resolve original DOI through records search");
    assert_eq!(record_by_latest_doi.id, first_record.id);

    let versions = client
        .list_record_versions(latest_record.id)
        .await
        .expect("list versions for latest record family");
    assert!(
        versions.hits.len() >= 2,
        "published lineage should expose at least two versions"
    );

    let latest_files = client
        .list_record_files(latest_record.id)
        .await
        .expect("list latest record files");
    assert!(
        latest_files.iter().any(|file| file.key == "payload.txt"),
        "latest record should expose the expected payload file"
    );

    let artifact_info = client
        .get_artifact_info(latest_record.id)
        .await
        .expect("artifact info for latest record");
    assert_eq!(artifact_info.latest.id, latest_record.id);
    assert!(artifact_info.files_by_key.contains_key("payload.txt"));

    let artifact_info_by_doi = client
        .get_artifact_info_by_doi(&version_doi)
        .await
        .expect("artifact info from original DOI");
    assert_eq!(artifact_info_by_doi.latest.id, latest_record.id);

    let (_latest_dir, latest_path) = download_path("payload-latest.txt");
    let latest_download = client
        .download_latest_record_file_by_key_to_path(first_record.id, "payload.txt", &latest_path)
        .await
        .expect("download latest payload through record-id helper");
    assert_eq!(latest_download.resolved_record, latest_record.id);
    let latest_payload = std::fs::read_to_string(&latest_path).expect("read payload download");
    assert_eq!(latest_payload, format!("v2 {run_suffix}\n"));

    let (_doi_dir, doi_path) = download_path("payload-doi.txt");
    let doi_download = client
        .download_file_by_doi_to_path(
            latest_record.doi.as_ref().expect("latest record DOI"),
            "payload.txt",
            true,
            &doi_path,
        )
        .await
        .expect("download latest payload through DOI helper");
    assert_eq!(doi_download.resolved_record, latest_record.id);

    let (_record_dir, record_path) = download_path("payload-id.txt");
    let record_download = client
        .download_record_file_by_key_to_path(latest_record.id, "payload.txt", &record_path)
        .await
        .expect("download named file through record-id helper");
    assert_eq!(record_download.resolved_record, latest_record.id);

    let (_selector_dir, selector_path) = download_path("selector.txt");
    let selector_download = client
        .download_artifact(
            &ArtifactSelector::latest_file_by_doi(version_doi.as_str(), "payload.txt")
                .expect("selector from DOI"),
            &selector_path,
        )
        .await
        .expect("download latest payload through high-level selector");
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

    let missing = client
        .download_artifact(
            &ArtifactSelector::file(latest_record.id, format!("missing-{run_suffix}.bin")),
            &selector_path,
        )
        .await
        .expect_err("missing files should surface as client errors");
    assert!(matches!(missing, ZenodoError::MissingFile { .. }));
}
