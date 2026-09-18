//! Multi-object tracker using IoU-based greedy assignment.
//!
//! Implements a simplified SORT/ByteTrack algorithm: maintains active tracks,
//! associates new detections via IoU cost matrix with greedy assignment, and
//! ages out unmatched tracks after `max_age` frames.

use crate::config::TrackerConfig;
use crate::types::{BBox, Detection, TrackedDetection};

/// Internal track state maintained across frames.
struct Track {
    id: u64,
    bbox: BBox,
    /// Frames since this track was last matched to a detection.
    age: u32,
    /// Total number of frames this track has been matched.
    visible: u32,
}

/// IoU-based multi-object tracker with greedy assignment.
///
/// Maintains a set of active tracks across frames, associating incoming
/// detections to existing tracks via IoU overlap. Unmatched detections
/// spawn new tracks; unmatched tracks age out after `max_age` frames.
pub struct ObjectTracker {
    tracks: Vec<Track>,
    next_id: u64,
    max_age: u32,
    iou_threshold: f32,
}

impl ObjectTracker {
    /// Create a new tracker from pipeline configuration.
    pub fn new(config: &TrackerConfig) -> Self {
        Self {
            tracks: Vec::new(),
            next_id: 1,
            max_age: config.max_age,
            iou_threshold: config.iou_threshold,
        }
    }

