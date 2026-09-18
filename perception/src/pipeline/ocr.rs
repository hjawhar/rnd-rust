//! Two-stage OCR pipeline: text detection (DBNet) then text recognition (CRNN).
//!
//! Detection produces a probability map thresholded into text-region bounding boxes.
//! Recognition crops each region, batches them into a fixed-height tensor, and
//! decodes output logits via CTC greedy search.

use std::path::Path;
use std::sync::Arc;

use ndarray::{s, Array4};
use opencv::core::{
    self, Mat, Point, Rect, Size, Vector, CV_8UC1,
};
use opencv::imgproc;
use opencv::prelude::*;
use tracing::debug;

use crate::engine::Engine;
use crate::error::{PerceptionError, Result};
use crate::types::{BBox, Frame, OcrResult};

/// ImageNet channel means (RGB order).
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// ImageNet channel standard deviations (RGB order).
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Binary-map threshold for DBNet probability output.
const DB_THRESHOLD: f32 = 0.3;
/// Minimum bounding-box area (in pixels) to keep a text region.
const MIN_BOX_AREA: f32 = 100.0;

/// Fixed height for CRNN recognition input.
const REC_HEIGHT: i32 = 48;
/// Maximum width for CRNN recognition input.
const REC_MAX_WIDTH: i32 = 320;

/// Detection input size (square).
const DET_SIZE: i32 = 640;

/// Two-stage OCR pipeline combining DBNet text detection with CRNN text recognition.
pub struct OcrPipeline {
    detector: Arc<Engine>,
    recognizer: Arc<Engine>,
    /// Character dictionary loaded from keys file. Index 0 is the CTC blank token.
    character_dict: Vec<String>,
    max_batch_size: u32,
}

impl OcrPipeline {
    /// Creates a new OCR pipeline.
    ///
    /// Loads the character dictionary from `keys_path` (one character per line).
    /// A blank token is prepended at index 0 for CTC decoding.
    pub fn new(
        det_engine: Arc<Engine>,
        rec_engine: Arc<Engine>,
        keys_path: &Path,
        max_batch_size: u32,
    ) -> Result<Self> {
        let content = std::fs::read_to_string(keys_path).map_err(|e| PerceptionError::Io {
            path: keys_path.to_path_buf(),
            source: e,
        })?;

        // Index 0 = CTC blank token, then one character per line from the keys file.
        let mut character_dict = vec![String::new()]; // blank at 0
        for line in content.lines() {
            if !line.is_empty() {
                character_dict.push(line.to_string());
            }
        }

        debug!(
            dict_size = character_dict.len(),
            "loaded OCR character dictionary"
        );

        Ok(Self {
            detector: det_engine,
            recognizer: rec_engine,
            character_dict,
            max_batch_size,
        })
    }

    /// Detect text regions in a frame.
    ///
    /// Runs DBNet on the frame and returns axis-aligned bounding boxes for each
    /// detected text region, filtered by minimum area.
    pub fn detect_text(&self, frame: &Frame) -> Result<Vec<BBox>> {
        let (input_tensor, scale_x, scale_y) = preprocess_detection(frame)?;

        let input_ref = ort::value::TensorRef::from_array_view(input_tensor.view())?;
        let outputs = self.detector.run(ort::inputs![input_ref])?;

        let output_view = outputs[0].try_extract_array::<f32>()?;

        // Output shape: [1, 1, H, W] — probability map.
        let shape = output_view.shape();
        let h = shape[2];
        let w = shape[3];

        // Threshold into a binary map and find contours.
        let boxes = postprocess_detection(&output_view, h, w, scale_x, scale_y)?;
        debug!(count = boxes.len(), "detected text regions");
        Ok(boxes)
    }

