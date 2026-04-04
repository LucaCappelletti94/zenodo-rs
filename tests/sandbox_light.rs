#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

mod support;

use std::io::Cursor;

use futures_util::StreamExt;
use zenodo_rs::{
    ArtifactSelector, DepositState, FileReplacePolicy, RecordQuery, UploadSpec, ZenodoError,
};

use crate::support::{
    deposition_id_from_url, download_path, live_client, metadata, path_upload, reader_upload, step,
    unique_suffix, wait_for_draft_deposition, wait_for_latest_by_doi,
    wait_for_published_deposition,
};

#[tokio::test]
#[ignore = "requires ZENODO_SANDBOX_TOKEN and live sandbox publishing permissions"]
async fn daily_sandbox_low_level_deposition_api_surface() {
    let _step = step("low-level deposition api surface");
    let client = live_client();
    let run_suffix = unique_suffix("lowlevel");

    let _step = step("create deposition and update metadata");
    let draft = client
        .create_deposition()
        .await
        .expect("create low-level draft");
    let draft = client
        .update_metadata(
            draft.id,
            &metadata("zenodo-rs daily low-level smoke", Some("0.1.0")),
        )
        .await
        .expect("update low-level draft metadata");
    let bucket = draft.bucket_url().cloned().expect("draft bucket url");

    let payload_a = format!("low-level-a-{run_suffix}.txt");
    let payload_b = format!("low-level-b-{run_suffix}.bin");

    let _step = step("upload path and reader through low-level apis");
    let (_a_dir, upload_a) = path_upload(&payload_a, b"low-level path upload");
    let uploaded_a = match upload_a.source {
        zenodo_rs::UploadSource::Path(path) => client
            .upload_path(&bucket, &upload_a.filename, &path)
            .await
            .expect("upload path directly"),
        zenodo_rs::UploadSource::Reader { .. } => panic!("expected path upload"),
    };
    assert_eq!(uploaded_a.key, payload_a);

    let uploaded_b = client
        .upload_reader(
            &bucket,
            &payload_b,
            Cursor::new(b"low-level reader upload".to_vec()),
            23,
            mime::APPLICATION_OCTET_STREAM,
        )
        .await
        .expect("upload reader directly");
    assert_eq!(uploaded_b.key, payload_b);

    let _step = step("list and delete draft files");
    let files = client
        .list_files(draft.id)
        .await
        .expect("list low-level files");
    assert!(files.iter().any(|file| file.filename == payload_a));
    let extra = files
        .iter()
        .find(|file| file.filename == payload_b)
        .expect("uploaded reader file");
    client
        .delete_file(draft.id, extra.id.clone())
        .await
        .expect("delete low-level uploaded file");

    let remaining = client
        .list_files(draft.id)
        .await
        .expect("list remaining files");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].filename, payload_a);

    let _step = step("publish directly and resolve the published record");
    let published = client.publish(draft.id).await.expect("direct publish");
    let published = wait_for_published_deposition(&client, published.id).await;
    let record_id = published.record_id.expect("published record id");
    let first_record = client
        .get_record(record_id)
        .await
        .expect("fetch first low-level published record");

    let _step = step("edit discard and version directly");
    let edited = client.edit(published.id).await.expect("direct edit");
    assert!(
        edited.status.state == DepositState::InProgress || edited.latest_draft_url().is_some(),
        "edit should expose an editable state or a draft link"
    );
    let discarded = client.discard(edited.id).await.expect("direct discard");
    assert!(discarded.is_published());

    let versioned = client
        .new_version(published.id)
        .await
        .expect("direct new version");
    let latest_draft = versioned
        .latest_draft_url()
        .cloned()
        .expect("latest draft url");
    let latest_draft_id = deposition_id_from_url(&latest_draft);
    let latest_draft = wait_for_draft_deposition(&client, latest_draft_id).await;

    let payload_v2 = format!("low-level-v2-{run_suffix}.txt");
    let (_v2_dir, upload_v2) = path_upload(&payload_v2, b"second version");
    let bucket_v2 = latest_draft
        .bucket_url()
        .cloned()
        .expect("new version bucket url");
    match upload_v2.source {
        zenodo_rs::UploadSource::Path(path) => client
            .upload_path(&bucket_v2, &upload_v2.filename, &path)
            .await
            .expect("upload second version path"),
        zenodo_rs::UploadSource::Reader { .. } => panic!("expected path upload"),
    };

    let republished = client
        .publish(latest_draft.id)
        .await
        .expect("publish second version directly");
    let republished = wait_for_published_deposition(&client, republished.id).await;
    let latest_record = client
        .get_latest_record(first_record.id)
        .await
        .expect("resolve latest after direct versioning");
    assert_eq!(
        republished.record_id,
        Some(latest_record.id),
        "republished deposition should point at the latest record"
    );
}

#[tokio::test]
#[ignore = "requires ZENODO_SANDBOX_TOKEN and live sandbox publishing permissions"]
async fn daily_sandbox_workflow_and_read_api_surface() {
    let _step = step("workflow and read api surface");
    let client = live_client();
    let run_suffix = unique_suffix("daily");

    let _step = step("create draft and verify initial state");
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

    let _step = step("exercise all draft file reconciliation policies");
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

    let _step = step("publish first version through workflow helper");
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

    let _step = step("enter edit mode and discard transient changes");
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

    let _step = step("publish second version through policy-aware helper");
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

    let _step = step("exercise latest-resolution and search helpers");
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
    let latest_by_doi_str = client
        .resolve_latest_by_doi_str(version_doi.as_str())
        .await
        .expect("resolve latest by DOI string");
    assert_eq!(latest_by_doi_str.id, latest_record.id);
    let search = client
        .search_records(
            &RecordQuery::builder()
                .query(format!("recid:{}", latest_record.id.0))
                .build(),
        )
        .await
        .expect("search records by recid");
    assert!(
        search
            .hits
            .iter()
            .any(|record| record.id == latest_record.id),
        "search results should include the latest record"
    );

    let _step = step("exercise version listing and artifact info helpers");
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

    let _step = step("exercise download and streaming helpers");
    let mut stream = client
        .open_artifact(&ArtifactSelector::file(
            latest_record.id,
            "payload.txt".to_owned(),
        ))
        .await
        .expect("open artifact stream");
    let mut streamed = Vec::new();
    while let Some(chunk) = stream.stream.next().await {
        streamed.extend_from_slice(&chunk.expect("stream chunk"));
    }
    assert_eq!(streamed, format!("v2 {run_suffix}\n").into_bytes());

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