    /// Update tracker with new detections, returning tracked detections with stable IDs.
    ///
    /// Performs greedy IoU-based assignment between existing tracks and incoming
    /// detections. Matched tracks are updated; unmatched detections create new
    /// tracks (marked `is_new = true`); stale tracks exceeding `max_age` are pruned.
    pub fn update(&mut self, detections: &[Detection]) -> Vec<TrackedDetection> {
        let num_dets = detections.len();

        // det_idx -> assigned track_id. Populated during matching and new-track creation.
        let mut det_track_id: Vec<u64> = vec![0; num_dets];
        let mut matched_dets = vec![false; num_dets];

        // --- Greedy IoU assignment ---
        let mut candidates: Vec<(usize, usize, f32)> = Vec::new();
        for (ti, track) in self.tracks.iter().enumerate() {
            for (di, det) in detections.iter().enumerate() {
                let iou = track.bbox.iou(&det.bbox);
                if iou >= self.iou_threshold {
                    candidates.push((ti, di, iou));
                }
            }
        }
        candidates.sort_unstable_by(|a, b| {
            b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut matched_tracks = vec![false; self.tracks.len()];

        for &(ti, di, _) in &candidates {
            if matched_tracks[ti] || matched_dets[di] {
                continue;
            }
            matched_tracks[ti] = true;
            matched_dets[di] = true;

            self.tracks[ti].bbox = detections[di].bbox.clone();
            self.tracks[ti].age = 0;
            self.tracks[ti].visible += 1;

            det_track_id[di] = self.tracks[ti].id;
        }

        // Age unmatched tracks.
        for (ti, matched) in matched_tracks.iter().enumerate() {
            if !matched {
                self.tracks[ti].age += 1;
            }
        }

        // Prune stale tracks (age > max_age).
        self.tracks.retain(|t| t.age <= self.max_age);

        // Create new tracks for unmatched detections.
        for di in 0..num_dets {
            if !matched_dets[di] {
                let id = self.next_id;
                self.next_id += 1;
                det_track_id[di] = id;
                self.tracks.push(Track {
                    id,
                    bbox: detections[di].bbox.clone(),
                    age: 0,
                    visible: 1,
                });
            }
        }

        // Build output.
        detections
            .iter()
            .enumerate()
            .map(|(di, det)| TrackedDetection {
                detection: det.clone(),
                track_id: det_track_id[di],
                is_new: !matched_dets[di],
            })
            .collect()
    }

    /// Number of currently active tracks.
    pub fn active_tracks(&self) -> usize {
        self.tracks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DetectionKind;

    fn make_config(max_age: u32, iou_threshold: f32) -> TrackerConfig {
        TrackerConfig {
            enabled: true,
            max_age,
            iou_threshold,
        }
    }

    fn make_det(x1: f32, y1: f32, x2: f32, y2: f32) -> Detection {
        Detection {
            bbox: BBox::new(x1, y1, x2, y2),
            confidence: 0.9,
            class_id: 0,
            label: "person".into(),
            kind: DetectionKind::Object,
        }
    }

    #[test]
    fn test_new_objects_get_unique_ids() {
        let config = make_config(30, 0.3);
        let mut tracker = ObjectTracker::new(&config);

        let dets = vec![
            make_det(0.0, 0.0, 50.0, 50.0),
            make_det(100.0, 100.0, 150.0, 150.0),
            make_det(200.0, 200.0, 250.0, 250.0),
        ];

        let tracked = tracker.update(&dets);
        assert_eq!(tracked.len(), 3);

        let mut ids: Vec<u64> = tracked.iter().map(|t| t.track_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3, "all 3 detections must have unique IDs");

        assert!(
            tracked.iter().all(|t| t.is_new),
            "all must be is_new on first frame"
        );
    }

    #[test]
    fn test_same_object_same_id() {
        let config = make_config(30, 0.3);
        let mut tracker = ObjectTracker::new(&config);

        // Frame 1: object appears.
        let tracked1 = tracker.update(&[make_det(100.0, 100.0, 200.0, 200.0)]);
        assert_eq!(tracked1.len(), 1);
        assert!(tracked1[0].is_new);
        let id = tracked1[0].track_id;

        // Frame 2: same object, slightly moved (high IoU).
        let tracked2 = tracker.update(&[make_det(105.0, 105.0, 205.0, 205.0)]);
        assert_eq!(tracked2.len(), 1);
        assert_eq!(tracked2[0].track_id, id, "same object must keep same ID");
        assert!(
            !tracked2[0].is_new,
            "must not be is_new on subsequent frame"
        );
    }

    #[test]
    fn test_object_disappears_and_returns() {
        let config = make_config(2, 0.3);
        let mut tracker = ObjectTracker::new(&config);

        // Frame 1: object appears.
        let tracked = tracker.update(&[make_det(100.0, 100.0, 200.0, 200.0)]);
        let original_id = tracked[0].track_id;

        // Frames 2..=4: object absent (3 empty frames > max_age=2).
        for _ in 0..3 {
            tracker.update(&[]);
        }

        // Frame 5: object reappears at same location.
        let tracked = tracker.update(&[make_det(100.0, 100.0, 200.0, 200.0)]);
        assert_eq!(tracked.len(), 1);
        assert!(tracked[0].is_new, "must be treated as new after expiry");
        assert_ne!(
            tracked[0].track_id, original_id,
            "must get a new ID after track expired"
        );
    }

    #[test]
    fn test_age_out() {
        let config = make_config(2, 0.3);
        let mut tracker = ObjectTracker::new(&config);

        // Frame 1: create a track.
        tracker.update(&[make_det(100.0, 100.0, 200.0, 200.0)]);
        assert_eq!(tracker.active_tracks(), 1);

        // Frame 2: no detections — age becomes 1.
        tracker.update(&[]);
        assert_eq!(tracker.active_tracks(), 1);

        // Frame 3: no detections — age becomes 2 (== max_age, still alive).
        tracker.update(&[]);
        assert_eq!(tracker.active_tracks(), 1);

        // Frame 4: no detections — age becomes 3 (> max_age, pruned).
        tracker.update(&[]);
        assert_eq!(tracker.active_tracks(), 0);
    }

    #[test]
    fn test_multiple_objects_tracked() {
        let config = make_config(30, 0.3);
        let mut tracker = ObjectTracker::new(&config);

        // 5 non-overlapping detections.
        let dets: Vec<Detection> = (0..5)
            .map(|i| {
                let x = i as f32 * 100.0;
                make_det(x, 0.0, x + 50.0, 50.0)
            })
            .collect();

        let tracked1 = tracker.update(&dets);
        let ids1: Vec<u64> = tracked1.iter().map(|t| t.track_id).collect();

        // Frame 2: same 5 objects, each shifted by 5px (high IoU).
        let dets2: Vec<Detection> = (0..5)
            .map(|i| {
                let x = i as f32 * 100.0 + 5.0;
                make_det(x, 0.0, x + 50.0, 50.0)
            })
            .collect();
        let tracked2 = tracker.update(&dets2);
        let ids2: Vec<u64> = tracked2.iter().map(|t| t.track_id).collect();

        // Frame 3: same 5 objects, shifted another 5px.
        let dets3: Vec<Detection> = (0..5)
            .map(|i| {
                let x = i as f32 * 100.0 + 10.0;
                make_det(x, 0.0, x + 50.0, 50.0)
            })
            .collect();
        let tracked3 = tracker.update(&dets3);
        let ids3: Vec<u64> = tracked3.iter().map(|t| t.track_id).collect();

        assert_eq!(ids1, ids2, "IDs must be stable across frame 1->2");
        assert_eq!(ids2, ids3, "IDs must be stable across frame 2->3");
        assert_eq!(tracker.active_tracks(), 5);
    }

    #[test]
    fn test_greedy_assignment_prefers_highest_iou() {
        let config = make_config(30, 0.1); // low threshold so both tracks are candidates
        let mut tracker = ObjectTracker::new(&config);

        // Create two tracks.
        tracker.update(&[
            make_det(100.0, 100.0, 200.0, 200.0),
            make_det(150.0, 150.0, 250.0, 250.0),
        ]);
        let id_result = tracker.update(&[
            make_det(100.0, 100.0, 200.0, 200.0),
            make_det(150.0, 150.0, 250.0, 250.0),
        ]);
        let track_a = id_result[0].track_id;

        // Single detection closer to track A: (102,102,202,202) has higher IoU
        // with (100,100,200,200) than with (150,150,250,250).
        let tracked = tracker.update(&[make_det(102.0, 102.0, 202.0, 202.0)]);
        assert_eq!(tracked.len(), 1);
        assert_eq!(
            tracked[0].track_id, track_a,
            "must match the track with higher IoU"
        );
        assert!(!tracked[0].is_new);
    }
}