    /// Recognize text in detected regions. Batches crops for efficiency.
    ///
    /// Each region is cropped from the original frame, preprocessed, and fed
    /// through the CRNN recognizer in batches of `max_batch_size`.
    pub fn recognize_text(&self, frame: &Frame, regions: &[BBox]) -> Result<Vec<OcrResult>> {
        if regions.is_empty() {
            return Ok(Vec::new());
        }

        let src_mat = frame_to_mat(frame)?;
        let mut results = Vec::with_capacity(regions.len());
        let batch_size = self.max_batch_size as usize;

        for chunk in regions.chunks(batch_size) {
            let (batch_tensor, widths) = preprocess_recognition_batch(&src_mat, chunk)?;

            let input_ref = ort::value::TensorRef::from_array_view(batch_tensor.view())?;
            let outputs = self.recognizer.run(ort::inputs![input_ref])?;

            let output_view = outputs[0].try_extract_array::<f32>()?;
            // Output shape: [N, seq_len, num_classes]
            let n = output_view.shape()[0];
            let seq_len = output_view.shape()[1];
            let num_classes = output_view.shape()[2];

            for (i, bbox) in chunk.iter().enumerate() {
                if i >= n {
                    break;
                }
                let logits = output_view.slice(s![i, .., ..]);

                // Argmax along class dimension for each timestep.
                let mut indices = Vec::with_capacity(seq_len);
                for t in 0..seq_len {
                    let row = logits.slice(s![t, ..]);
                    let (max_idx, _) = row
                        .iter()
                        .enumerate()
                        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or((0, &0.0));
                    indices.push(max_idx);
                }

                // Compute mean confidence from max logit values (softmax approximation).
                let confidence = {
                    let mut sum = 0.0f32;
                    let mut count = 0usize;
                    for t in 0..seq_len {
                        let row = logits.slice(s![t, ..]);
                        let max_val = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                        if indices[t] != 0 {
                            // Only count non-blank predictions.
                            sum += max_val;
                            count += 1;
                        }
                    }
                    if count > 0 { sum / count as f32 } else { 0.0 }
                };

                let text = ctc_decode(&indices, &self.character_dict);

                if !text.is_empty() {
                    results.push(OcrResult {
                        text,
                        bbox: bbox.clone(),
                        confidence,
                    });
                }
            }
        }

        debug!(count = results.len(), "recognized text regions");
        Ok(results)
    }

    /// Full pipeline: detect then recognize.
    ///
    /// Runs text detection followed by text recognition on all detected regions.
    pub fn detect_and_recognize(&self, frame: &Frame) -> Result<Vec<OcrResult>> {
        let regions = self.detect_text(frame)?;
        self.recognize_text(frame, &regions)
    }
}

