//! Domain types shared across the perception pipeline.
//!
//! These types flow through the system: capture produces [`Frame`]s,
//! pipelines produce [`Detection`]s, and the storage layer persists [`Event`]s.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Raw frame data extracted from a capture source.
///
/// Owns its pixel data as a byte vector so it can cross thread/channel
/// boundaries (OpenCV `Mat` is not `Send`).
#[derive(Debug, Clone)]
pub struct Frame {
    /// Raw pixel bytes in BGR format (OpenCV convention).
    pub data: Vec<u8>,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Number of channels (typically 3 for BGR).
    pub channels: u32,
    /// Monotonically increasing frame counter from the source.
    pub frame_number: u64,
    /// Capture timestamp.
    pub timestamp: DateTime<Utc>,
}

impl Frame {
    /// Total expected byte length: width * height * channels.
    pub fn byte_len(&self) -> usize {
        (self.width * self.height * self.channels) as usize
    }

    /// Returns true if the frame has valid dimensions and matching data length.
    pub fn is_valid(&self) -> bool {
        !self.data.is_empty() && self.data.len() == self.byte_len()
    }
}

/// Axis-aligned bounding box in pixel coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl BBox {
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    pub fn width(&self) -> f32 {
        self.x2 - self.x1
    }

    pub fn height(&self) -> f32 {
        self.y2 - self.y1
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn center(&self) -> (f32, f32) {
        ((self.x1 + self.x2) / 2.0, (self.y1 + self.y2) / 2.0)
    }

    /// Intersection-over-Union with another bounding box.
    pub fn iou(&self, other: &BBox) -> f32 {
        let inter_x1 = self.x1.max(other.x1);
        let inter_y1 = self.y1.max(other.y1);
        let inter_x2 = self.x2.min(other.x2);
        let inter_y2 = self.y2.min(other.y2);

        let inter_w = (inter_x2 - inter_x1).max(0.0);
        let inter_h = (inter_y2 - inter_y1).max(0.0);
        let inter_area = inter_w * inter_h;

        let union_area = self.area() + other.area() - inter_area;
        if union_area <= 0.0 {
            return 0.0;
        }
        inter_area / union_area
    }
}

/// What kind of detection this represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionKind {
    Object,
    Face,
    Text,
}

impl std::fmt::Display for DetectionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetectionKind::Object => write!(f, "object"),
            DetectionKind::Face => write!(f, "face"),
            DetectionKind::Text => write!(f, "text"),
        }
    }
}

/// A single detection produced by a pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub bbox: BBox,
    pub confidence: f32,
    pub class_id: u32,
    pub label: String,
    pub kind: DetectionKind,
}

/// A detection with a stable tracking ID assigned by ByteTrack.
#[derive(Debug, Clone)]
pub struct TrackedDetection {
    pub detection: Detection,
    /// Stable ID persisting across frames for the same physical object.
    pub track_id: u64,
    /// True only on the first frame this object appeared.
    pub is_new: bool,
}

/// Result of face recognition on a detected face.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceMatch {
    pub embedding: Vec<f32>,
    pub identity: Option<String>,
    pub similarity: f32,
}

/// Result of OCR on a detected text region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    pub bbox: BBox,
    pub confidence: f32,
}

/// A persisted event representing something the pipeline observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub frame_number: u64,
    pub track_id: Option<u64>,
    pub detection_kind: DetectionKind,
    pub label: Option<String>,
    pub confidence: f32,
    pub bbox: BBox,
    pub ocr_text: Option<String>,
    pub face_identity: Option<String>,
    pub face_similarity: Option<f32>,
    pub crop_path: Option<String>,
}

impl Event {
    pub fn new(
        timestamp: DateTime<Utc>,
        frame_number: u64,
        detection_kind: DetectionKind,
        confidence: f32,
        bbox: BBox,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp,
            frame_number,
            track_id: None,
            detection_kind,
            label: None,
            confidence,
            bbox,
            ocr_text: None,
            face_identity: None,
            face_similarity: None,
            crop_path: None,
        }
    }
}

/// Filter criteria for querying stored events.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub kind: Option<DetectionKind>,
    pub label: Option<String>,
    pub track_id: Option<u64>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<u32>,
}
