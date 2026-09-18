//! JSONL sample recorder with optional byte-counted rotation.
//!
//! Owned by a single tokio task; no internal synchronization.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use makana_common::Sample;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::warn;

/// Appends `Sample`s as JSONL to a file, with optional size-based rotation.
///
/// Rotation, when enabled, keeps `keep` most-recent rotated files named
/// `<base>.1 .. <base>.<keep>`. `<base>.1` is the most recent.
pub struct Recorder {
    base: PathBuf,
    rotate_bytes: Option<u64>,
    keep: usize,
    file: File,
    bytes_written: u64,
}

impl Recorder {
    /// Open (or create & append) the base file. Fails if the parent directory
    /// doesn't exist or the path isn't writable.
    ///
    /// `rotate_mb == None` disables rotation entirely (single-file mode).
    /// `rotate_mb == Some(n)` sets threshold to `n * 1024 * 1024` bytes.
    pub async fn new(base: PathBuf, rotate_mb: Option<u64>, keep: usize) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&base)
            .await?;
        let rotate_bytes = rotate_mb.map(|mb| mb.saturating_mul(1024 * 1024));
        Ok(Self {
            base,
            rotate_bytes,
            keep,
            file,
            bytes_written: 0,
        })
    }

    /// Serialize one `Sample` as a JSONL line. Never panics; on I/O or
    /// serialization failure the error is logged and the call returns, so the
    /// broadcast loop keeps draining.
    pub async fn write(&mut self, sample: &Sample) {
        let json = match serde_json::to_string(sample) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "recorder: serialize failed");
                return;
            }
        };
        if let Err(e) = self.file.write_all(json.as_bytes()).await {
            warn!(error = %e, "recorder: write failed");
            return;
        }
        if let Err(e) = self.file.write_all(b"\n").await {
            warn!(error = %e, "recorder: write failed");
            return;
        }
        if let Err(e) = self.file.flush().await {
            warn!(error = %e, "recorder: flush failed");
        }
        // +1 for the newline. Saturating so a no-rotation multi-decade session
        // can't panic; if rotation is on we'd rotate long before saturation.
        let n = (json.len() as u64).saturating_add(1);
        self.bytes_written = self.bytes_written.saturating_add(n);

        if let Some(limit) = self.rotate_bytes
            && self.bytes_written >= limit
            && let Err(e) = self.rotate().await
        {
            warn!(error = %e, "recorder: rotation failed; continuing with current file");
        }
    }

    /// Close the current file, shift rotated indices, and reopen a fresh base.
    async fn rotate(&mut self) -> std::io::Result<()> {
        // Flush before renaming to avoid losing buffered bytes.
        self.file.flush().await?;

        // Delete any stale rotations beyond `keep` (defensive: prior runs with
        // a larger --record-keep may have left them).
        let mut i = self.keep.saturating_add(1);
        loop {
            let p = rotated_path(&self.base, i);
            match tokio::fs::remove_file(&p).await {
                Ok(()) => i = i.saturating_add(1),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e),
            }
        }

        // Shift <base>.(keep-1) → <base>.keep, ..., <base>.1 → <base>.2.
        // For keep == 1 this range is empty and the next rename simply
        // overwrites the previous <base>.1.
        for i in (1..self.keep).rev() {
            let src = rotated_path(&self.base, i);
            let dst = rotated_path(&self.base, i + 1);
            if tokio::fs::try_exists(&src).await? {
                tokio::fs::rename(&src, &dst).await?;
            }
        }

        // Move current → .1, open fresh base.
        let one = rotated_path(&self.base, 1);
        tokio::fs::rename(&self.base, &one).await?;

        let new_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.base)
            .await?;
        self.file = new_file;
        self.bytes_written = 0;
        Ok(())
    }
}

