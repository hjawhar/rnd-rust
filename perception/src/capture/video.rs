//! Video file frame source.
//!
//! Wraps an OpenCV `VideoCapture` on a file and yields frames sequentially
//! until EOF.

use opencv::prelude::*;
use opencv::videoio::{self, VideoCapture, VideoCaptureTraitConst};

use crate::error::{PerceptionError, Result};
use crate::types::Frame;

use super::{mat_to_frame, FrameSource};

/// A [`FrameSource`] that reads frames from a video file.
///
/// The underlying `VideoCapture` is **not** `Send`, so this struct is also
/// not automatically `Send`. We satisfy the `FrameSource: Send` bound via an
/// unsafe impl because the `VideoCapture` is only accessed from the owning
/// thread — callers must not move it across threads while a read is in progress.
pub struct VideoSource {
    cap: VideoCapture,
    frame_number: u64,
    fps_value: Option<f64>,
}

// SAFETY: VideoCapture is accessed exclusively by the owning thread.
// The struct is moved to a single consumer thread at creation and never shared.
unsafe impl Send for VideoSource {}

impl VideoSource {
    /// Open a video file at `path`.
    pub fn open(path: &str) -> Result<Self> {
        let cap = VideoCapture::from_file(path, videoio::CAP_ANY).map_err(|e| {
            PerceptionError::Capture(format!("failed to open video '{path}': {e}"))
        })?;

        if !cap.is_opened().map_err(|e| {
            PerceptionError::Capture(format!("failed to check video state: {e}"))
        })? {
            return Err(PerceptionError::Capture(format!(
                "could not open video file '{path}'"
            )));
        }

        let fps_raw = cap.get(videoio::CAP_PROP_FPS).unwrap_or(0.0);
        let fps_value = if fps_raw > 0.0 { Some(fps_raw) } else { None };

        Ok(Self {
            cap,
            frame_number: 0,
            fps_value,
        })
    }
}

impl FrameSource for VideoSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        let mut mat = opencv::core::Mat::default();
        let grabbed = self.cap.read(&mut mat).map_err(|e| {
            PerceptionError::Capture(format!("failed to read video frame: {e}"))
        })?;

        if !grabbed || mat.empty() {
            return Ok(None);
        }

        let frame = mat_to_frame(&mat, self.frame_number)?;
        self.frame_number += 1;
        Ok(Some(frame))
    }

    fn fps(&self) -> Option<f64> {
        self.fps_value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_nonexistent_video_is_error() {
        let result = VideoSource::open("/tmp/__perception_nonexistent_test_video.mp4");
        assert!(result.is_err());
    }
}
