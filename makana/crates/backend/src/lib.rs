//! Backend library: CLI parsing + sample recorder. The binary entry point
//! lives in `main.rs` and is kept deliberately thin.

#![forbid(unsafe_code)]

pub mod recorder;

use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, ValueEnum};
use makana_common::ObdPid;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum TransportKind {
    Mock,
    Elm327,
    Logfile,
}

#[derive(Parser, Debug)]
#[command(name = "makana-backend", about = "Vehicle telemetry broker")]
pub struct Cli {
    #[arg(long, env = "MAKANA_TRANSPORT", value_enum, default_value = "mock")]
    pub transport: TransportKind,

    /// Required for `--transport elm327` (serial/pty path) and `--transport
    /// logfile` (JSONL file path).
    #[arg(long, env = "MAKANA_DEVICE")]
    pub device: Option<String>,

    /// Baud rate for ELM327 serial.
    #[arg(long, env = "MAKANA_BAUD", default_value_t = 38_400)]
    pub baud: u32,

    /// Comma-separated list of 4-hex-digit OBD PIDs (`mode` + `pid`) for
    /// ELM327 polling, e.g. `010C,010D,0105`. Only meaningful when
    /// `--transport elm327`.
    #[arg(long, env = "MAKANA_PIDS", value_parser = parse_pids_arg)]
    pub pids: Option<Vec<ObdPid>>,

    #[arg(long, env = "MAKANA_BIND", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,

    /// Mock source tick interval in milliseconds.
    #[arg(long, env = "MAKANA_MOCK_TICK_MS", default_value_t = 100)]
    pub mock_tick_ms: u64,

    /// Logfile playback speed. `2.0` = twice real-time. Only meaningful for
    /// `--transport logfile`.
    #[arg(long, env = "MAKANA_SPEED", default_value_t = 1.0, value_parser = parse_positive_speed)]
    pub speed: f32,

    /// Logfile repeat: loop back to start at EOF forever. Only meaningful for
    /// `--transport logfile`.
    #[arg(long, env = "MAKANA_REPEAT", default_value_t = false)]
    pub repeat: bool,

    /// Append every broadcast `Sample` as a JSONL line to this file. Creates
    /// the file if it doesn't exist. The parent directory must exist.
    #[arg(long, env = "MAKANA_RECORD")]
    pub record: Option<PathBuf>,

    /// Rotate the recording file when it reaches this many megabytes. Only
    /// meaningful with `--record`. Must be > 0.
    #[arg(long, env = "MAKANA_RECORD_ROTATE_MB", value_parser = parse_nonzero_u64)]
    pub record_rotate_mb: Option<u64>,

    /// Keep this many rotated files (`<record>.1 .. <record>.<N>`). Only
    /// meaningful with `--record-rotate-mb`. Must be > 0. Default 5.
    #[arg(long, env = "MAKANA_RECORD_KEEP", value_parser = parse_nonzero_u32)]
    pub record_keep: Option<u32>,
}

/// Parse `--pids` value. Each token must be exactly 4 hex digits; first two
/// become `mode`, last two become `pid`.
pub fn parse_pids_arg(s: &str) -> Result<Vec<ObdPid>, String> {
    if s.trim().is_empty() {
        return Err("--pids: expected at least one PID token, got empty string".into());
    }
    let mut out = Vec::new();
    for token in s.split(',') {
        let t = token.trim();
        if t.len() != 4 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "--pids: expected 4 hex digits per token, got '{t}'"
            ));
        }
        let mode = u8::from_str_radix(&t[0..2], 16)
            .map_err(|e| format!("--pids: invalid mode in '{t}': {e}"))?;
        let pid = u8::from_str_radix(&t[2..4], 16)
            .map_err(|e| format!("--pids: invalid pid in '{t}': {e}"))?;
        out.push(ObdPid { mode, pid });
    }
    Ok(out)
}

pub fn parse_positive_speed(s: &str) -> Result<f32, String> {
    let v: f32 = s
        .parse()
        .map_err(|e| format!("--speed: invalid float '{s}': {e}"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("--speed: must be > 0, got {v}"));
    }
    Ok(v)
}

pub fn parse_nonzero_u64(s: &str) -> Result<u64, String> {
    let v: u64 = s
        .parse()
        .map_err(|e| format!("invalid unsigned integer '{s}': {e}"))?;
    if v == 0 {
        return Err("must be > 0".into());
    }
    Ok(v)
}

