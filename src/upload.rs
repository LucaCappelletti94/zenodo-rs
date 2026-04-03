//! Upload input types and file replacement policies.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use mime::Mime;

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
    /// Builds an upload spec from a local path and MIME-type guess.
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
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "path has no final file name segment",
                )
            })?;

        Ok(Self {
            content_type: mime_guess::from_path(&path).first_or_octet_stream(),
            filename,
            source: UploadSource::Path(path),
        })
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{UploadSource, UploadSpec};

    #[test]
    fn path_upload_uses_filename_and_mime_guess() {
        let spec = UploadSpec::from_path(PathBuf::from("/tmp/archive.tar.gz")).unwrap();
        assert_eq!(spec.filename, "archive.tar.gz");
        assert_eq!(spec.content_type.as_ref(), "application/gzip");
    }

    #[test]
    fn path_upload_rejects_missing_filename() {
        let error = UploadSpec::from_path(PathBuf::from("/")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
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
}
