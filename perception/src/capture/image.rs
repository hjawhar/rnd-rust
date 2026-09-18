//! Single-image frame source.
//!
//! Reads one image from disk via OpenCV and yields it exactly once.

use opencv::imgcodecs;
use opencv::prelude::MatTraitConst;

use crate::error::{PerceptionError, Result};
use crate::types::Frame;

use super::{mat_to_frame, FrameSource};

/// A [`FrameSource`] that yields a single frame from an image file.
pub struct ImageSource {
    /// The frame is consumed on first call to `next_frame`.
    frame: Option<Frame>,
}

impl ImageSource {
    /// Open and decode an image file at `path`.
    ///
    /// The image is decoded immediately and stored as an owned [`Frame`].
    /// Returns an error if the file cannot be read or decoded.
    pub fn open(path: &str) -> Result<Self> {
        let mat = imgcodecs::imread(path, imgcodecs::IMREAD_COLOR).map_err(|e| {
            PerceptionError::Capture(format!("failed to read image '{path}': {e}"))
        })?;

        if mat.empty() {
            return Err(PerceptionError::Capture(format!(
                "imread returned empty Mat for '{path}' — file may not exist or is not a valid image"
            )));
        }

        let frame = mat_to_frame(&mat, 0)?;
        Ok(Self { frame: Some(frame) })
    }
}

impl FrameSource for ImageSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        Ok(self.frame.take())
    }

    /// Images have no inherent frame rate.
    fn fps(&self) -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_nonexistent_image_is_error() {
        let result = ImageSource::open("/tmp/__perception_nonexistent_test_image.png");
        assert!(result.is_err());
    }

    #[test]
    fn yields_none_after_first_frame() {
        // Construct an ImageSource with a pre-built frame (bypasses imread).
        let frame = Frame {
            data: vec![0u8; 3 * 2 * 3],
            width: 3,
            height: 2,
            channels: 3,
            frame_number: 0,
            timestamp: chrono::Utc::now(),
        };
        let mut source = ImageSource {
            frame: Some(frame),
        };

        let first = source.next_frame().unwrap();
        assert!(first.is_some());

        let second = source.next_frame().unwrap();
        assert!(second.is_none());
    }
}
