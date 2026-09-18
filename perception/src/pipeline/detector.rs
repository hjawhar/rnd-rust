//! YOLO26 object detection with letterbox preprocessing and NMS-free postprocessing.

use std::sync::Arc;

use ndarray::{s, Array4, ArrayD};
use tracing::debug;

use crate::config::DetectionConfig;
use crate::engine::Engine;
use crate::error::{PerceptionError, Result};
use crate::types::{BBox, Detection, DetectionKind, Frame};

/// COCO dataset class labels (80 classes).
pub const COCO_LABELS: [&str; 80] = [
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck", "boat",
    "traffic light", "fire hydrant", "stop sign", "parking meter", "bench", "bird", "cat",
    "dog", "horse", "sheep", "cow", "elephant", "bear", "zebra", "giraffe", "backpack",
    "umbrella", "handbag", "tie", "suitcase", "frisbee", "skis", "snowboard", "sports ball",
    "kite", "baseball bat", "baseball glove", "skateboard", "surfboard", "tennis racket",
    "bottle", "wine glass", "cup", "fork", "knife", "spoon", "bowl", "banana", "apple",
    "sandwich", "orange", "broccoli", "carrot", "hot dog", "pizza", "donut", "cake", "chair",
    "couch", "potted plant", "bed", "dining table", "toilet", "tv", "laptop", "mouse", "remote",
    "keyboard", "cell phone", "microwave", "oven", "toaster", "sink", "refrigerator", "book",
    "clock", "vase", "scissors", "teddy bear", "hair drier", "toothbrush",
];

/// Metadata from letterbox resize, used to map detections back to original coordinates.
#[derive(Debug, Clone, Copy)]
pub struct LetterboxInfo {
    /// Uniform scale factor applied to the original image.
    pub scale: f32,
    /// Horizontal padding added (pixels in resized space).
    pub pad_x: f32,
    /// Vertical padding added (pixels in resized space).
    pub pad_y: f32,
}

/// YOLO26 object detector with configurable confidence threshold and class filtering.
pub struct YoloDetector {
    engine: Arc<Engine>,
    /// Target input dimensions (width, height).
    input_size: (u32, u32),
    confidence_threshold: f32,
    /// Class name filter. Empty means accept all classes.
    class_filter: Vec<String>,
}

impl YoloDetector {
    /// Creates a new YOLO detector.
    ///
    /// `config.classes` controls which COCO classes are emitted (empty = all).
    /// `confidence` sets the minimum score for a detection to be returned.
    pub fn new(engine: Arc<Engine>, config: &DetectionConfig, confidence: f32) -> Self {
        Self {
            engine,
            input_size: (640, 640),
            confidence_threshold: confidence,
            class_filter: config.classes.clone(),
        }
    }

    /// Preprocesses a frame for YOLO inference.
    ///
    /// 1. Letterbox resize to `input_size` preserving aspect ratio (gray padding)
    /// 2. BGR → RGB
    /// 3. Normalize to \[0, 1\]
    /// 4. HWC → CHW
    /// 5. Add batch dimension → \[1, 3, H, W\]
    pub fn preprocess(&self, frame: &Frame) -> Result<(Array4<f32>, LetterboxInfo)> {
        preprocess(frame, self.input_size)
    }

    /// Maps raw YOLO output back to detections in original image coordinates.
    ///
    /// Output shape: `[1, N, 4+num_classes]` where each row is
    /// `[x_center, y_center, w, h, class_scores...]`.
    pub fn postprocess(
        &self,
        output: &ArrayD<f32>,
        info: &LetterboxInfo,
        orig_w: u32,
        orig_h: u32,
    ) -> Vec<Detection> {
        postprocess(
            output,
            info,
            orig_w,
            orig_h,
            self.confidence_threshold,
            &self.class_filter,
        )
    }

