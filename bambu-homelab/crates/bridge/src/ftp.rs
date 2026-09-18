//! FTPS client for Bambu Lab printers (implicit FTPS on port 990).
//! Uses curl as the FTPS client because suppaftp's implicit TLS hangs
//! on the X1C's FTP server.

use anyhow::Context;
use tracing::info;

#[derive(Debug, serde::Serialize)]
pub struct FileEntry {
    pub name: String,
}

/// List files on the printer's SD card via FTPS (curl).
pub fn list_files(ip: &str, access_code: &str) -> anyhow::Result<Vec<FileEntry>> {
    let url = format!("ftps://{ip}:990/");
    let output = std::process::Command::new("curl")
        .args([
            "--ssl-reqd",
            "--insecure",
            "--user", &format!("bblp:{access_code}"),
            "--list-only",
            "--connect-timeout", "10",
            "--max-time", "15",
            &url,
        ])
        .output()
        .context("failed to run curl")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("curl failed: {stderr}");
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    let files: Vec<FileEntry> = listing
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && *l != "." && *l != "..")
        .map(|name| FileEntry { name: name.to_string() })
        .collect();

    info!(count = files.len(), "listed SD card files");
    Ok(files)
}

/// Upload a file to the printer's SD card via FTPS (curl).
pub fn upload_file(ip: &str, access_code: &str, filename: &str, data: &[u8]) -> anyhow::Result<()> {
    // Write data to a temp file for curl
    let tmp = std::env::temp_dir().join(format!("bambu_upload_{filename}"));
    std::fs::write(&tmp, data).context("failed to write temp file")?;

    let url = format!("ftps://{ip}:990/{filename}");
    let output = std::process::Command::new("curl")
        .args([
            "--ssl-reqd",
            "--insecure",
            "--user", &format!("bblp:{access_code}"),
            "--upload-file", tmp.to_str().unwrap_or(""),
            "--connect-timeout", "10",
            "--max-time", "120",
            &url,
        ])
        .output()
        .context("failed to run curl")?;

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("curl upload failed: {stderr}");
    }

    info!(filename, size = data.len(), "uploaded file to printer");
    Ok(())
}
