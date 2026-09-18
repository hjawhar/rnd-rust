//! Two-stage face pipeline: SCRFD detection followed by ArcFace recognition.
//!
//! The pipeline first detects faces in a frame using SCRFD, producing bounding
//! boxes and optional landmarks. Each detected face can then be cropped and
//! passed through ArcFace to extract a 512-dimensional embedding suitable for
//! identity matching via cosine similarity.

use std::sync::Arc;

use tracing::debug;

use crate::engine::Engine;
use crate::error::{PerceptionError, Result};
use crate::types::{BBox, Frame};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single face detection with bounding box, confidence, and optional landmarks.
#[derive(Debug, Clone)]
pub struct FaceDetection {
    /// Bounding box in original-image pixel coordinates.
    pub bbox: BBox,
    /// Detection confidence in [0, 1].
    pub confidence: f32,
    /// Five landmark points as (x, y) pairs: left eye, right eye, nose,
    /// left mouth corner, right mouth corner.
    pub landmarks: Option<[f32; 10]>,
}

/// Two-stage face pipeline: detection (SCRFD) then recognition (ArcFace).
pub struct FacePipeline {
    detector: Arc<Engine>,
    recognizer: Arc<Engine>,
    det_input_size: (u32, u32),
    rec_input_size: (u32, u32),
    confidence_threshold: f32,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Mean subtracted during SCRFD / ArcFace normalization.
const NORM_MEAN: f32 = 127.5;
/// Scale divisor during SCRFD / ArcFace normalization.
const NORM_STD: f32 = 128.0;
/// Default IOU threshold for NMS.
const NMS_IOU_THRESHOLD: f32 = 0.4;
/// Number of floats per SCRFD detection row: x1,y1,x2,y2 + conf + 5×(x,y) landmarks.
const MIN_ROW_STRIDE: usize = 15;

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl FacePipeline {
    /// Create a new face pipeline from pre-loaded detection and recognition engines.
    pub fn new(
        detector: Arc<Engine>,
        recognizer: Arc<Engine>,
        confidence_threshold: f32,
    ) -> Self {
        Self {
            detector,
            recognizer,
            det_input_size: (640, 640),
            rec_input_size: (112, 112),
            confidence_threshold,
        }
    }

    /// Detect faces in a frame, returning bounding boxes and landmarks.
    pub fn detect_faces(&self, frame: &Frame) -> Result<Vec<FaceDetection>> {
        let (input_w, input_h) = self.det_input_size;
        let (tensor_data, scale, pad_x, pad_y) =
            preprocess_letterbox(frame, input_w, input_h)?;

        let shape = [1_usize, 3, input_h as usize, input_w as usize];
        let input_value =
            ort::value::Tensor::from_array((shape, tensor_data.into_boxed_slice()))?;
        let outputs = self.detector.run(ort::inputs![input_value])?;

        let (_, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| PerceptionError::Inference(format!("extract detection tensor: {e}")))?;

        let stride = if !data.is_empty() { determine_stride(data, MIN_ROW_STRIDE) } else { MIN_ROW_STRIDE };
        let num_rows = if stride > 0 { data.len() / stride } else { 0 };

        let mut detections: Vec<(BBox, f32, [f32; 10])> = Vec::new();

        for i in 0..num_rows {
            let off = i * stride;
            if off + MIN_ROW_STRIDE > data.len() {
                break;
            }

            let conf = data[off + 4];
            if conf < self.confidence_threshold {
                continue;
            }

            // Map coordinates from letterbox space back to original image.
            let x1 = ((data[off] - pad_x) / scale).clamp(0.0, frame.width as f32);
            let y1 = ((data[off + 1] - pad_y) / scale).clamp(0.0, frame.height as f32);
            let x2 = ((data[off + 2] - pad_x) / scale).clamp(0.0, frame.width as f32);
            let y2 = ((data[off + 3] - pad_y) / scale).clamp(0.0, frame.height as f32);

            let mut landmarks = [0.0f32; 10];
            for j in 0..5 {
                landmarks[j * 2] = (data[off + 5 + j * 2] - pad_x) / scale;
                landmarks[j * 2 + 1] = (data[off + 5 + j * 2 + 1] - pad_y) / scale;
            }

            detections.push((BBox::new(x1, y1, x2, y2), conf, landmarks));
        }

        nms(&mut detections, NMS_IOU_THRESHOLD);
        debug!(count = detections.len(), "face detections after NMS");

        Ok(detections
            .into_iter()
            .map(|(bbox, confidence, lm)| FaceDetection {
                bbox,
                confidence,
                landmarks: Some(lm),
            })
            .collect())
    }

    /// Extract a 512-dim embedding from a face crop.
    pub fn extract_embedding(
        &self,
        frame: &Frame,
        face: &FaceDetection,
    ) -> Result<Vec<f32>> {
        let (rec_w, rec_h) = self.rec_input_size;
        let crop = crop_and_resize(frame, &face.bbox, rec_w, rec_h)?;
        let tensor_data = normalize_hwc_to_nchw_flat(&crop, rec_w, rec_h);

        let shape = [1_usize, 3, rec_h as usize, rec_w as usize];
        let input_value =
            ort::value::Tensor::from_array((shape, tensor_data.into_boxed_slice()))?;
        let outputs = self.recognizer.run(ort::inputs![input_value])?;

        let (_, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| PerceptionError::Inference(format!("extract embedding tensor: {e}")))?;

        let embedding = l2_normalize(data);
        debug!(dim = embedding.len(), "extracted face embedding");
        Ok(embedding)
    }

    /// Detect faces and extract embeddings for all detected faces.
    pub fn detect_and_recognize(
        &self,
        frame: &Frame,
    ) -> Result<Vec<(FaceDetection, Vec<f32>)>> {
        let faces = self.detect_faces(frame)?;
        if faces.is_empty() {
            return Ok(Vec::new());
        }

        let embeddings = self.extract_embeddings_batch(frame, &faces)?;
        Ok(faces.into_iter().zip(embeddings).collect())
    }

    /// Crop all faces, stack into a batch tensor, and run a single inference call.
    pub fn extract_embeddings_batch(
        &self,
        frame: &Frame,
        faces: &[FaceDetection],
    ) -> Result<Vec<Vec<f32>>> {
        if faces.is_empty() {
            return Ok(Vec::new());
        }

        let (rec_w, rec_h) = self.rec_input_size;
        let batch_size = faces.len();
        let pixels_per_image = 3 * (rec_h as usize) * (rec_w as usize);

        // Build flat batch tensor [N, 3, 112, 112].
        let mut batch = vec![0.0f32; batch_size * pixels_per_image];
        for (i, face) in faces.iter().enumerate() {
            let crop = crop_and_resize(frame, &face.bbox, rec_w, rec_h)?;
            let single = normalize_hwc_to_nchw_flat(&crop, rec_w, rec_h);
            batch[i * pixels_per_image..(i + 1) * pixels_per_image]
                .copy_from_slice(&single);
        }

        let shape = [batch_size, 3, rec_h as usize, rec_w as usize];
        let input_value =
            ort::value::Tensor::from_array((shape, batch.into_boxed_slice()))?;
        let outputs = self.recognizer.run(ort::inputs![input_value])?;

        let (_, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| PerceptionError::Inference(format!("extract batch embedding: {e}")))?;

        // Output shape: [N, embed_dim].
        let embed_dim = if batch_size > 0 { data.len() / batch_size } else { 0 };

        let embeddings: Vec<Vec<f32>> = (0..batch_size)
            .map(|i| l2_normalize(&data[i * embed_dim..(i + 1) * embed_dim]))
            .collect();

        debug!(batch = batch_size, dim = embed_dim, "batch face embeddings");
        Ok(embeddings)
    }
}

// ---------------------------------------------------------------------------
// Preprocessing helpers
// ---------------------------------------------------------------------------

/// Letterbox-resize a frame to `(target_w, target_h)` and normalize for SCRFD.
///
/// Returns `(flat_nchw_tensor, scale, pad_x, pad_y)` where `scale` and pads
/// allow mapping output coordinates back to the original image space.
fn preprocess_letterbox(
    frame: &Frame,
    target_w: u32,
    target_h: u32,
) -> Result<(Vec<f32>, f32, f32, f32)> {
    if !frame.is_valid() {
        return Err(PerceptionError::Inference("invalid frame data".into()));
    }

    let (fw, fh) = (frame.width as f32, frame.height as f32);
    let (tw, th) = (target_w as f32, target_h as f32);

    let scale = (tw / fw).min(th / fh);
    let new_w = (fw * scale) as u32;
    let new_h = (fh * scale) as u32;
    let pad_x = (target_w - new_w) as f32 / 2.0;
    let pad_y = (target_h - new_h) as f32 / 2.0;

    let resized = resize_bilinear(
        &frame.data,
        frame.width,
        frame.height,
        new_w,
        new_h,
        frame.channels,
    );

    // Padded image filled with 0 (black letterbox bars).
    let mut padded = vec![0u8; (target_w * target_h * frame.channels) as usize];
    let pad_x_u = pad_x as u32;
    let pad_y_u = pad_y as u32;

    for y in 0..new_h {
        let src_row_start = (y * new_w * frame.channels) as usize;
        let dst_row_start = ((y + pad_y_u) * target_w + pad_x_u) as usize * frame.channels as usize;
        let row_bytes = (new_w * frame.channels) as usize;
        if src_row_start + row_bytes <= resized.len()
            && dst_row_start + row_bytes <= padded.len()
        {
            padded[dst_row_start..dst_row_start + row_bytes]
                .copy_from_slice(&resized[src_row_start..src_row_start + row_bytes]);
        }
    }

    let tensor = normalize_hwc_to_nchw_flat(&padded, target_w, target_h);
    Ok((tensor, scale, pad_x, pad_y))
}

/// Convert BGR HWC u8 buffer to a flat NCHW f32 vec with mean/std normalization.
///
/// Output layout: `[1, 3, H, W]` stored in row-major order.
fn normalize_hwc_to_nchw_flat(data: &[u8], width: u32, height: u32) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    let hw = h * w;
    let mut out = vec![0.0f32; 3 * hw];

    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * 3;
            if src + 2 < data.len() {
                let pixel = y * w + x;
                // Channels kept in BGR order (SCRFD and ArcFace convention).
                out[pixel] = (data[src] as f32 - NORM_MEAN) / NORM_STD;
                out[hw + pixel] = (data[src + 1] as f32 - NORM_MEAN) / NORM_STD;
                out[2 * hw + pixel] = (data[src + 2] as f32 - NORM_MEAN) / NORM_STD;
            }
        }
    }

    out
}