fn rotated_path(base: &Path, i: usize) -> PathBuf {
    let mut s: OsString = base.as_os_str().to_owned();
    s.push(format!(".{i}"));
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use makana_common::{ObdValue, Sample, SamplePayload};
    use tempfile::tempdir;

    fn sample(i: u32) -> Sample {
        Sample {
            source: "test".into(),
            timestamp_ms: 1_700_000_000_000 + u64::from(i),
            payload: SamplePayload::Obd(ObdValue::Rpm(i)),
        }
    }

    async fn count_lines(path: &Path) -> usize {
        match tokio::fs::read_to_string(path).await {
            Ok(s) => s.lines().filter(|l| !l.trim().is_empty()).count(),
            Err(_) => 0,
        }
    }

    /// Parse every JSONL line in a file; return count. Panics on malformed.
    async fn parse_all(path: &Path) -> usize {
        let contents = tokio::fs::read_to_string(path).await.unwrap_or_default();
        let mut n = 0usize;
        for (i, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let _: Sample = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{}:{i} parse: {e} — {line}", path.display()));
            n += 1;
        }
        n
    }

    #[tokio::test]
    async fn single_file_mode_no_rotation() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        let mut rec = Recorder::new(base.clone(), None, 5).await.unwrap();
        for i in 0..100 {
            rec.write(&sample(i)).await;
        }
        drop(rec);

        assert_eq!(count_lines(&base).await, 100);
        assert_eq!(parse_all(&base).await, 100);
        // No rotation siblings.
        assert!(!rotated_path(&base, 1).exists());
        assert!(!rotated_path(&base, 2).exists());
    }

    #[tokio::test]
    async fn rotation_triggers_when_threshold_exceeded() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        // Threshold well below one sample so every write rotates.
        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(1),
            keep: 3,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };

        rec.write(&sample(1)).await;
        // After one write the file we just wrote should have moved to .1 and
        // base should exist (reopened empty).
        assert!(base.exists());
        assert!(rotated_path(&base, 1).exists());
        assert_eq!(count_lines(&base).await, 0);
        assert_eq!(count_lines(&rotated_path(&base, 1)).await, 1);
    }

    #[tokio::test]
    async fn multiple_rotations_shift_indices() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(1),
            keep: 5,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };

        // Three writes → three rotations.
        rec.write(&sample(1)).await; // s1 → .1
        rec.write(&sample(2)).await; // s1 → .2, s2 → .1
        rec.write(&sample(3)).await; // s1 → .3, s2 → .2, s3 → .1

        let read_first_sample = |p: PathBuf| async move {
            let s = tokio::fs::read_to_string(&p).await.unwrap();
            let first = s.lines().next().unwrap().to_owned();
            let v: Sample = serde_json::from_str(&first).unwrap();
            v
        };

        let s1 = read_first_sample(rotated_path(&base, 3)).await;
        let s2 = read_first_sample(rotated_path(&base, 2)).await;
        let s3 = read_first_sample(rotated_path(&base, 1)).await;
        assert_eq!(s1.timestamp_ms, 1_700_000_000_001);
        assert_eq!(s2.timestamp_ms, 1_700_000_000_002);
        assert_eq!(s3.timestamp_ms, 1_700_000_000_003);
    }

    #[tokio::test]
    async fn files_beyond_keep_are_deleted() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(1),
            keep: 2,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };

        // 5 writes with keep=2 → only base, .1, .2 should exist.
        for i in 1..=5 {
            rec.write(&sample(i)).await;
        }

        assert!(base.exists());
        assert!(rotated_path(&base, 1).exists());
        assert!(rotated_path(&base, 2).exists());
        assert!(!rotated_path(&base, 3).exists());
        assert!(!rotated_path(&base, 4).exists());

        // Contents are most-recent: .1 holds sample(5), .2 holds sample(4).
        let read_first = |p: PathBuf| async move {
            let s = tokio::fs::read_to_string(&p).await.unwrap();
            let first = s.lines().next().unwrap().to_owned();
            let v: Sample = serde_json::from_str(&first).unwrap();
            v.timestamp_ms
        };
        assert_eq!(read_first(rotated_path(&base, 1)).await, 1_700_000_000_005);
        assert_eq!(read_first(rotated_path(&base, 2)).await, 1_700_000_000_004);
    }

    #[tokio::test]
    async fn preexisting_stale_rotations_beyond_keep_are_cleaned() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        // Simulate a prior run that used keep=5: leave stale .3, .4, .5 files.
        tokio::fs::write(&base, b"original\n").await.unwrap();
        tokio::fs::write(rotated_path(&base, 3), b"old3\n")
            .await
            .unwrap();
        tokio::fs::write(rotated_path(&base, 4), b"old4\n")
            .await
            .unwrap();
        tokio::fs::write(rotated_path(&base, 5), b"old5\n")
            .await
            .unwrap();

        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(1),
            keep: 2,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };
        rec.write(&sample(1)).await;

        assert!(!rotated_path(&base, 3).exists());
        assert!(!rotated_path(&base, 4).exists());
        assert!(!rotated_path(&base, 5).exists());
    }

    #[tokio::test]
    async fn roundtrip_every_line_parses() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(200), // a few samples per file
            keep: 3,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };
        for i in 0..40 {
            rec.write(&sample(i)).await;
        }
        drop(rec);

        let mut total = parse_all(&base).await;
        for i in 1..=3 {
            let p = rotated_path(&base, i);
            if p.exists() {
                total += parse_all(&p).await;
            }
        }
        assert!(total > 0, "expected samples spread across rotations");
        // File count invariant: at most keep+1 (current + rotated).
        let existing = (1..=4).filter(|i| rotated_path(&base, *i).exists()).count()
            + usize::from(base.exists());
        assert!(existing <= 3 + 1, "file count {existing} > keep+1");
    }

    #[tokio::test]
    async fn keep_one_keeps_only_latest_rotation() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("samples.jsonl");
        let mut rec = Recorder {
            base: base.clone(),
            rotate_bytes: Some(1),
            keep: 1,
            file: OpenOptions::new()
                .append(true)
                .create(true)
                .open(&base)
                .await
                .unwrap(),
            bytes_written: 0,
        };
        for i in 1..=4 {
            rec.write(&sample(i)).await;
        }
        assert!(base.exists());
        assert!(rotated_path(&base, 1).exists());
        assert!(!rotated_path(&base, 2).exists());
        // .1 holds the most recent rotated sample.
        let contents = tokio::fs::read_to_string(rotated_path(&base, 1))
            .await
            .unwrap();
        let v: Sample = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(v.timestamp_ms, 1_700_000_000_004);
    }
}
