//! Pipeline orchestrator — wires detection, tracking, face recognition, OCR,
//! and storage into a single per-frame processing loop.

pub mod detector;
pub mod face;
pub mod ocr;
pub mod tracker;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::capture::{self, FrameSource};
use crate::config::Config;
use crate::download::ModelPaths;
use crate::engine::Engine;
use crate::error::Result;
use crate::face_db::FaceDb;
use crate::preview::{PreviewWindow, VideoRecorder};
use crate::storage::{self, StorageBackend};
use crate::types::{Detection, DetectionKind, Event, Frame, TrackedDetection};

use self::detector::YoloDetector;
use self::face::FacePipeline;
use self::ocr::OcrPipeline;
use self::tracker::ObjectTracker;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Vehicle class labels used to partition new detections for OCR.
const VEHICLE_LABELS: &[&str] = &["car", "truck", "bus", "motorcycle"];

/// Bounded channel capacity between capture thread and async pipeline.
const CHANNEL_CAPACITY: usize = 4;

/// Log metrics every N processed frames.
const METRICS_INTERVAL: u64 = 100;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if `label` is a vehicle class eligible for plate OCR.
fn is_vehicle(label: &str) -> bool {
    VEHICLE_LABELS.contains(&label)
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

struct Metrics {
    frames_captured: u64,
    frames_processed: u64,
    frames_dropped: u64,
    events_total: u64,
    start_time: Instant,
}

impl Metrics {
    fn new() -> Self {
        Self {
            frames_captured: 0,
            frames_processed: 0,
            frames_dropped: 0,
            events_total: 0,
            start_time: Instant::now(),
        }
    }

    fn fps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.frames_processed as f64 / elapsed
        } else {
            0.0
        }
    }

    fn log_if_due(&self) {
        if self.frames_processed > 0 && self.frames_processed % METRICS_INTERVAL == 0 {
            info!(
                fps = format!("{:.1}", self.fps()),
                processed = self.frames_processed,
                dropped = self.frames_dropped,
                events = self.events_total,
                "pipeline metrics"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

pub struct Pipeline {
    detector: Option<YoloDetector>,
    face_pipeline: Option<FacePipeline>,
    face_db: Option<FaceDb>,
    ocr_pipeline: Option<OcrPipeline>,
    tracker: Option<ObjectTracker>,
    storage: Box<dyn StorageBackend>,
    preview: Option<PreviewWindow>,
    recorder: Option<VideoRecorder>,
    record_path: Option<PathBuf>,
    config: Config,
}

impl Pipeline {
    /// Conditionally construct pipeline components based on config flags and
    /// available model paths.
    pub async fn new(config: &Config, model_paths: &ModelPaths, record_path: Option<PathBuf>) -> Result<Self> {
        let detector = if config.pipeline.detection {
            if let Some(ref path) = model_paths.detection {
                let engine = Arc::new(Engine::new(path)?);
                Some(YoloDetector::new(
                    engine,
                    &config.pipeline.detection_config,
                    config.pipeline.confidence_threshold,
                ))
            } else {
                warn!("detection enabled but no model path available");
                None
            }
        } else {
            None
        };

        let (face_pipeline, face_db) = if config.pipeline.face_recognition {
            match (&model_paths.face_detection, &model_paths.face_recognition) {
                (Some(det_path), Some(rec_path)) => {
                    let det_engine = Arc::new(Engine::new(det_path)?);
                    let rec_engine = Arc::new(Engine::new(rec_path)?);
                    let fp = FacePipeline::new(
                        det_engine,
                        rec_engine,
                        config.pipeline.confidence_threshold,
                    );
                    let db = FaceDb::load_from_dir(
                        &config.pipeline.face_config.known_faces_dir,
                        config.pipeline.face_config.similarity_threshold,
                    )?;
                    info!(known_faces = db.len(), "face database loaded");
                    (Some(fp), Some(db))
                }
                _ => {
                    warn!("face_recognition enabled but face model paths unavailable");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        let ocr_pipeline = if config.pipeline.ocr {
            match (
                &model_paths.ocr_detection,
                &model_paths.ocr_recognition,
                &model_paths.ocr_keys,
            ) {
                (Some(det_path), Some(rec_path), Some(keys_path)) => {
                    let det_engine = Arc::new(Engine::new(det_path)?);
                    let rec_engine = Arc::new(Engine::new(rec_path)?);
                    Some(OcrPipeline::new(
                        det_engine,
                        rec_engine,
                        keys_path,
                        config.pipeline.ocr_config.max_batch_size,
                    )?)
                }
                _ => {
                    warn!("OCR enabled but OCR model paths unavailable");
                    None
                }
            }
        } else {
            None
        };

        let tracker = if config.pipeline.tracker_config.enabled {
            Some(ObjectTracker::new(&config.pipeline.tracker_config))
        } else {
            None
        };

        let storage = storage::create_storage(&config.storage).await?;

        let preview = if config.preview.enabled {
            Some(PreviewWindow::new(config.preview.window_width)?)
        } else {
            None
        };

        Ok(Self {
            detector,
            face_pipeline,
            face_db,
            ocr_pipeline,
            tracker,
            storage,
            preview,
            recorder: None, // initialized lazily on first frame (needs dimensions)
            config: config.clone(),
            record_path,
        })
    }

    /// Process a single frame through all enabled pipeline stages.
    ///
    /// Returns events produced from detections, face matches, and OCR results.
    pub async fn process_frame(&mut self, frame: &Frame) -> Result<Vec<Event>> {
        // 1. Detection
        let detections = match self.detector {
            Some(ref detector) => detector.detect(frame)?,
            None => Vec::new(),
        };

        // 2. Tracking
        let tracked = match self.tracker {
            Some(ref mut tracker) => tracker.update(&detections),
            None => wrap_as_tracked(detections),
        };

        // 3. Partition new detections by type
        let new_persons: Vec<&TrackedDetection> = tracked
            .iter()
            .filter(|t| t.is_new && t.detection.label == "person")
            .collect();

        let new_vehicles: Vec<&TrackedDetection> = tracked
            .iter()
            .filter(|t| t.is_new && is_vehicle(&t.detection.label))
            .collect();

        // 4. Face recognition on new persons
        let mut face_results: Vec<(u64, String, f32)> = Vec::new();
        if let (Some(ref face_pipe), Some(ref face_db)) =
            (&self.face_pipeline, &self.face_db)
        {
            if !new_persons.is_empty() {
                for person in &new_persons {
                    let faces = face_pipe.detect_faces(frame)?;
                    if !faces.is_empty() {
                        let embeddings = face_pipe.extract_embeddings_batch(frame, &faces)?;
                        for emb in &embeddings {
                            if let Some(m) = face_db.find_match(emb) {
                                face_results.push((
                                    person.track_id,
                                    m.name.clone(),
                                    m.similarity,
                                ));
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 5. OCR on new vehicles
        let mut ocr_results: Vec<(u64, String)> = Vec::new();
        if let Some(ref ocr_pipe) = self.ocr_pipeline {
            if !new_vehicles.is_empty() {
                let bboxes: Vec<_> = new_vehicles
                    .iter()
                    .map(|v| v.detection.bbox.clone())
                    .collect();
                let results = ocr_pipe.recognize_text(frame, &bboxes)?;
                for (i, ocr) in results.into_iter().enumerate() {
                    if !ocr.text.is_empty() {
                        if let Some(vehicle) = new_vehicles.get(i) {
                            ocr_results.push((vehicle.track_id, ocr.text));
                        }
                    }
                }
            }
        }

        // 6. Build events
        let mut events: Vec<Event> = Vec::with_capacity(tracked.len());
        for td in &tracked {
            let mut event = Event::new(
                frame.timestamp,
                frame.frame_number,
                td.detection.kind,
                td.detection.confidence,
                td.detection.bbox.clone(),
            );
            event.track_id = Some(td.track_id);
            event.label = Some(td.detection.label.clone());

            if let Some((_, ref name, sim)) =
                face_results.iter().find(|(tid, _, _)| *tid == td.track_id)
            {
                event.face_identity = Some(name.clone());
                event.face_similarity = Some(*sim);
            }

            if let Some((_, ref text)) =
                ocr_results.iter().find(|(tid, _)| *tid == td.track_id)
            {
                event.ocr_text = Some(text.clone());
            }

            events.push(event);
        }

        // 7. Store
        self.storage.store_events(&events).await?;

        // 8. Preview — returns false if ESC was pressed
        if let Some(ref preview) = self.preview {
            if !preview.show(frame, &tracked)? {
                return Err(crate::error::PerceptionError::Capture(
                    "preview window closed".into(),
                ));
            }
        }


        // 9. Record — initialize lazily on first frame, then write every frame
        if self.record_path.is_some() && self.recorder.is_none() {
            let path = self.record_path.as_ref().unwrap();
            let fps = self.config.capture.fps_limit.max(1) as f64;
            self.recorder = Some(VideoRecorder::new(
                path,
                fps,
                frame.width as i32,
                frame.height as i32,
            )?);
            info!(path = %path.display(), "recording annotated output");
        }
        if let Some(ref mut recorder) = self.recorder {
            recorder.write_frame(frame, &tracked)?;
        }

        Ok(events)
    }
}

/// Wrap raw detections as TrackedDetection with sequential IDs, all new.
fn wrap_as_tracked(detections: Vec<Detection>) -> Vec<TrackedDetection> {
    detections
        .into_iter()
        .enumerate()
        .map(|(i, detection)| TrackedDetection {
            detection,
            track_id: i as u64 + 1,
            is_new: true,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Top-level run function
// ---------------------------------------------------------------------------

/// Entry point called from main.rs. Constructs the pipeline, spawns the
/// capture thread, and runs the per-frame processing loop until the source
/// is exhausted or Ctrl+C is received.
pub async fn run(config: Config, model_paths: ModelPaths, record_path: Option<PathBuf>) -> Result<()> {
    let mut pipeline = Pipeline::new(&config, &model_paths, record_path).await?;
    let mut metrics = Metrics::new();

    // Create frame source (must happen before thread spawn so errors surface here).
    let mut source = capture::create_source(&config.capture)?;

    // Bounded channel: capture thread -> async pipeline.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(CHANNEL_CAPACITY);

    // Shared stop flag: set by Ctrl+C or ESC, checked by capture thread.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_capture = stop.clone();

    // Track dropped frames in the capture thread.
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dropped_tx = dropped.clone();

    // Determine frame pacing for video files.
    let source_fps = source.fps();
    let fps_limit = config.capture.fps_limit;
    let frame_interval = if config.capture.source == "video" {
        let target_fps = if fps_limit > 0 {
            (fps_limit as f64).min(source_fps.unwrap_or(30.0))
        } else {
            source_fps.unwrap_or(30.0)
        };
        Some(std::time::Duration::from_secs_f64(1.0 / target_fps))
    } else if fps_limit > 0 {
        Some(std::time::Duration::from_secs_f64(1.0 / fps_limit as f64))
    } else {
        None
    };

    let capture_handle = std::thread::spawn(move || {
        loop {
            if stop_capture.load(std::sync::atomic::Ordering::Relaxed) {
                debug!("capture thread: stop signal received");
                break;
            }
            let frame_start = Instant::now();
            match source.next_frame() {
                Ok(Some(frame)) => {
                    if tx.try_send(frame).is_err() {
                        dropped_tx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Ok(None) => {
                    debug!("capture source exhausted");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "capture error, stopping");
                    break;
                }
            }
            if let Some(interval) = frame_interval {
                let elapsed = frame_start.elapsed();
                if elapsed < interval {
                    std::thread::sleep(interval - elapsed);
                }
            }
        }
    });

    // Main processing loop.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            biased;

            _ = &mut ctrl_c => {
                info!("Ctrl+C received, shutting down");
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }

            frame = rx.recv() => {
                match frame {
                    Some(frame) => {
                        metrics.frames_captured += 1;
                        metrics.frames_dropped =
                            dropped.load(std::sync::atomic::Ordering::Relaxed);

                        match pipeline.process_frame(&frame).await {
                            Ok(events) => {
                                metrics.events_total += events.len() as u64;
                                metrics.frames_processed += 1;
                                metrics.log_if_due();
                            }
                            Err(ref e) if e.to_string().contains("preview window closed") => {
                                info!("preview window closed (ESC), shutting down");
                                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                                break;
                            }
                            Err(e) => {
                                error!(
                                    frame = frame.frame_number,
                                    error = %e,
                                    "frame processing failed"
                                );
                            }
                        }
                    }
                    None => {
                        debug!("capture channel closed");
                        break;
                    }
                }
            }
        }
    }

    // Drain any remaining frames in the channel.
    while let Ok(frame) = rx.try_recv() {
        if let Ok(events) = pipeline.process_frame(&frame).await {
            metrics.events_total += events.len() as u64;
            metrics.frames_processed += 1;
        }
    }

    // Wait for capture thread to finish.
    let _ = capture_handle.join();

    // Final metrics
    metrics.frames_dropped = dropped.load(std::sync::atomic::Ordering::Relaxed);
    let elapsed = metrics.start_time.elapsed();
    info!(
        processed = metrics.frames_processed,
        dropped = metrics.frames_dropped,
        events = metrics.events_total,
        elapsed_secs = format!("{:.1}", elapsed.as_secs_f64()),
        avg_fps = format!("{:.1}", metrics.fps()),
        "pipeline finished"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vehicle_label_matching() {
        assert!(is_vehicle("car"));
        assert!(is_vehicle("truck"));
        assert!(is_vehicle("bus"));
        assert!(is_vehicle("motorcycle"));
        assert!(!is_vehicle("person"));
        assert!(!is_vehicle("bicycle"));
        assert!(!is_vehicle("dog"));
        assert!(!is_vehicle(""));
    }

    #[test]
    fn metrics_counter_increments() {
        let mut m = Metrics::new();
        assert_eq!(m.frames_processed, 0);
        assert_eq!(m.frames_dropped, 0);
        assert_eq!(m.events_total, 0);

        m.frames_processed += 1;
        m.frames_captured += 1;
        m.events_total += 3;
        m.frames_dropped += 2;

        assert_eq!(m.frames_processed, 1);
        assert_eq!(m.frames_captured, 1);
        assert_eq!(m.events_total, 3);
        assert_eq!(m.frames_dropped, 2);
    }

    #[test]
    fn metrics_fps_computation() {
        let m = Metrics {
            frames_captured: 0,
            frames_processed: 0,
            frames_dropped: 0,
            events_total: 0,
            start_time: Instant::now(),
        };
        // With 0 frames processed, FPS should be 0 (or near zero).
        assert!(m.fps() < 1.0);
    }

    #[test]
    fn wrap_as_tracked_assigns_sequential_ids() {
        let detections = vec![
            Detection {
                bbox: crate::types::BBox::new(0.0, 0.0, 10.0, 10.0),
                confidence: 0.9,
                class_id: 0,
                label: "person".into(),
                kind: DetectionKind::Object,
            },
            Detection {
                bbox: crate::types::BBox::new(20.0, 20.0, 30.0, 30.0),
                confidence: 0.8,
                class_id: 2,
                label: "car".into(),
                kind: DetectionKind::Object,
            },
        ];

        let tracked = wrap_as_tracked(detections);
        assert_eq!(tracked.len(), 2);
        assert_eq!(tracked[0].track_id, 1);
        assert_eq!(tracked[1].track_id, 2);
        assert!(tracked[0].is_new);
        assert!(tracked[1].is_new);
        assert_eq!(tracked[0].detection.label, "person");
        assert_eq!(tracked[1].detection.label, "car");
    }
}