/// Simple bilinear resize of a raw pixel buffer.
fn resize_bilinear(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    channels: u32,
) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * channels) as usize];
    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;

    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx = (dx as f32 * x_ratio).min((src_w - 1) as f32);
            let sy = (dy as f32 * y_ratio).min((src_h - 1) as f32);

            let x0 = sx as u32;
            let y0 = sy as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let y1 = (y0 + 1).min(src_h - 1);

            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;

            let dst_idx = ((dy * dst_w + dx) * channels) as usize;

            for c in 0..channels as usize {
                let idx00 = ((y0 * src_w + x0) * channels) as usize + c;
                let idx10 = ((y0 * src_w + x1) * channels) as usize + c;
                let idx01 = ((y1 * src_w + x0) * channels) as usize + c;
                let idx11 = ((y1 * src_w + x1) * channels) as usize + c;

                let v00 = src.get(idx00).copied().unwrap_or(0) as f32;
                let v10 = src.get(idx10).copied().unwrap_or(0) as f32;
                let v01 = src.get(idx01).copied().unwrap_or(0) as f32;
                let v11 = src.get(idx11).copied().unwrap_or(0) as f32;

                let val = v00 * (1.0 - fx) * (1.0 - fy)
                    + v10 * fx * (1.0 - fy)
                    + v01 * (1.0 - fx) * fy
                    + v11 * fx * fy;

                dst[dst_idx + c] = val.round() as u8;
            }
        }
    }

    dst
}