    /// Runs full detection pipeline: preprocess → inference → postprocess.
    pub fn detect(&self, frame: &Frame) -> Result<Vec<Detection>> {
        let (tensor, info) = self.preprocess(frame)?;
        let input_ref = ort::value::TensorRef::from_array_view(tensor.view())
            .map_err(|e| PerceptionError::Inference(format!("failed to build input: {e}")))?;

        let outputs = self.engine.run(
            ort::inputs!["images" => input_ref],
        )?;

        let output_tensor = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| PerceptionError::Inference(format!("failed to extract output: {e}")))?;

        let output = output_tensor.into_owned().into_dyn();

        Ok(self.postprocess(&output, &info, frame.width, frame.height))
    }
}

/// Letterbox-resize and normalize a frame into a `[1, 3, H, W]` tensor.
fn preprocess(frame: &Frame, input_size: (u32, u32)) -> Result<(Array4<f32>, LetterboxInfo)> {
    let (iw, ih) = (input_size.0 as f32, input_size.1 as f32);
    let (fw, fh) = (frame.width as f32, frame.height as f32);

    let scale = (iw / fw).min(ih / fh);
    let new_w = (fw * scale).round();
    let new_h = (fh * scale).round();
    let pad_x = (iw - new_w) / 2.0;
    let pad_y = (ih - new_h) / 2.0;

    let info = LetterboxInfo { scale, pad_x, pad_y };

    let new_w_u = new_w as u32;
    let new_h_u = new_h as u32;
    let iw_u = input_size.0 as usize;
    let ih_u = input_size.1 as usize;
    let pad_x_u = pad_x as usize;
    let pad_y_u = pad_y as usize;

    // Start with gray (114, 114, 114) fill
    let mut letterboxed = vec![114u8; ih_u * iw_u * 3];

    // Nearest-neighbor resize + BGR→RGB swap
    for y in 0..new_h_u {
        for x in 0..new_w_u {
            let src_x = ((x as f32) / scale).min(fw - 1.0) as u32;
            let src_y = ((y as f32) / scale).min(fh - 1.0) as u32;

            let src_idx = ((src_y * frame.width + src_x) * frame.channels) as usize;
            let dst_x = x as usize + pad_x_u;
            let dst_y = y as usize + pad_y_u;
            let dst_idx = (dst_y * iw_u + dst_x) * 3;

            if src_idx + 2 < frame.data.len() && dst_idx + 2 < letterboxed.len() {
                // BGR -> RGB
                letterboxed[dst_idx] = frame.data[src_idx + 2];
                letterboxed[dst_idx + 1] = frame.data[src_idx + 1];
                letterboxed[dst_idx + 2] = frame.data[src_idx];
            }
        }
    }

    // Build [1, 3, H, W] tensor normalized to [0, 1]
    let mut tensor = Array4::<f32>::zeros((1, 3, ih_u, iw_u));
    for y in 0..ih_u {
        for x in 0..iw_u {
            let idx = (y * iw_u + x) * 3;
            tensor[[0, 0, y, x]] = letterboxed[idx] as f32 / 255.0;
            tensor[[0, 1, y, x]] = letterboxed[idx + 1] as f32 / 255.0;
            tensor[[0, 2, y, x]] = letterboxed[idx + 2] as f32 / 255.0;
        }
    }

    Ok((tensor, info))
}

