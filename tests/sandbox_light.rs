#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

mod support;

use zenodo_rs::{ArtifactSelector, FileReplacePolicy, RecordSelector, UploadSpec, ZenodoError};

use crate::support::{
    download_path, draft_deposition_id, live_client, metadata, path_upload, published_record_doi,
    reader_upload, unique_suffix,
};

#[tokio::test]
#[ignore = "requires ZENODO_SANDBOX_TOKEN, ZENODO_SANDBOX_DRAFT_DEPOSITION_ID, and ZENODO_SANDBOX_RECORD_DOI"]
async fn daily_sandbox_smoke_covers_light_draft_and_read_apis() {
    let client = live_client();
    let draft_id = draft_deposition_id();
    let fixture_doi = published_record_doi();

    let original_draft = client
        .get_deposition(draft_id)
        .await
        .expect("fetch managed sandbox draft");
    assert!(
        !original_draft.is_published(),
        "ZENODO_SANDBOX_DRAFT_DEPOSITION_ID must point to an unpublished draft"
    );

    let run_suffix = unique_suffix("daily");
    let stable_filename = "zenodo-rs-daily-state.txt";
    let unique_a = format!("daily-{run_suffix}-a.txt");
    let unique_b = format!("daily-{run_suffix}-b.bin");

    let (_a_dir, upload_a) = path_upload(&unique_a, b"draft smoke v1");
    let upload_a_replacement = UploadSpec::from_reader(
        unique_a.clone(),
        std::io::Cursor::new(b"draft smoke v2".to_vec()),
        14,
        mime::APPLICATION_OCTET_STREAM,
    );
    let upload_b = reader_upload(&unique_b, b"draft smoke extra");
    let (_stable_dir, stable_upload) = path_upload(stable_filename, b"stable daily state");

    let refreshed_draft = client
        .update_metadata(draft_id, &metadata("zenodo-rs daily draft smoke", None))
        .await
        .expect("update managed draft metadata");

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

    client
        .replace_all_files(&refreshed_draft, vec![stable_upload])
        .await
        .expect("restore managed draft to one stable file");

    let final_files = client
        .list_files(draft_id)
        .await
        .expect("list final managed draft files");
    assert_eq!(final_files.len(), 1);
    assert_eq!(final_files[0].filename, stable_filename);

    let record = client
        .get_record_by_doi_str(&fixture_doi)
        .await
        .expect("resolve configured sandbox fixture DOI");
    let record_by_id = client
        .get_record(record.id)
        .await
        .expect("fetch record by id");
    assert_eq!(record_by_id.id, record.id);

    let latest_by_doi = client
        .resolve_latest_by_doi_str(&fixture_doi)
        .await
        .expect("resolve latest by DOI");
    let latest_by_id = client
        .get_latest_record(record.id)
        .await
        .expect("resolve latest by record id");
    assert_eq!(latest_by_doi.id, latest_by_id.id);

    let artifact_info = client
        .get_artifact_info(latest_by_id.id)
        .await
        .expect("load artifact info");
    assert_eq!(artifact_info.latest.id, latest_by_id.id);
    assert!(!artifact_info.files_by_key.is_empty());

    let record_files = client
        .list_record_files(latest_by_id.id)
        .await
        .expect("list record files");
    let first_key = record_files
        .first()
        .map(|file| file.key.clone())
        .expect("configured fixture record must expose at least one file");

    let (_file_dir, file_path) = download_path(&first_key);
    let file_download = client
        .download_file_by_doi_to_path(
            latest_by_doi.doi.as_ref().expect("latest record DOI"),
            &first_key,
            true,
            &file_path,
        )
        .await
        .expect("download named record file by DOI");
    assert_eq!(file_download.resolved_record, latest_by_id.id);
    assert_eq!(
        std::fs::metadata(&file_path)
            .expect("downloaded file")
            .len(),
        file_download.bytes_written
    );

    let (_archive_dir, archive_path) = download_path("fixture.zip");
    let archive_download = client
        .download_record_archive_to_path(latest_by_id.id, &archive_path)
        .await
        .expect("download archive by record id");
    assert_eq!(archive_download.resolved_record, latest_by_id.id);
    assert!(
        std::fs::metadata(&archive_path)
            .expect("downloaded archive")
            .len()
            > 0
    );

    let (_selector_dir, selector_path) = download_path("selector.bin");
    let selector_download = client
        .download_artifact(
            &ArtifactSelector::latest_file(RecordSelector::record_id(record.id), first_key.clone()),
            &selector_path,
        )
        .await
        .expect("download file through high-level selector");
    assert_eq!(selector_download.resolved_record, latest_by_id.id);
    assert_eq!(
        std::fs::metadata(&selector_path)
            .expect("selector download")
            .len(),
        selector_download.bytes_written
    );

    let missing = client
        .download_artifact(
            &ArtifactSelector::file(record.id, format!("missing-{run_suffix}.bin")),
            &selector_path,
        )
        .await
        .expect_err("missing files should surface as client errors");
    assert!(matches!(missing, ZenodoError::MissingFile { .. }));
}