/// Crop a face region from a frame and resize to the given dimensions.
fn crop_and_resize(frame: &Frame, bbox: &BBox, target_w: u32, target_h: u32) -> Result<Vec<u8>> {
    if !frame.is_valid() {
        return Err(PerceptionError::Inference("invalid frame data".into()));
    }

    let ch = frame.channels;
    let fw = frame.width;
    let fh = frame.height;

    // Clamp bbox to frame bounds.
    let x1 = (bbox.x1.max(0.0) as u32).min(fw.saturating_sub(1));
    let y1 = (bbox.y1.max(0.0) as u32).min(fh.saturating_sub(1));
    let x2 = (bbox.x2.max(0.0).ceil() as u32).min(fw);
    let y2 = (bbox.y2.max(0.0).ceil() as u32).min(fh);

    let crop_w = x2.saturating_sub(x1).max(1);
    let crop_h = y2.saturating_sub(y1).max(1);

    let mut crop = vec![0u8; (crop_w * crop_h * ch) as usize];
    for row in 0..crop_h {
        let src_start = (((y1 + row) * fw + x1) * ch) as usize;
        let src_end = src_start + (crop_w * ch) as usize;
        let dst_start = (row * crop_w * ch) as usize;
        let dst_end = dst_start + (crop_w * ch) as usize;

        if src_end <= frame.data.len() && dst_end <= crop.len() {
            crop[dst_start..dst_end].copy_from_slice(&frame.data[src_start..src_end]);
        }
    }

    Ok(resize_bilinear(&crop, crop_w, crop_h, target_w, target_h, ch))
}

