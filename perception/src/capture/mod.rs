//! Frame capture abstraction over images, video files, and cameras.
//!
//! Each source implements [`FrameSource`] and is constructed via [`create_source`]
//! based on the `source` field in [`CaptureConfig`].

pub mod camera;
pub mod image;
pub mod video;

use chrono::Utc;
use opencv::core::Mat;
use opencv::prelude::{MatTraitConst, MatTraitConstManual};

use crate::config::CaptureConfig;
use crate::error::{PerceptionError, Result};
use crate::types::Frame;

/// A source of video frames that can be polled sequentially.
pub trait FrameSource: Send {
    /// Returns the next frame, or `None` if the source is exhausted.
    fn next_frame(&mut self) -> Result<Option<Frame>>;

    /// Source FPS if known (from file metadata or camera config).
    fn fps(&self) -> Option<f64>;
}

/// Create a [`FrameSource`] based on the capture configuration.
///
/// Dispatches on `config.source`:
/// - `"image"` — single image file
/// - `"video"` — video file
/// - `"camera"` or `"rtsp"` — live camera / RTSP stream
pub fn create_source(config: &CaptureConfig) -> Result<Box<dyn FrameSource>> {
    match config.source.as_str() {
        "image" => Ok(Box::new(image::ImageSource::open(&config.path)?)),
        "video" => Ok(Box::new(video::VideoSource::open(&config.path)?)),
        "camera" | "rtsp" => Ok(Box::new(camera::CameraSource::open(
            &config.path,
            config.fps_limit,
        )?)),
        other => Err(PerceptionError::Capture(format!(
            "unknown capture source type: '{other}'"
        ))),
    }
}

/// Convert an OpenCV [`Mat`] to an owned [`Frame`].
///
/// Copies the pixel data out of the `Mat` so the resulting `Frame` is `Send`.
/// Used by both video and camera sources.
pub(crate) fn mat_to_frame(mat: &Mat, frame_number: u64) -> Result<Frame> {
    if mat.empty() {
        return Err(PerceptionError::Capture(
            "received empty Mat from capture source".into(),
        ));
    }

    let rows = mat.rows() as u32;
    let cols = mat.cols() as u32;
    let channels = mat.channels() as u32;

    let data = mat
        .data_bytes()
        .map_err(|e| PerceptionError::Capture(format!("failed to read Mat data: {e}")))?
        .to_vec();

    Ok(Frame {
        data,
        width: cols,
        height: rows,
        channels,
        frame_number,
        timestamp: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CaptureConfig;

    #[test]
    fn factory_rejects_unknown_source() {
        let config = CaptureConfig {
            source: "webcam_turbo".into(),
            path: String::new(),
            fps_limit: 0,
        };
        let result = create_source(&config);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("unknown capture source type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn mat_to_frame_rejects_empty_mat() {
        let mat = Mat::default();
        let err = mat_to_frame(&mat, 0).unwrap_err();
        assert!(
            err.to_string().contains("empty Mat"),
            "unexpected error: {err}"
        );
    }
}