/// CTC greedy decode: collapse consecutive duplicates, remove blank tokens (index 0).
pub(crate) fn ctc_decode(indices: &[usize], dict: &[String]) -> String {
    let mut result = String::new();
    let mut prev: Option<usize> = None;

    for &idx in indices {
        // Skip if same as previous (collapse duplicates).
        if prev == Some(idx) {
            continue;
        }
        prev = Some(idx);

        // Skip blank token.
        if idx == 0 {
            continue;
        }

        if let Some(ch) = dict.get(idx) {
            result.push_str(ch);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert a `Frame` (BGR, u8) to an OpenCV `Mat`.
fn frame_to_mat(frame: &Frame) -> Result<Mat> {
    let mat = unsafe {
        Mat::new_rows_cols_with_data_unsafe_def(
            frame.height as i32,
            frame.width as i32,
            core::CV_8UC3,
            frame.data.as_ptr() as *mut std::ffi::c_void,
        )?
    };
    // Clone to own the data — the original slice must not be aliased.
    Ok(mat.clone())
}

/// ImageNet-normalize a single pixel (BGR u8 → RGB f32 normalized).
pub(crate) fn normalize_imagenet(b: u8, g: u8, r: u8) -> [f32; 3] {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    [
        (rf - IMAGENET_MEAN[0]) / IMAGENET_STD[0],
        (gf - IMAGENET_MEAN[1]) / IMAGENET_STD[1],
        (bf - IMAGENET_MEAN[2]) / IMAGENET_STD[2],
    ]
}

/// Preprocess a frame for DBNet detection.
///
/// Returns (CHW tensor [1,3,H,W], scale_x, scale_y) where scales map detection
/// coordinates back to original frame coordinates.
fn preprocess_detection(frame: &Frame) -> Result<(Array4<f32>, f32, f32)> {
    let src = frame_to_mat(frame)?;
    let mut resized = Mat::default();
    imgproc::resize(
        &src,
        &mut resized,
        Size::new(DET_SIZE, DET_SIZE),
        0.0,
        0.0,
        imgproc::INTER_LINEAR,
    )?;

    let scale_x = frame.width as f32 / DET_SIZE as f32;
    let scale_y = frame.height as f32 / DET_SIZE as f32;

    let h = DET_SIZE as usize;
    let w = DET_SIZE as usize;
    let mut tensor = Array4::<f32>::zeros((1, 3, h, w));

    let data = resized.data_bytes()?;
    for y in 0..h {
        for x in 0..w {
            let offset = (y * w + x) * 3;
            let b = data[offset];
            let g = data[offset + 1];
            let r = data[offset + 2];
            let [rn, gn, bn] = normalize_imagenet(b, g, r);
            tensor[[0, 0, y, x]] = rn;
            tensor[[0, 1, y, x]] = gn;
            tensor[[0, 2, y, x]] = bn;
        }
    }

    Ok((tensor, scale_x, scale_y))
}

/// Postprocess DBNet output: threshold → binary map → contours → bounding boxes.
fn postprocess_detection(
    prob_map: &ndarray::ArrayViewD<'_, f32>,
    h: usize,
    w: usize,
    scale_x: f32,
    scale_y: f32,
) -> Result<Vec<BBox>> {
    // Build a CV_8UC1 binary map from the probability map.
    let mut binary = unsafe { Mat::new_rows_cols(h as i32, w as i32, CV_8UC1)? };

    for y in 0..h {
        for x in 0..w {
            let val = prob_map[[0, 0, y, x]];
            let pixel: u8 = if val > DB_THRESHOLD { 255 } else { 0 };
            *binary.at_2d_mut::<u8>(y as i32, x as i32)? = pixel;
        }
    }

    // Find contours.
    let mut contours: Vector<Vector<Point>> = Vector::new();
    imgproc::find_contours_def(&binary, &mut contours, imgproc::RETR_EXTERNAL, imgproc::CHAIN_APPROX_SIMPLE)?;

    let mut boxes = Vec::new();
    for i in 0..contours.len() {
        let contour = contours.get(i)?;
        let rect = imgproc::bounding_rect(&contour)?;

        // Scale back to original frame coordinates.
        let x1 = rect.x as f32 * scale_x;
        let y1 = rect.y as f32 * scale_y;
        let x2 = (rect.x + rect.width) as f32 * scale_x;
        let y2 = (rect.y + rect.height) as f32 * scale_y;

        let bbox = BBox::new(x1, y1, x2, y2);
        if bbox.area() < MIN_BOX_AREA {
            continue;
        }
        boxes.push(bbox);
    }

    Ok(boxes)
}

/// Preprocess a batch of text-region crops for CRNN recognition.
///
/// Each crop is resized to `REC_HEIGHT` height with aspect-preserving width
/// (capped at `REC_MAX_WIDTH`), then padded to `REC_MAX_WIDTH`.
///
/// Returns (batch tensor [N,3,48,320], actual widths before padding).
fn preprocess_recognition_batch(
    src: &Mat,
    regions: &[BBox],
) -> Result<(Array4<f32>, Vec<i32>)> {
    let n = regions.len();
    let mut batch = Array4::<f32>::zeros((n, 3, REC_HEIGHT as usize, REC_MAX_WIDTH as usize));
    let mut widths = Vec::with_capacity(n);

    let src_h = src.rows();
    let src_w = src.cols();

    for (i, bbox) in regions.iter().enumerate() {
        // Clamp to frame bounds.
        let x1 = (bbox.x1 as i32).max(0).min(src_w - 1);
        let y1 = (bbox.y1 as i32).max(0).min(src_h - 1);
        let x2 = (bbox.x2 as i32).max(x1 + 1).min(src_w);
        let y2 = (bbox.y2 as i32).max(y1 + 1).min(src_h);

        let roi = Rect::new(x1, y1, x2 - x1, y2 - y1);
        let crop = Mat::roi(src, roi)?;

        // Resize to fixed height, maintaining aspect ratio.
        let crop_h = crop.rows() as f32;
        let crop_w = crop.cols() as f32;
        let ratio = REC_HEIGHT as f32 / crop_h;
        let new_w = ((crop_w * ratio).round() as i32).min(REC_MAX_WIDTH).max(1);

        let mut resized = Mat::default();
        imgproc::resize(
            &crop,
            &mut resized,
            Size::new(new_w, REC_HEIGHT),
            0.0,
            0.0,
            imgproc::INTER_LINEAR,
        )?;

        widths.push(new_w);

        // Fill the tensor (zero-padded beyond new_w).
        let data = resized.data_bytes()?;
        for y in 0..REC_HEIGHT as usize {
            for x in 0..new_w as usize {
                let offset = (y * new_w as usize + x) * 3;
                let b = data[offset];
                let g = data[offset + 1];
                let r = data[offset + 2];
                let [rn, gn, bn] = normalize_imagenet(b, g, r);
                batch[[i, 0, y, x]] = rn;
                batch[[i, 1, y, x]] = gn;
                batch[[i, 2, y, x]] = bn;
            }
        }
        // Remaining columns [new_w..REC_MAX_WIDTH] stay at 0.0 from zeros init.
    }

    Ok((batch, widths))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dict() -> Vec<String> {
        // Index 0 = blank, 1 = "a", 2 = "b", 3 = "c"
        vec![
            String::new(),
            "a".into(),
            "b".into(),
            "c".into(),
        ]
    }

    #[test]
    fn test_ctc_decode_basic() {
        let dict = sample_dict();
        // [1,1,2,0,3,3] → collapse: [1,2,0,3] → remove blank: [1,2,3] → "abc"
        let indices = [1, 1, 2, 0, 3, 3];
        assert_eq!(ctc_decode(&indices, &dict), "abc");
    }

    #[test]
    fn test_ctc_decode_all_blanks() {
        let dict = sample_dict();
        let indices = [0, 0, 0];
        assert_eq!(ctc_decode(&indices, &dict), "");
    }

    #[test]
    fn test_ctc_decode_no_repeats() {
        let dict = sample_dict();
        let indices = [1, 2, 3];
        assert_eq!(ctc_decode(&indices, &dict), "abc");
    }

    #[test]
    fn test_text_region_filter_area() {
        // A box with area < 100 should be filtered.
        let small_box = BBox::new(0.0, 0.0, 5.0, 10.0); // area = 50
        assert!(small_box.area() < MIN_BOX_AREA);

        let large_box = BBox::new(0.0, 0.0, 20.0, 10.0); // area = 200
        assert!(large_box.area() >= MIN_BOX_AREA);
    }

    #[test]
    fn test_normalize_imagenet() {
        // Known pixel: BGR (128, 128, 128) → RGB all 128.
        let [r, g, b] = normalize_imagenet(128, 128, 128);

        let val = 128.0 / 255.0;
        let expected_r = (val - IMAGENET_MEAN[0]) / IMAGENET_STD[0];
        let expected_g = (val - IMAGENET_MEAN[1]) / IMAGENET_STD[1];
        let expected_b = (val - IMAGENET_MEAN[2]) / IMAGENET_STD[2];

        assert!((r - expected_r).abs() < 1e-5, "r: got {r}, expected {expected_r}");
        assert!((g - expected_g).abs() < 1e-5, "g: got {g}, expected {expected_g}");
        assert!((b - expected_b).abs() < 1e-5, "b: got {b}, expected {expected_b}");
    }

    #[test]
    fn test_normalize_imagenet_black() {
        let [r, g, b] = normalize_imagenet(0, 0, 0);
        // 0/255 = 0.0 → (0 - mean) / std
        assert!((r - (-IMAGENET_MEAN[0] / IMAGENET_STD[0])).abs() < 1e-5);
        assert!((g - (-IMAGENET_MEAN[1] / IMAGENET_STD[1])).abs() < 1e-5);
        assert!((b - (-IMAGENET_MEAN[2] / IMAGENET_STD[2])).abs() < 1e-5);
    }

    #[test]
    fn test_normalize_imagenet_white() {
        let [r, g, b] = normalize_imagenet(255, 255, 255);
        let expected_r = (1.0 - IMAGENET_MEAN[0]) / IMAGENET_STD[0];
        let expected_g = (1.0 - IMAGENET_MEAN[1]) / IMAGENET_STD[1];
        let expected_b = (1.0 - IMAGENET_MEAN[2]) / IMAGENET_STD[2];
        assert!((r - expected_r).abs() < 1e-5);
        assert!((g - expected_g).abs() < 1e-5);
        assert!((b - expected_b).abs() < 1e-5);
    }

    #[test]
    fn test_batch_padding() {
        // Create a small "frame" that will produce a narrow crop.
        // 100 wide x 48 tall, 3 channels, all pixel value 128.
        let width = 100u32;
        let height = 48u32;
        let data = vec![128u8; (width * height * 3) as usize];

        let mat = unsafe {
            Mat::new_rows_cols_with_data_unsafe_def(
                height as i32,
                width as i32,
                core::CV_8UC3,
                data.as_ptr() as *mut std::ffi::c_void,
            )
            .unwrap()
        };
        let mat = mat.clone();

        // Region covering the full mat — width 100 at height 48, ratio = 48/48 = 1,
        // so new_w = 100, which is < 320.
        let regions = [BBox::new(0.0, 0.0, width as f32, height as f32)];
        let (batch, widths) = preprocess_recognition_batch(&mat, &regions).unwrap();

        assert_eq!(batch.shape(), &[1, 3, 48, 320]);
        assert_eq!(widths[0], 100);

        // Pixel at x=50 (within content) should be non-zero.
        assert!(batch[[0, 0, 24, 50]].abs() > 1e-6);

        // Pixel at x=310 (in padded region) should be zero.
        assert!((batch[[0, 0, 24, 310]]).abs() < 1e-6);
        assert!((batch[[0, 1, 24, 310]]).abs() < 1e-6);
        assert!((batch[[0, 2, 24, 310]]).abs() < 1e-6);
    }

    #[test]
    fn test_ctc_decode_blank_between_same_chars() {
        // "aa" requires a blank separator: [1, 0, 1] → "aa"
        let dict = sample_dict();
        let indices = [1, 0, 1];
        assert_eq!(ctc_decode(&indices, &dict), "aa");
    }

    #[test]
    fn test_ctc_decode_empty_input() {
        let dict = sample_dict();
        assert_eq!(ctc_decode(&[], &dict), "");
    }

    #[test]
    fn test_ctc_decode_out_of_bounds_index() {
        let dict = sample_dict(); // len 4
        // Index 99 is out of bounds — should be silently skipped.
        let indices = [1, 99, 2];
        assert_eq!(ctc_decode(&indices, &dict), "ab");
    }

    #[test]
    #[ignore = "requires real ONNX model files on disk"]
    fn test_detect_and_recognize_integration() {
        // Placeholder for integration testing with real models.
        // Would load real engines, a real keys file, and a real frame.
    }
}
