//! Display-only text previews; file access and document selection belong to the caller.


/// Caller-supplied text and identity, kept separate from local filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocument {
    /// Display name supplied by the Host or caller.
    pub name: String,
    /// Host-side path for display only; never resolved on the desktop filesystem.
    pub host_path: String,
    /// Decoded text, rendered literally without markup or link interpretation.
    pub text: String,
    /// Freshness token from the Host, used to guard a later write.
    pub version: Option<String>,
}

/// Caller-supplied decoded image and identity.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageDocument {
    /// Display name supplied by the Host or caller.
    pub name: String,
    /// Host-side path for display only; never resolved on the desktop filesystem.
    pub host_path: String,
    /// Decoded width and height in pixels.
    pub size: [usize; 2],
    /// Unmultiplied RGBA pixels, `size[0] * size[1] * 4` bytes.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for ImageDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageDocument")
            .field("name", &self.name)
            .field("host_path", &self.host_path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// One preview target decoded from the Host.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewDocument {
    Text(TextDocument),
    Image(ImageDocument),
}