pub fn parse_nonzero_u32(s: &str) -> Result<u32, String> {
    let v: u32 = s
        .parse()
        .map_err(|e| format!("invalid unsigned integer '{s}': {e}"))?;
    if v == 0 {
        return Err("must be > 0".into());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pids_valid_two() {
        let pids = parse_pids_arg("010C,0105").unwrap();
        assert_eq!(
            pids,
            vec![
                ObdPid {
                    mode: 0x01,
                    pid: 0x0C
                },
                ObdPid {
                    mode: 0x01,
                    pid: 0x05
                },
            ]
        );
    }

    #[test]
    fn parse_pids_valid_three_mixed_case() {
        let pids = parse_pids_arg("010c,010D,0942").unwrap();
        assert_eq!(
            pids,
            vec![
                ObdPid {
                    mode: 0x01,
                    pid: 0x0C
                },
                ObdPid {
                    mode: 0x01,
                    pid: 0x0D
                },
                ObdPid {
                    mode: 0x09,
                    pid: 0x42
                },
            ]
        );
    }

    #[test]
    fn parse_pids_empty_string_errors() {
        let err = parse_pids_arg("").unwrap_err();
        assert!(err.contains("empty string"), "got: {err}");
    }

    #[test]
    fn parse_pids_blank_whitespace_errors() {
        let err = parse_pids_arg("   ").unwrap_err();
        assert!(err.contains("empty string"), "got: {err}");
    }

    #[test]
    fn parse_pids_bad_hex_errors() {
        let err = parse_pids_arg("010X").unwrap_err();
        assert!(
            err.contains("4 hex digits") && err.contains("010X"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_pids_wrong_length_errors() {
        let err = parse_pids_arg("010").unwrap_err();
        assert!(err.contains("4 hex digits"), "got: {err}");
        let err = parse_pids_arg("010CD").unwrap_err();
        assert!(err.contains("4 hex digits"), "got: {err}");
    }

    #[test]
    fn parse_pids_trailing_comma_errors() {
        let err = parse_pids_arg("010C,").unwrap_err();
        assert!(err.contains("4 hex digits"), "got: {err}");
    }

    #[test]
    fn parse_positive_speed_accepts_one() {
        assert_eq!(parse_positive_speed("1.0").unwrap(), 1.0);
        assert_eq!(parse_positive_speed("100").unwrap(), 100.0);
    }

    #[test]
    fn parse_positive_speed_rejects_zero_and_negative() {
        assert!(parse_positive_speed("0").is_err());
        assert!(parse_positive_speed("0.0").is_err());
        assert!(parse_positive_speed("-1.0").is_err());
    }

    #[test]
    fn parse_positive_speed_rejects_nan_and_inf() {
        assert!(parse_positive_speed("nan").is_err());
        assert!(parse_positive_speed("inf").is_err());
    }

    #[test]
    fn parse_nonzero_u64_rejects_zero() {
        assert!(parse_nonzero_u64("0").is_err());
        assert_eq!(parse_nonzero_u64("1").unwrap(), 1);
        assert_eq!(parse_nonzero_u64("1024").unwrap(), 1024);
    }

    #[test]
    fn parse_nonzero_u32_rejects_zero() {
        assert!(parse_nonzero_u32("0").is_err());
        assert_eq!(parse_nonzero_u32("1").unwrap(), 1);
        assert_eq!(parse_nonzero_u32("5").unwrap(), 5);
    }

    #[test]
    fn cli_rejects_zero_rotate_mb() {
        let res = Cli::try_parse_from([
            "makana-backend",
            "--record",
            "/tmp/x.jsonl",
            "--record-rotate-mb",
            "0",
        ]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_rejects_zero_keep() {
        let res = Cli::try_parse_from([
            "makana-backend",
            "--record",
            "/tmp/x.jsonl",
            "--record-rotate-mb",
            "1",
            "--record-keep",
            "0",
        ]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_accepts_rotate_flags() {
        let cli = Cli::try_parse_from([
            "makana-backend",
            "--record",
            "/tmp/x.jsonl",
            "--record-rotate-mb",
            "10",
            "--record-keep",
            "3",
        ])
        .unwrap();
        assert_eq!(cli.record_rotate_mb, Some(10));
        assert_eq!(cli.record_keep, Some(3));
    }
}
