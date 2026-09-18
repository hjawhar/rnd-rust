//! Known face embedding database.
//!
//! Stores face embeddings for known individuals. When a new face is detected,
//! its embedding is compared against this database using cosine similarity.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::{PerceptionError, Result};

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "embedding dimensions must match");
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// In-memory database of known face embeddings.
pub struct FaceDb {
    entries: Vec<FaceEntry>,
    threshold: f32,
}

struct FaceEntry {
    name: String,
    embedding: Vec<f32>,
}

/// Result of matching an embedding against the known faces database.
pub struct MatchResult {
    pub name: String,
    pub similarity: f32,
}

impl FaceDb {
    /// Create an empty face database with the given similarity threshold.
    pub fn new(threshold: f32) -> Self {
        Self {
            entries: Vec::new(),
            threshold,
        }
    }

    /// Load known faces from the faces directory.
    ///
    /// Expected structure: `known_faces_dir/{name}/` containing embedding files.
    /// Each embedding file is a JSON array of 512 f32 values.
    pub fn load_from_dir(dir: &Path, threshold: f32) -> Result<Self> {
        let mut db = Self::new(threshold);

        if !dir.exists() {
            tracing::debug!(?dir, "known faces directory does not exist, starting empty");
            return Ok(db);
        }

        let entries = std::fs::read_dir(dir).map_err(|e| PerceptionError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| PerceptionError::Io {
                path: dir.to_path_buf(),
                source: e,
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Look for embedding.json inside the person directory.
            let emb_path = path.join("embedding.json");
            if emb_path.exists() {
                let data = std::fs::read_to_string(&emb_path).map_err(|e| PerceptionError::Io {
                    path: emb_path.clone(),
                    source: e,
                })?;
                let embedding: Vec<f32> = serde_json::from_str(&data)
                    .map_err(|e| PerceptionError::Config(format!("bad embedding for {name}: {e}")))?;
                db.add(&name, embedding);
                tracing::info!(%name, "loaded known face");
            }
        }

        Ok(db)
    }

    /// Add a named embedding to the database.
    pub fn add(&mut self, name: &str, embedding: Vec<f32>) {
        self.entries.push(FaceEntry {
            name: name.to_string(),
            embedding,
        });
    }

    /// Find the best match for the given embedding.
    ///
    /// Returns `None` if no known face exceeds the similarity threshold.
    pub fn find_match(&self, query: &[f32]) -> Option<MatchResult> {
        let mut best: Option<MatchResult> = None;

        for entry in &self.entries {
            let sim = cosine_similarity(query, &entry.embedding);
            if sim >= self.threshold {
                if best.as_ref().map_or(true, |b| sim > b.similarity) {
                    best = Some(MatchResult {
                        name: entry.name.clone(),
                        similarity: sim,
                    });
                }
            }
        }

        best
    }

    /// Number of known faces in the database.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Names of all known faces.
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }
}

// --- CLI helpers called from main.rs ---

/// Add a known face from an image file.
///
/// In a full implementation this would run face detection + ArcFace embedding
/// on the provided image. For now it creates the directory structure.
pub async fn add_face(config: &Config, name: &str, image_path: &Path) -> Result<()> {
    let faces_dir = &config.pipeline.face_config.known_faces_dir;
    let person_dir = faces_dir.join(name);
    std::fs::create_dir_all(&person_dir).map_err(|e| PerceptionError::Io {
        path: person_dir.clone(),
        source: e,
    })?;

    // Copy the image into the person directory.
    let dest = person_dir.join(
        image_path
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("face.jpg")),
    );
    std::fs::copy(image_path, &dest).map_err(|e| PerceptionError::Io {
        path: dest.clone(),
        source: e,
    })?;

    tracing::info!(%name, ?dest, "face image saved — embedding will be computed on next pipeline run");
    Ok(())
}

/// List all known faces.
pub fn list_faces(config: &Config) -> Result<Vec<String>> {
    let faces_dir = &config.pipeline.face_config.known_faces_dir;
    if !faces_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    let entries = std::fs::read_dir(faces_dir).map_err(|e| PerceptionError::Io {
        path: faces_dir.clone(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| PerceptionError::Io {
            path: faces_dir.clone(),
            source: e,
        })?;
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }

    names.sort();
    Ok(names)
}

/// Remove a known face by name.
pub fn remove_face(config: &Config, name: &str) -> Result<()> {
    let person_dir = config.pipeline.face_config.known_faces_dir.join(name);
    if person_dir.exists() {
        std::fs::remove_dir_all(&person_dir).map_err(|e| PerceptionError::Io {
            path: person_dir,
            source: e,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![1.0, 2.0];
        let b = vec![0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn face_db_match() {
        let mut db = FaceDb::new(0.5);
        db.add("alice", vec![1.0, 0.0, 0.0]);
        db.add("bob", vec![0.0, 1.0, 0.0]);

        let query = vec![0.9, 0.1, 0.0]; // close to alice
        let m = db.find_match(&query).unwrap();
        assert_eq!(m.name, "alice");
        assert!(m.similarity > 0.9);
    }

    #[test]
    fn face_db_no_match_below_threshold() {
        let mut db = FaceDb::new(0.9);
        db.add("alice", vec![1.0, 0.0, 0.0]);

        let query = vec![0.5, 0.5, 0.5]; // not close enough
        assert!(db.find_match(&query).is_none());
    }

    #[test]
    fn face_db_empty() {
        let db = FaceDb::new(0.5);
        assert!(db.is_empty());
        assert!(db.find_match(&[1.0, 0.0]).is_none());
    }

    #[test]
    fn list_faces_nonexistent_dir() {
        let config = test_config();
        let names = list_faces(&config).unwrap();
        assert!(names.is_empty());
    }

    fn test_config() -> Config {
        Config::from_str(
            r#"
[capture]
source = "image"
path = "test.jpg"

[pipeline.face_config]
known_faces_dir = "/tmp/perception_test_nonexistent_faces/"
"#,
        )
        .unwrap()
    }
}
