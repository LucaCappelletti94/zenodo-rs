//! Upload input types and file replacement policies.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use client_uploader_traits::collect_upload_filenames;
use mime::Mime;

use crate::error::ZenodoError;

/// Policy for reconciling existing draft files with new uploads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileReplacePolicy {
    /// Delete all visible draft files before uploading.
    ReplaceAll,
    /// Replace files that share the same filename.
    UpsertByFilename,
    /// Keep existing files and add new uploads alongside them.
    KeepExistingAndAdd,
}

/// Source data for a single upload.
pub enum UploadSource {
    /// Upload from a local file path.
    Path(
        /// Local source path.
        PathBuf,
    ),
    /// Upload from a blocking reader with an explicit content length.
    Reader {
        /// Reader that produces the upload bytes.
        reader: Box<dyn Read + Send>,
        /// Exact number of bytes that the reader will produce.
        content_length: u64,
    },
}

impl fmt::Debug for UploadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Reader { content_length, .. } => f
                .debug_struct("Reader")
                .field("content_length", content_length)
                .finish_non_exhaustive(),
        }
    }
}

/// Specification for one file upload.
#[derive(Debug)]
pub struct UploadSpec {
    /// Filename to expose in Zenodo.
    pub filename: String,
    /// Upload source.
    pub source: UploadSource,
    /// MIME type to send with the upload.
    pub content_type: Mime,
}

impl UploadSpec {
    /// Builds an upload spec from a local path.
    ///
    /// Zenodo bucket uploads commonly expect `application/octet-stream`, so
    /// that is the safe default used here. Callers can still override
    /// [`Self::content_type`] explicitly before upload when needed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use zenodo_rs::UploadSpec;
    ///
    /// let spec = UploadSpec::from_path(PathBuf::from("/tmp/archive.tar.gz"))?;
    /// assert_eq!(spec.filename, "archive.tar.gz");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not contain a final filename segment.
    pub fn from_path(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(path_without_filename_error)?;

        Ok(Self {
            content_type: mime::APPLICATION_OCTET_STREAM,
            filename,
            source: UploadSource::Path(path),
        })
    }

    /// Builds an upload spec from a local path and explicit uploaded filename.
    ///
    /// This is a shorthand for [`Self::from_path`] followed by
    /// [`Self::with_filename`] when you want the local path and archive filename
    /// to differ.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use zenodo_rs::{UploadSource, UploadSpec};
    ///
    /// let spec = UploadSpec::from_path_as(
    ///     PathBuf::from("/tmp/local-name.bin"),
    ///     "archive-name.bin",
    /// )?;
    ///
    /// assert_eq!(spec.filename, "archive-name.bin");
    /// match spec.source {
    ///     UploadSource::Path(path) => assert_eq!(path, PathBuf::from("/tmp/local-name.bin")),
    ///     UploadSource::Reader { .. } => unreachable!("expected path source"),
    /// }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not contain a final filename segment
    /// or if the uploaded filename is empty.
    pub fn from_path_as(
        path: impl Into<PathBuf>,
        filename: impl Into<String>,
    ) -> std::io::Result<Self> {
        let filename = filename.into();
        if filename.is_empty() {
            return Err(empty_upload_filename_error());
        }

        Ok(Self::from_path(path)?.with_filename(filename))
    }

    /// Returns this upload spec with a different uploaded filename.
    ///
    /// # Examples
    ///
    /// ```
    /// use zenodo_rs::UploadSpec;
    ///
    /// let spec = UploadSpec::from_path("/tmp/local-name.bin")?.with_filename("archive-name.bin");
    /// assert_eq!(spec.filename, "archive-name.bin");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = filename.into();
        self
    }

    /// Builds validated upload specs from `(archive_filename, local_path)` pairs.
    ///
    /// This is useful for manifest-driven upload code that already knows the
    /// final archive filenames up front.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use zenodo_rs::{UploadSource, UploadSpec};
    ///
    /// let specs = UploadSpec::from_named_paths([
    ///     ("release.tar.gz", "/tmp/build-output.bin"),
    ///     ("manifest.json", "/tmp/manifest.local.json"),
    /// ])?;
    ///
    /// assert_eq!(specs.len(), 2);
    /// assert_eq!(specs[0].filename, "release.tar.gz");
    /// match &specs[0].source {
    ///     UploadSource::Path(path) => assert_eq!(path, &PathBuf::from("/tmp/build-output.bin")),
    ///     UploadSource::Reader { .. } => unreachable!("expected path source"),
    /// }
    /// # Ok::<(), zenodo_rs::ZenodoError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any local path lacks a final filename segment, if an
    /// uploaded filename is empty, or if multiple entries target the same final
    /// filename.
    pub fn from_named_paths<I, F, P>(entries: I) -> Result<Vec<Self>, ZenodoError>
    where
        I: IntoIterator<Item = (F, P)>,
        F: Into<String>,
        P: Into<PathBuf>,
    {
        let mut specs = Vec::new();
        for (filename, path) in entries {
            specs.push(Self::from_path_as(path, filename)?);
        }

        collect_upload_filenames(specs.iter()).map_err(ZenodoError::from)?;
        Ok(specs)
    }

    /// Returns the exact number of bytes that this upload will send.
    ///
    /// Path-based uploads read the current local file size. Reader-based uploads
    /// return the explicit content length supplied at construction time.
    ///
    /// # Errors
    ///
    /// Returns an error if the source path metadata cannot be read.
    pub fn content_length(&self) -> std::io::Result<u64> {
        match &self.source {
            UploadSource::Path(path) => Ok(std::fs::metadata(path)?.len()),
            UploadSource::Reader { content_length, .. } => Ok(*content_length),
        }
    }

    /// Builds an upload spec from a reader and explicit metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    /// use zenodo_rs::{UploadSource, UploadSpec};
    ///
    /// let spec = UploadSpec::from_reader(
    ///     "artifact.bin",
    ///     Cursor::new(vec![1_u8, 2, 3]),
    ///     3,
    ///     mime::APPLICATION_OCTET_STREAM,
    /// );
    ///
    /// assert_eq!(spec.filename, "artifact.bin");
    /// match spec.source {
    ///     UploadSource::Reader { content_length, .. } => assert_eq!(content_length, 3),
    ///     UploadSource::Path(_) => unreachable!("expected reader source"),
    /// }
    /// ```
    #[must_use]
    pub fn from_reader(
        filename: impl Into<String>,
        reader: impl Read + Send + 'static,
        content_length: u64,
        content_type: Mime,
    ) -> Self {
        Self {
            filename: filename.into(),
            source: UploadSource::Reader {
                reader: Box::new(reader),
                content_length,
            },
            content_type,
        }
    }
}

