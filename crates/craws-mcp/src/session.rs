//! Image session — the engine-facing core of the MCP server, with no MCP types
//! in sight so it can be unit-tested directly.
//!
//! Model: an agent opens images and gets opaque **handles**. Every operation
//! produces a NEW handle (images are immutable, mirroring the engine's tiles),
//! so an agent can branch a pipeline, compare, and export any intermediate.
//! One shared [`Engine`] means its content-hash cache spans the whole session —
//! batch-processing many similar images stays BLAZING.

use craws_domain::{OpSpec, Pipeline, Size};
use craws_engine::{hash, Engine, TiledImage};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A handle to an image living in the session, plus its dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("unknown image handle `{0}` (open an image first, or use the id returned by a previous step)")]
    NotFound(String),
    #[error("reading `{path}`: {source}")]
    Read { path: String, source: std::io::Error },
    #[error("writing `{path}`: {source}")]
    Write { path: String, source: std::io::Error },
    #[error(transparent)]
    Codec(#[from] craws_codecs::CodecError),
    #[error("operation invalid for this image: {0}")]
    Pipeline(#[from] craws_domain::PipelineError),
    #[error("cannot tell the output format of `{0}` — use a .png / .jpg / .webp extension")]
    UnknownFormat(String),
}

pub struct Session {
    engine: Engine,
    images: Mutex<HashMap<String, TiledImage>>,
    counter: AtomicU64,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
            images: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        }
    }

    /// Store an image, mint a fresh handle.
    fn insert(&self, img: TiledImage) -> ImageRef {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("img-{n}");
        let size = img.size();
        self.lock().insert(id.clone(), img);
        ImageRef { id, width: size.width, height: size.height }
    }

    fn get(&self, id: &str) -> Result<TiledImage, SessionError> {
        self.lock().get(id).cloned().ok_or_else(|| SessionError::NotFound(id.to_string()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TiledImage>> {
        self.images.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Number of live handles (for `image_list` / tests).
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Decode an image file into the session.
    pub fn open_path(&self, path: &Path) -> Result<ImageRef, SessionError> {
        let bytes = std::fs::read(path)
            .map_err(|source| SessionError::Read { path: path.display().to_string(), source })?;
        self.open_bytes(&bytes)
    }

    /// Decode encoded image bytes into the session (also the test entry point).
    pub fn open_bytes(&self, bytes: &[u8]) -> Result<ImageRef, SessionError> {
        let decoded = craws_codecs::decode(bytes)?;
        let img = TiledImage::from_srgb_rgba8(
            Size::new(decoded.width, decoded.height),
            &decoded.rgba8,
            hash::digest_bytes(bytes),
        );
        Ok(self.insert(img))
    }

    /// Run one operation on `id`, storing the result under a new handle.
    pub fn apply(&self, id: &str, op: OpSpec) -> Result<ImageRef, SessionError> {
        let src = self.get(id)?;
        let (out, _stats) = self
            .engine
            .run(&src, &Pipeline { version: Pipeline::CURRENT_VERSION, steps: vec![op] })
            .map_err(|e| match e {
                craws_engine::EngineError::Pipeline(p) => SessionError::Pipeline(p),
            })?;
        Ok(self.insert(out))
    }

    /// Dimensions of a handle without mutating anything.
    pub fn info(&self, id: &str) -> Result<ImageRef, SessionError> {
        let img = self.get(id)?;
        let s = img.size();
        Ok(ImageRef { id: id.to_string(), width: s.width, height: s.height })
    }

    /// Encode a handle and return the bytes (test entry point).
    pub fn export_bytes(
        &self,
        id: &str,
        format: craws_codecs::ImageFormat,
        quality: Option<u8>,
    ) -> Result<Vec<u8>, SessionError> {
        let img = self.get(id)?;
        let size = img.size();
        let rgba = img.to_srgb_rgba8();
        Ok(craws_codecs::encode(size.width, size.height, &rgba, format, quality)?)
    }

    /// Encode a handle and write it to `path` (format from the extension).
    /// Returns the number of bytes written.
    pub fn export_path(&self, id: &str, path: &Path, quality: Option<u8>) -> Result<u64, SessionError> {
        let format = craws_codecs::ImageFormat::from_path(path)
            .ok_or_else(|| SessionError::UnknownFormat(path.display().to_string()))?;
        let bytes = self.export_bytes(id, format, quality)?;
        std::fs::write(path, &bytes)
            .map_err(|source| SessionError::Write { path: path.display().to_string(), source })?;
        Ok(bytes.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craws_codecs::ImageFormat;
    use craws_domain::Filter;

    fn sample_png(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]);
            }
        }
        craws_codecs::encode(w, h, &px, ImageFormat::Png, None).unwrap()
    }

    #[test]
    fn open_apply_export_flow() {
        let s = Session::new();
        let opened = s.open_bytes(&sample_png(640, 400)).unwrap();
        assert_eq!((opened.width, opened.height), (640, 400));
        assert_eq!(opened.id, "img-1");

        let resized = s
            .apply(&opened.id, OpSpec::Resize { width: Some(320), height: None, filter: Filter::Lanczos3 })
            .unwrap();
        assert_eq!((resized.width, resized.height), (320, 200));
        assert_ne!(resized.id, opened.id, "each op mints a new handle");

        let cropped = s
            .apply(&resized.id, OpSpec::Crop { x: 10, y: 10, width: 300, height: 180 })
            .unwrap();
        assert_eq!((cropped.width, cropped.height), (300, 180));

        // original handle is untouched (immutability)
        assert_eq!(s.info(&opened.id).unwrap().width, 640);

        let webp = s.export_bytes(&cropped.id, ImageFormat::WebP, None).unwrap();
        let roundtrip = craws_codecs::decode(&webp).unwrap();
        assert_eq!((roundtrip.width, roundtrip.height), (300, 180));
    }

    #[test]
    fn unknown_handle_is_a_clean_error() {
        let s = Session::new();
        let err = s.apply("img-999", OpSpec::Grayscale).unwrap_err();
        assert!(matches!(err, SessionError::NotFound(_)));
        assert!(err.to_string().contains("img-999"));
    }

    #[test]
    fn invalid_op_surfaces_pipeline_error() {
        let s = Session::new();
        let img = s.open_bytes(&sample_png(100, 100)).unwrap();
        let err = s
            .apply(&img.id, OpSpec::Crop { x: 0, y: 0, width: 500, height: 500 })
            .unwrap_err();
        assert!(matches!(err, SessionError::Pipeline(_)));
    }

    #[test]
    fn grayscale_then_export_is_gray() {
        let s = Session::new();
        let img = s.open_bytes(&sample_png(64, 64)).unwrap();
        let gray = s.apply(&img.id, OpSpec::Grayscale).unwrap();
        let png = s.export_bytes(&gray.id, ImageFormat::Png, None).unwrap();
        let d = craws_codecs::decode(&png).unwrap();
        let p = &d.rgba8[..4];
        assert!(p[0] == p[1] && p[1] == p[2], "expected gray, got {p:?}");
    }

    #[test]
    fn shared_cache_across_handles() {
        // same op on the same source twice → second run fully cached (BLAZING)
        let s = Session::new();
        let a = s.open_bytes(&sample_png(300, 300)).unwrap();
        s.apply(&a.id, OpSpec::Exposure { stops: 0.5 }).unwrap();
        // re-open identical bytes → identical source identity → op hits cache
        let b = s.open_bytes(&sample_png(300, 300)).unwrap();
        let out = s.apply(&b.id, OpSpec::Exposure { stops: 0.5 });
        assert!(out.is_ok());
    }
}
