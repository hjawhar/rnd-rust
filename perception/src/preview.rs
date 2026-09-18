//! Optional live preview window and video recording.
//!
//! Preview is feature-gated behind the `preview` Cargo feature. `VideoRecorder`
//! writes annotated frames to an MP4 file and is always available.

use std::path::Path;

use opencv::prelude::*;
use opencv::{core, imgproc};

use crate::error::PerceptionError;
use crate::types::{DetectionKind, Frame, TrackedDetection};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build an OpenCV Mat from a Frame's raw BGR bytes (cloned to own data).
fn frame_to_mat(frame: &Frame) -> crate::error::Result<Mat> {
    let mat = unsafe {
        core::Mat::new_rows_cols_with_data_unsafe_def(
            frame.height as i32,
            frame.width as i32,
            core::CV_8UC3,
            frame.data.as_ptr() as *mut std::ffi::c_void,
        )
    }
    .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;
    Ok(mat.clone())
}

/// Draw detection bounding boxes and labels onto a Mat.
fn draw_overlays(display: &mut Mat, detections: &[TrackedDetection]) -> crate::error::Result<()> {
    let black = core::Scalar::new(0.0, 0.0, 0.0, 0.0);

    for td in detections {
        let det = &td.detection;
        let color = match det.kind {
            DetectionKind::Object => core::Scalar::new(50.0, 220.0, 50.0, 0.0),
            DetectionKind::Face => core::Scalar::new(255.0, 100.0, 50.0, 0.0),
            DetectionKind::Text => core::Scalar::new(50.0, 50.0, 255.0, 0.0),
        };

        let rect = core::Rect::new(
            det.bbox.x1 as i32,
            det.bbox.y1 as i32,
            (det.bbox.x2 - det.bbox.x1) as i32,
            (det.bbox.y2 - det.bbox.y1) as i32,
        );

        imgproc::rectangle(display, rect, color, 1, imgproc::LINE_AA, 0)
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        let label = format!(
            "[{}] {} {:.0}%",
            td.track_id,
            det.label,
            det.confidence * 100.0
        );

        let font = imgproc::FONT_HERSHEY_SIMPLEX;
        let font_scale = 0.45;
        let thickness = 1;
        let origin = core::Point::new(det.bbox.x1 as i32, det.bbox.y1 as i32 - 4);

        // Measure text to draw a filled dark background.
        let mut baseline = 0;
        let text_size =
            imgproc::get_text_size(&label, font, font_scale, thickness, &mut baseline)
                .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        let bg_rect = core::Rect::new(
            origin.x,
            origin.y - text_size.height - 2,
            text_size.width + 4,
            text_size.height + baseline + 4,
        );
        imgproc::rectangle(display, bg_rect, black, imgproc::FILLED, imgproc::LINE_8, 0)
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        imgproc::put_text(
            display,
            &label,
            origin,
            font,
            font_scale,
            color,
            thickness,
            imgproc::LINE_AA,
            false,
        )
        .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PreviewWindow
// ---------------------------------------------------------------------------

/// Live preview window for displaying annotated frames.
pub struct PreviewWindow {
    window_name: String,
    #[allow(dead_code)]
    width: u32,
}

impl PreviewWindow {
    /// Create and open a new preview window.
    pub fn new(width: u32) -> crate::error::Result<Self> {
        let window_name = "perception".to_string();

        #[cfg(feature = "preview")]
        {
            opencv::highgui::named_window(&window_name, opencv::highgui::WINDOW_AUTOSIZE)
                .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;
        }

        Ok(Self {
            window_name,
            width,
        })
    }

    /// Display a frame with detection overlays.
    ///
    /// Returns `true` if the window should continue, `false` if ESC was pressed.
    #[cfg(feature = "preview")]
    pub fn show(
        &self,
        frame: &Frame,
        detections: &[TrackedDetection],
    ) -> crate::error::Result<bool> {
        use opencv::highgui;

        let mut display = frame_to_mat(frame)?;
        draw_overlays(&mut display, detections)?;

        highgui::imshow(&self.window_name, &display)
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        let key = highgui::wait_key(1)
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        Ok(key != 27)
    }

    /// Stub when preview feature is not enabled.
    #[cfg(not(feature = "preview"))]
    pub fn show(
        &self,
        _frame: &Frame,
        _detections: &[TrackedDetection],
    ) -> crate::error::Result<bool> {
        Ok(true)
    }
}

impl Drop for PreviewWindow {
    fn drop(&mut self) {
        #[cfg(feature = "preview")]
        {
            let _ = opencv::highgui::destroy_window(&self.window_name);
        }
    }
}

// ---------------------------------------------------------------------------
// VideoRecorder
// ---------------------------------------------------------------------------

/// Records annotated frames to an MP4 video file.
pub struct VideoRecorder {
    writer: opencv::videoio::VideoWriter,
}

impl VideoRecorder {
    /// Create a recorder writing to `output_path` at the given FPS and frame dimensions.
    pub fn new(
        output_path: &Path,
        fps: f64,
        width: i32,
        height: i32,
    ) -> crate::error::Result<Self> {
        use opencv::videoio;

        let fourcc = videoio::VideoWriter::fourcc('m', 'p', '4', 'v')
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        let writer = videoio::VideoWriter::new(
            output_path.to_str().unwrap_or("output.mp4"),
            fourcc,
            fps,
            core::Size::new(width, height),
            true,
        )
        .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;

        if !writer
            .is_opened()
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?
        {
            return Err(PerceptionError::OpenCv(format!(
                "failed to open video writer at {}",
                output_path.display()
            )));
        }

        Ok(Self { writer })
    }

    /// Write an annotated frame to the output video.
    pub fn write_frame(
        &mut self,
        frame: &Frame,
        detections: &[TrackedDetection],
    ) -> crate::error::Result<()> {
        let mut display = frame_to_mat(frame)?;
        draw_overlays(&mut display, detections)?;
        self.writer
            .write(&display)
            .map_err(|e| PerceptionError::OpenCv(e.to_string()))?;
        Ok(())
    }
}