fn empty_upload_filename_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "upload filename cannot be empty",
    )
}

fn path_without_filename_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "path has no final file name segment",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        empty_upload_filename_error, path_without_filename_error, UploadSource, UploadSpec,
    };
    use crate::error::ZenodoError;

    #[test]
    fn path_upload_defaults_to_octet_stream() {
        let spec = UploadSpec::from_path(PathBuf::from("/tmp/archive.tar.gz")).unwrap();
        assert_eq!(spec.filename, "archive.tar.gz");
        assert_eq!(spec.content_type, mime::APPLICATION_OCTET_STREAM);
    }

    #[test]
    fn path_upload_can_override_uploaded_filename() {
        let spec =
            UploadSpec::from_path_as(PathBuf::from("/tmp/local-name.bin"), "archive-name.bin")
                .unwrap();
        assert_eq!(spec.filename, "archive-name.bin");
        match spec.source {
            UploadSource::Path(path) => assert_eq!(path, PathBuf::from("/tmp/local-name.bin")),
            UploadSource::Reader { .. } => panic!("expected path source"),
        }
    }

    #[test]
    fn with_filename_renames_existing_upload_spec() {
        let spec = UploadSpec::from_path(PathBuf::from("/tmp/local-name.bin"))
            .unwrap()
            .with_filename("archive-name.bin");
        assert_eq!(spec.filename, "archive-name.bin");
        match spec.source {
            UploadSource::Path(path) => assert_eq!(path, PathBuf::from("/tmp/local-name.bin")),
            UploadSource::Reader { .. } => panic!("expected path source"),
        }
    }

    #[test]
    fn path_upload_rejects_missing_filename() {
        let error = UploadSpec::from_path(PathBuf::from("/")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn missing_filename_error_has_stable_message() {
        let error = path_without_filename_error();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "path has no final file name segment");
    }

    #[test]
    fn empty_uploaded_filename_error_has_stable_message() {
        let error = empty_upload_filename_error();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "upload filename cannot be empty");
    }

    #[test]
    fn from_named_paths_rejects_duplicate_archive_names() {
        let error = UploadSpec::from_named_paths([
            ("artifact.bin", "/tmp/one.bin"),
            ("artifact.bin", "/tmp/two.bin"),
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            ZenodoError::DuplicateUploadFilename { filename } if filename == "artifact.bin"
        ));
    }

    #[test]
    fn from_named_paths_preserves_manifest_names_and_paths() {
        let specs = UploadSpec::from_named_paths([
            ("first.bin", "/tmp/a.bin"),
            ("second.bin", "/tmp/b.bin"),
        ])
        .unwrap();

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].filename, "first.bin");
        assert_eq!(specs[1].filename, "second.bin");
        match &specs[0].source {
            UploadSource::Path(path) => assert_eq!(path, &PathBuf::from("/tmp/a.bin")),
            UploadSource::Reader { .. } => panic!("expected path source"),
        }
    }

    #[test]
    fn reader_upload_debug_hides_reader() {
        let spec = UploadSpec::from_reader(
            "artifact.bin",
            std::io::Cursor::new(vec![1, 2, 3]),
            3,
            mime::APPLICATION_OCTET_STREAM,
        );

        match spec.source {
            UploadSource::Reader { content_length, .. } => assert_eq!(content_length, 3),
            UploadSource::Path(_) => panic!("expected reader source"),
        }
        assert!(format!("{spec:?}").contains("artifact.bin"));
    }

    #[test]
    fn path_source_debug_shows_path_variant() {
        let spec = UploadSpec::from_path(PathBuf::from("/tmp/report.txt")).unwrap();
        assert!(format!("{:?}", spec.source).contains("Path"));
        match spec.source {
            UploadSource::Path(path) => assert_eq!(path, PathBuf::from("/tmp/report.txt")),
            UploadSource::Reader { .. } => panic!("expected path source"),
        }
    }

    #[test]
    fn content_length_uses_reader_length() {
        let spec = UploadSpec::from_reader(
            "artifact.bin",
            std::io::Cursor::new(vec![1, 2, 3]),
            3,
            mime::APPLICATION_OCTET_STREAM,
        );

        assert_eq!(spec.content_length().unwrap(), 3);
    }

    #[test]
    fn content_length_reads_path_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.txt");
        std::fs::write(&path, b"hello").unwrap();

        let spec = UploadSpec::from_path(&path).unwrap();
        assert_eq!(spec.content_length().unwrap(), 5);
    }
}
