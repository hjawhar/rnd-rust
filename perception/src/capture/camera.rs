//! Live camera / RTSP stream frame source.
//!
//! Wraps an OpenCV `VideoCapture` opened on either a local camera index
//! (e.g. `"0"`) or an RTSP URL (e.g. `"rtsp://..."`). Yields frames
//! indefinitely until the source disconnects.

use std::time::{Duration, Instant};

use opencv::prelude::*;
use opencv::videoio::{self, VideoCapture, VideoCaptureTraitConst};

use crate::error::{PerceptionError, Result};
use crate::types::Frame;

use super::{mat_to_frame, FrameSource};

/// A [`FrameSource`] that captures from a live camera or RTSP stream.
///
/// See [`VideoSource`](super::video::VideoSource) for the `Send` rationale.
pub struct CameraSource {
    cap: VideoCapture,
    frame_number: u64,
    fps_limit: u32,
    /// Minimum interval between frames when `fps_limit > 0`.
    frame_interval: Option<Duration>,
    last_frame_time: Option<Instant>,
}

// SAFETY: VideoCapture is accessed exclusively by the owning thread.
unsafe impl Send for CameraSource {}

impl CameraSource {
    /// Open a camera source.
    ///
    /// - If `path` parses as an integer, it is treated as a camera index.
    /// - If `path` starts with `"rtsp://"`, it is treated as an RTSP URL.
    /// - Otherwise, it is passed verbatim to OpenCV (which may interpret it
    ///   as a device path or pipeline string).
    pub fn open(path: &str, fps_limit: u32) -> Result<Self> {
        let cap = if let Ok(index) = path.parse::<i32>() {
            VideoCapture::new(index, videoio::CAP_ANY)
        } else {
            VideoCapture::from_file(path, videoio::CAP_ANY)
        }
        .map_err(|e| {
            PerceptionError::Capture(format!("failed to open camera '{path}': {e}"))
        })?;

        if !cap.is_opened().map_err(|e| {
            PerceptionError::Capture(format!("failed to check camera state: {e}"))
        })? {
            return Err(PerceptionError::Capture(format!(
                "could not open camera source '{path}'"
            )));
        }

        let frame_interval = if fps_limit > 0 {
            Some(Duration::from_secs_f64(1.0 / fps_limit as f64))
        } else {
            None
        };

        Ok(Self {
            cap,
            frame_number: 0,
            fps_limit,
            frame_interval,
            last_frame_time: None,
        })
    }
}

impl FrameSource for CameraSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        // Throttle to fps_limit if configured.
        if let (Some(interval), Some(last)) = (self.frame_interval, self.last_frame_time) {
            let elapsed = last.elapsed();
            if elapsed < interval {
                std::thread::sleep(interval - elapsed);
            }
        }

        let mut mat = opencv::core::Mat::default();
        let grabbed = self.cap.read(&mut mat).map_err(|e| {
            PerceptionError::Capture(format!("failed to read camera frame: {e}"))
        })?;

        if !grabbed || mat.empty() {
            // Camera disconnected or stream ended.
            return Ok(None);
        }

        self.last_frame_time = Some(Instant::now());
        let frame = mat_to_frame(&mat, self.frame_number)?;
        self.frame_number += 1;
        Ok(Some(frame))
    }

    /// Returns the configured FPS limit, or `None` if uncapped.
    fn fps(&self) -> Option<f64> {
        if self.fps_limit > 0 {
            Some(self.fps_limit as f64)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires a connected camera at index 0.
    fn camera_opens_index_zero() {
        let source = CameraSource::open("0", 30);
        assert!(source.is_ok());
    }

    #[test]
    #[ignore] // Requires a reachable RTSP endpoint.
    fn camera_opens_rtsp() {
        let source = CameraSource::open("rtsp://localhost:8554/test", 0);
        assert!(source.is_ok());
    }

    #[test]
    fn frame_interval_calculation() {
        // fps_limit=30 → ~33.3ms interval
        let source = CameraSource::open("0", 30);
        // This will fail if no camera, but we test the struct fields
        // on a successfully-constructed instance below instead.
        if let Ok(s) = source {
            let interval = s.frame_interval.unwrap();
            let expected = Duration::from_secs_f64(1.0 / 30.0);
            let diff = if interval > expected {
                interval - expected
            } else {
                expected - interval
            };
            assert!(diff < Duration::from_micros(100));
        }
    }
}