/// Decode YOLO output tensor into detections in original image coordinates.
fn postprocess(
    output: &ArrayD<f32>,
    info: &LetterboxInfo,
    orig_w: u32,
    orig_h: u32,
    confidence_threshold: f32,
    class_filter: &[String],
) -> Vec<Detection> {
    // YOLO11/v8 exports output as [1, C, N] where C = 4+num_classes and N = num_detections.
    // Standard format is [1, N, C]. We detect the transposed format by checking:
    // - C is small (4 + num_classes, typically 84 for COCO)
    // - N is large (number of anchors, typically 8400 for 640x640)
    // If dim1 looks like a feature count (<=200) and dim2 is much larger, transpose.
    let output_2d = match output.ndim() {
        3 => {
            let (d1, d2) = (output.shape()[1], output.shape()[2]);
            let is_transposed = d1 < d2 && d1 <= 200 && d2 >= d1 * 10;
            if is_transposed {
                debug!(shape = ?output.shape(), "transposing YOLO output from [1,C,N] to [N,C]");
                let view_3d = output.to_shape((1, d1, d2)).unwrap();
                view_3d.slice(s![0, .., ..]).t().to_owned()
            } else {
                output.to_shape((d1, d2)).unwrap().to_owned()
            }
        }
        2 => output.to_shape((output.shape()[0], output.shape()[1])).unwrap().to_owned(),
        _ => return Vec::new(),
    };

    let n = output_2d.shape()[0];
    let cols = output_2d.shape()[1];

    if cols < 5 {
        return Vec::new();
    }
    let num_classes = cols - 4;

    let mut detections = Vec::new();

    for i in 0..n {
        let row = output_2d.slice(s![i, ..]);

        // Find best class score
        let class_scores = row.slice(s![4..]);
        let (class_id, &max_score) = class_scores
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((0, &0.0));

        if max_score < confidence_threshold {
            continue;
        }

        // xywh center-format -> xyxy in letterboxed space
        let cx = row[0];
        let cy = row[1];
        let w = row[2];
        let h = row[3];

        // Map to original image space and clamp
        let x1 = ((cx - w / 2.0 - info.pad_x) / info.scale).clamp(0.0, orig_w as f32);
        let y1 = ((cy - h / 2.0 - info.pad_y) / info.scale).clamp(0.0, orig_h as f32);
        let x2 = ((cx + w / 2.0 - info.pad_x) / info.scale).clamp(0.0, orig_w as f32);
        let y2 = ((cy + h / 2.0 - info.pad_y) / info.scale).clamp(0.0, orig_h as f32);

        let label = if class_id < num_classes.min(COCO_LABELS.len()) {
            COCO_LABELS[class_id].to_string()
        } else {
            format!("class_{class_id}")
        };

        // Apply class filter
        if !class_filter.is_empty() && !class_filter.iter().any(|c| c == &label) {
            continue;
        }

        detections.push(Detection {
            bbox: BBox::new(x1, y1, x2, y2),
            confidence: max_score,
            class_id: class_id as u32,
            label,
            kind: DetectionKind::Object,
        });
    }

    debug!(count = detections.len(), "postprocess complete");
    detections
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_frame(width: u32, height: u32, bgr: [u8; 3]) -> Frame {
        let channels = 3u32;
        let mut data = Vec::with_capacity((width * height * channels) as usize);
        for _ in 0..(width * height) {
            data.extend_from_slice(&bgr);
        }
        Frame {
            data,
            width,
            height,
            channels,
            frame_number: 0,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_letterbox_square() {
        // 640x480 landscape -> fits width, pads top/bottom
        let frame = make_frame(640, 480, [100, 150, 200]);
        let (tensor, info) = preprocess(&frame, (640, 640)).unwrap();

        assert_eq!(tensor.shape(), &[1, 3, 640, 640]);
        // scale = min(640/640, 640/480) = 1.0
        assert!((info.scale - 1.0).abs() < 1e-4, "scale={}", info.scale);
        assert!((info.pad_x).abs() < 1e-4, "pad_x={}", info.pad_x);
        // pad_y = (640 - 480) / 2 = 80
        assert!((info.pad_y - 80.0).abs() < 1e-4, "pad_y={}", info.pad_y);
    }

    #[test]
    fn test_letterbox_tall() {
        // 480x640 portrait -> fits height, pads left/right
        let frame = make_frame(480, 640, [100, 150, 200]);
        let (tensor, info) = preprocess(&frame, (640, 640)).unwrap();

        assert_eq!(tensor.shape(), &[1, 3, 640, 640]);
        assert!((info.scale - 1.0).abs() < 1e-4, "scale={}", info.scale);
        // pad_x = (640 - 480) / 2 = 80
        assert!((info.pad_x - 80.0).abs() < 1e-4, "pad_x={}", info.pad_x);
        assert!((info.pad_y).abs() < 1e-4, "pad_y={}", info.pad_y);
    }

    #[test]
    fn test_letterbox_already_square() {
        let frame = make_frame(640, 640, [100, 150, 200]);
        let (tensor, info) = preprocess(&frame, (640, 640)).unwrap();

        assert_eq!(tensor.shape(), &[1, 3, 640, 640]);
        assert!((info.scale - 1.0).abs() < 1e-4);
        assert!((info.pad_x).abs() < 1e-4);
        assert!((info.pad_y).abs() < 1e-4);
    }

    #[test]
    fn test_normalize() {
        // BGR [0, 128, 255] -> RGB [255, 128, 0] -> normalized [1.0, ~0.502, 0.0]
        let frame = make_frame(640, 640, [0, 128, 255]);
        let (tensor, _) = preprocess(&frame, (640, 640)).unwrap();

        let r = tensor[[0, 0, 320, 320]];
        let g = tensor[[0, 1, 320, 320]];
        let b = tensor[[0, 2, 320, 320]];

        assert!((r - 1.0).abs() < 1e-3, "R={r}, expected 1.0");
        assert!((g - 128.0 / 255.0).abs() < 1e-3, "G={g}, expected ~0.502");
        assert!(b.abs() < 1e-3, "B={b}, expected 0.0");
    }

    #[test]
    fn test_postprocess_threshold() {
        let info = LetterboxInfo { scale: 1.0, pad_x: 0.0, pad_y: 0.0 };

        // det 0: score 0.9 (above 0.7), det 1: score 0.3 (below 0.7)
        let data = ndarray::arr3(&[[
            [320.0_f32, 240.0, 100.0, 80.0, 0.9],
            [100.0, 100.0, 50.0, 50.0, 0.3],
        ]]);

        let dets = postprocess(&data.into_dyn(), &info, 640, 480, 0.7, &[]);
        assert_eq!(dets.len(), 1);
        assert!((dets[0].confidence - 0.9).abs() < 1e-5);
    }

    #[test]
    fn test_postprocess_coordinate_mapping() {
        // 1280x960 letterboxed to 640x640: scale=0.5, pad_x=0, pad_y=80
        let info = LetterboxInfo { scale: 0.5, pad_x: 0.0, pad_y: 80.0 };

        // Detection at (320, 320) size 100x100 in letterbox space.
        // Original: center_x=(320-0)/0.5=640, center_y=(320-80)/0.5=480
        //           w=100/0.5=200, h=100/0.5=200
        //           -> (540, 380, 740, 580)
        let data = ndarray::arr3(&[[[320.0_f32, 320.0, 100.0, 100.0, 0.95]]]);

        let dets = postprocess(&data.into_dyn(), &info, 1280, 960, 0.1, &[]);
        assert_eq!(dets.len(), 1);

        let b = &dets[0].bbox;
        assert!((b.x1 - 540.0).abs() < 1.0, "x1={}, expected 540", b.x1);
        assert!((b.y1 - 380.0).abs() < 1.0, "y1={}, expected 380", b.y1);
        assert!((b.x2 - 740.0).abs() < 1.0, "x2={}, expected 740", b.x2);
        assert!((b.y2 - 580.0).abs() < 1.0, "y2={}, expected 580", b.y2);
    }

    #[test]
    fn test_class_filter() {
        let info = LetterboxInfo { scale: 1.0, pad_x: 0.0, pad_y: 0.0 };
        let filter = vec!["person".to_string()];

        // det 0: person (class 0, score 0.9), det 1: car (class 2, score 0.8)
        let data = ndarray::arr3(&[[
            [320.0_f32, 240.0, 100.0, 80.0, 0.9, 0.1, 0.05],
            [100.0, 100.0, 50.0, 50.0, 0.1, 0.05, 0.8],
        ]]);

        let dets = postprocess(&data.into_dyn(), &info, 640, 480, 0.1, &filter);
        assert_eq!(dets.len(), 1, "only person should pass filter");
        assert_eq!(dets[0].label, "person");
        assert_eq!(dets[0].class_id, 0);
    }
}