// ---------------------------------------------------------------------------
// Postprocessing helpers
// ---------------------------------------------------------------------------

/// Non-maximum suppression: retain highest-confidence detections and remove
/// lower-confidence duplicates that overlap above `iou_threshold`.
fn nms(detections: &mut Vec<(BBox, f32, [f32; 10])>, iou_threshold: f32) {
    detections.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep = Vec::with_capacity(detections.len());
    let mut suppressed = vec![false; detections.len()];

    for i in 0..detections.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(detections[i].clone());

        for j in (i + 1)..detections.len() {
            if !suppressed[j] && detections[i].0.iou(&detections[j].0) > iou_threshold {
                suppressed[j] = true;
            }
        }
    }

    *detections = keep;
}

/// L2-normalize a vector. Returns a zero vector if the norm is zero.
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return vec![0.0; v.len()];
    }
    v.iter().map(|x| x / norm).collect()
}

/// Determine the row stride from raw flat output data.
///
/// SCRFD models emit rows of at least `min_stride` floats (bbox + conf + landmarks).
/// If the total length is evenly divisible by `min_stride`, use it; otherwise try
/// common wider strides before falling back.
fn determine_stride(data: &[f32], min_stride: usize) -> usize {
    if data.len() % min_stride == 0 {
        return min_stride;
    }
    for candidate in [16, 17, 20] {
        if candidate > min_stride && data.len() % candidate == 0 {
            return candidate;
        }
    }
    min_stride
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BBox;

    #[test]
    fn test_nms_removes_overlapping() {
        // Two nearly-identical boxes — the lower-confidence one should be suppressed.
        let mut dets = vec![
            (BBox::new(10.0, 10.0, 100.0, 100.0), 0.9, [0.0; 10]),
            (BBox::new(12.0, 12.0, 102.0, 102.0), 0.7, [0.0; 10]),
        ];
        nms(&mut dets, 0.3);
        assert_eq!(dets.len(), 1);
        assert!((dets[0].1 - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_nms_keeps_non_overlapping() {
        // Two distant boxes — both should survive NMS.
        let mut dets = vec![
            (BBox::new(0.0, 0.0, 50.0, 50.0), 0.9, [0.0; 10]),
            (BBox::new(200.0, 200.0, 300.0, 300.0), 0.85, [0.0; 10]),
        ];
        nms(&mut dets, 0.3);
        assert_eq!(dets.len(), 2);
    }

    #[test]
    fn test_l2_normalize() {
        let v = vec![3.0, 4.0];
        let normed = l2_normalize(&v);
        // Expected: [3/5, 4/5] = [0.6, 0.8].
        assert!((normed[0] - 0.6).abs() < 1e-6);
        assert!((normed[1] - 0.8).abs() < 1e-6);

        // Norm of result should be ~1.0.
        let norm: f32 = normed.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let v = vec![0.0, 0.0, 0.0];
        let normed = l2_normalize(&v);
        assert!(normed.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn test_batch_crop_dimensions() {
        // Synthetic 200×200 BGR frame.
        let width = 200u32;
        let height = 200u32;
        let channels = 3u32;
        let frame = Frame {
            data: vec![128u8; (width * height * channels) as usize],
            width,
            height,
            channels,
            frame_number: 0,
            timestamp: chrono::Utc::now(),
        };

        let faces = vec![
            FaceDetection {
                bbox: BBox::new(10.0, 10.0, 80.0, 80.0),
                confidence: 0.95,
                landmarks: None,
            },
            FaceDetection {
                bbox: BBox::new(100.0, 100.0, 180.0, 180.0),
                confidence: 0.90,
                landmarks: None,
            },
            FaceDetection {
                bbox: BBox::new(50.0, 50.0, 150.0, 150.0),
                confidence: 0.88,
                landmarks: None,
            },
        ];

        let target_w = 112u32;
        let target_h = 112u32;

        for (i, face) in faces.iter().enumerate() {
            let crop = crop_and_resize(&frame, &face.bbox, target_w, target_h)
                .expect("crop_and_resize should succeed");
            let expected_len = (target_w * target_h * channels) as usize;
            assert_eq!(
                crop.len(),
                expected_len,
                "crop {i} size {}, expected {expected_len}",
                crop.len()
            );
        }
    }

    #[test]
    fn test_normalize_hwc_to_nchw_flat_shape() {
        let w = 112u32;
        let h = 112u32;
        let data = vec![128u8; (w * h * 3) as usize];
        let tensor = normalize_hwc_to_nchw_flat(&data, w, h);
        assert_eq!(tensor.len(), 3 * 112 * 112);
    }

    #[test]
    fn test_normalize_hwc_to_nchw_flat_values() {
        // Single pixel, BGR = (0, 127, 255).
        let data = vec![0u8, 127, 255];
        let tensor = normalize_hwc_to_nchw_flat(&data, 1, 1);
        // channel 0 (B): (0 - 127.5) / 128 = -0.99609375
        // channel 1 (G): (127 - 127.5) / 128 = -0.00390625
        // channel 2 (R): (255 - 127.5) / 128 =  0.99609375
        assert!((tensor[0] - (-0.99609375)).abs() < 1e-5);
        assert!((tensor[1] - (-0.00390625)).abs() < 1e-5);
        assert!((tensor[2] - 0.99609375).abs() < 1e-5);
    }

    #[test]
    fn test_resize_bilinear_identity() {
        // Resizing to the same dimensions should be a no-op.
        let data = vec![100u8, 150, 200, 50, 60, 70, 80, 90, 100, 10, 20, 30];
        let result = resize_bilinear(&data, 2, 2, 2, 2, 3);
        assert_eq!(data, result);
    }

    #[test]
    fn test_determine_stride() {
        assert_eq!(determine_stride(&vec![0.0; 30], 15), 15);
        // 32 floats not divisible by 15, but divisible by 16.
        assert_eq!(determine_stride(&vec![0.0; 32], 15), 16);
    }

    #[test]
    fn test_letterbox_scale_and_padding() {
        // 100×200 image letterboxed into 640×640.
        // Scale: min(640/100, 640/200) = min(6.4, 3.2) = 3.2
        // new_w = 320, new_h = 640, pad_x = 160, pad_y = 0
        let frame = Frame {
            data: vec![128u8; 100 * 200 * 3],
            width: 100,
            height: 200,
            channels: 3,
            frame_number: 0,
            timestamp: chrono::Utc::now(),
        };
        let (tensor, scale, pad_x, pad_y) = preprocess_letterbox(&frame, 640, 640)
            .expect("preprocess should succeed");
        assert!((scale - 3.2).abs() < 1e-4);
        assert!((pad_x - 160.0).abs() < 1e-4);
        assert!((pad_y - 0.0).abs() < 1e-4);
        assert_eq!(tensor.len(), 3 * 640 * 640);
    }

    #[test]
    #[ignore = "requires real SCRFD + ArcFace ONNX models on disk"]
    fn test_integration_detect_and_recognize() {
        // Would load real models and run on a test image.
    }
}
