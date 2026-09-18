//! ELM327 serial transport.
//!
//! Works against real ELM327-compatible OBD-II dongles and against the
//! `ELM327-emulator` Python project over a pseudo-tty. Same code path.
//!
//! Protocol cheatsheet:
//! - Commands end with `\r`.
//! - Responses end with `\r\r>` (the `>` is the ready prompt).
//! - Mode 01 responses start with `0x41` (= `0x40 | mode`), then the PID, then
//!   the data bytes. Example: query `010C` (RPM) → `41 0C 1A F8`.
//! - "NO DATA", "UNABLE TO CONNECT", "SEARCHING..." are control strings, not
//!   data frames. We skip them and surface as source events or errors.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::stream::StreamExt;
use makana_common::{Dtc, ObdPid, ObdValue, Sample, SamplePayload, SourceEvent, TransportError};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_serial::SerialPortBuilderExt;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::{CarDataSource, SampleStream};

/// Anything that reads and writes bytes asynchronously. Lets us use both a
/// real serial port (tokio-serial) and a plain file (a pty opened directly)
/// behind the same code path.
trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + ?Sized> Duplex for T {}

/// Default PID rotation. Conservative — every ELM327-compatible ECU answers these.
pub const DEFAULT_PIDS: &[ObdPid] = &[
    ObdPid::RPM,
    ObdPid::SPEED,
    ObdPid::COOLANT_TEMP,
    ObdPid::INTAKE_TEMP,
    ObdPid::THROTTLE_POS,
    ObdPid::ENGINE_LOAD,
    ObdPid::BATTERY_VOLTAGE,
];

/// Default cadence for mode-03 DTC queries. Slower than PID polling since
/// stored codes don't change millisecond-to-millisecond.
pub const DTC_QUERY_INTERVAL: Duration = Duration::from_secs(10);

/// Initial reconnect backoff.
const RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_millis(500);

/// Maximum reconnect backoff.
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Doubles `d` with a cap of 30s. Pure — unit-testable.
fn next_backoff(d: Duration) -> Duration {
    let doubled = d.saturating_mul(2);
    if doubled > RECONNECT_BACKOFF_MAX {
        RECONNECT_BACKOFF_MAX
    } else {
        doubled
    }
}

pub struct Elm327Source {
    device: String,
    baud: u32,
    pids: Vec<ObdPid>,
    poll_interval: Duration,
    query_timeout: Duration,
    dtc_interval: Duration,
    discover: bool,
}

impl Elm327Source {
    pub fn new(device: impl Into<String>) -> Self {
        Self {
            device: device.into(),
            baud: 38_400,
            pids: DEFAULT_PIDS.to_vec(),
            poll_interval: Duration::from_millis(200),
            query_timeout: Duration::from_millis(750),
            dtc_interval: DTC_QUERY_INTERVAL,
            discover: true,
        }
    }

    pub fn with_baud(mut self, baud: u32) -> Self {
        self.baud = baud;
        self
    }

    pub fn with_pids(mut self, pids: Vec<ObdPid>) -> Self {
        self.pids = pids;
        self
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn with_dtc_interval(mut self, interval: Duration) -> Self {
        self.dtc_interval = interval;
        self
    }

    pub fn with_discovery(mut self, on: bool) -> Self {
        self.discover = on;
        self
    }
}

#[async_trait::async_trait]
impl CarDataSource for Elm327Source {
    fn name(&self) -> &str {
        &self.device
    }

    async fn start(self: Box<Self>) -> Result<SampleStream, TransportError> {
        let (tx, rx) = mpsc::channel::<Result<Sample, TransportError>>(64);
        let source_name = format!("elm327:{}", self.device);

        tokio::spawn(run_loop(
            self.device,
            self.baud,
            source_name,
            self.pids,
            self.poll_interval,
            self.query_timeout,
            self.dtc_interval,
            self.discover,
            tx,
        ));

        Ok(ReceiverStream::new(rx).boxed())
    }
}

/// Open the device and return a read/write handle.
///
/// Strategy: if the path is a known pty pattern (macOS `/dev/ttys*`, Linux
/// `/dev/pts/*`), open it directly as a plain file — in-memory ptys don't
/// accept the `tcsetattr` calls that `tokio-serial` makes for baud setup.
/// Otherwise try `tokio-serial` first (needed for USB/BT dongles where baud
/// rate matters), and fall back to a plain file open if the serial crate
/// reports ENOTTY anyway.
async fn open_device(path: &str, baud: u32) -> Result<Box<dyn Duplex>, TransportError> {
    if is_pty_path(path) {
        debug!(path, "pty path detected; opening as plain file");
        return open_as_file(path).await;
    }
    match tokio_serial::new(path, baud)
        .timeout(Duration::from_millis(500))
        .open_native_async()
    {
        Ok(p) => Ok(Box::new(p)),
        Err(e) => {
            let io_err = std::io::Error::from(e);
            if looks_like_enotty(&io_err) {
                debug!(
                    path,
                    "serial open returned ENOTTY; falling back to plain file"
                );
                return open_as_file(path).await;
            }
            Err(TransportError::Io(io_err))
        }
    }
}

async fn open_as_file(path: &str) -> Result<Box<dyn Duplex>, TransportError> {
    let f = OpenOptions::new().read(true).write(true).open(path).await?;
    Ok(Box::new(f))
}

/// Macro-stable pty paths across the platforms we care about.
fn is_pty_path(path: &str) -> bool {
    // macOS: slaves are /dev/ttys002, /dev/ttys003, ... (note: short `ttys`
    // prefix, not `tty.` which is for real serial devices).
    // Linux: slaves are /dev/pts/0, /dev/pts/1, ...
    if let Some(rest) = path.strip_prefix("/dev/ttys") {
        return rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty();
    }
    path.starts_with("/dev/pts/")
}

fn looks_like_enotty(err: &std::io::Error) -> bool {
    if err.raw_os_error() == Some(25) {
        return true;
    }
    // serialport flattens nix errors into `io::Error::other(_)` with no
    // `raw_os_error`. Fall back to message sniffing — the OS-provided text
    // for ENOTTY is stable across libc versions.
    let msg = err.to_string();
    msg.contains("Not a typewriter") || msg.contains("Inappropriate ioctl")
}

#[cfg(test)]
mod path_tests {
    use super::is_pty_path;

    #[test]
    fn detects_macos_pty() {
        assert!(is_pty_path("/dev/ttys002"));
        assert!(is_pty_path("/dev/ttys999"));
    }

    #[test]
    fn detects_linux_pty() {
        assert!(is_pty_path("/dev/pts/0"));
    }

    #[test]
    fn rejects_real_serial() {
        // macOS USB-serial dongles
        assert!(!is_pty_path("/dev/cu.usbserial-A1B2"));
        assert!(!is_pty_path("/dev/tty.usbserial-A1B2"));
        // Linux
        assert!(!is_pty_path("/dev/ttyUSB0"));
        assert!(!is_pty_path("/dev/ttyACM0"));
        assert!(!is_pty_path("/dev/ttyS0"));
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    device: String,
    baud: u32,
    source: String,
    pids_configured: Vec<ObdPid>,
    poll_interval: Duration,
    query_timeout: Duration,
    dtc_interval: Duration,
    discover: bool,
    tx: mpsc::Sender<Result<Sample, TransportError>>,
) {
    let mut backoff = RECONNECT_BACKOFF_INITIAL;
    let mut connected_once = false;

    loop {
        if tx.is_closed() {
            return;
        }

        let open_result = open_device(&device, baud).await;
        let connect_outcome = match open_result {
            Ok(mut port) => match init_adapter(&mut *port, query_timeout).await {
                Ok(()) => Ok(port),
                Err(e) => Err(format!("init failed: {e}")),
            },
            Err(e) => Err(format!("open failed: {e}")),
        };

        let mut port = match connect_outcome {
            Ok(p) => p,
            Err(msg) => {
                warn!(%msg, "elm327 connect attempt failed");
                if emit_event(&tx, &source, SourceEvent::Error { message: msg })
                    .await
                    .is_err()
                {
                    return;
                }
                if connected_once
                    && emit_event(&tx, &source, SourceEvent::Disconnected)
                        .await
                        .is_err()
                {
                    return;
                }
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
                continue;
            }
        };

        if emit_event(&tx, &source, SourceEvent::Connected)
            .await
            .is_err()
        {
            return;
        }
        connected_once = true;
        backoff = RECONNECT_BACKOFF_INITIAL;

        let pids_active =
            resolve_active_pids(&mut *port, &pids_configured, discover, query_timeout).await;

        match poll_loop(
            &mut *port,
            &source,
            &pids_active,
            poll_interval,
            query_timeout,
            dtc_interval,
            &tx,
        )
        .await
        {
            PollExit::ConsumerGone => return,
            PollExit::Fatal => {
                drop(port);
                if emit_event(&tx, &source, SourceEvent::Disconnected)
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
            }
        }
    }
}

enum PollExit {
    ConsumerGone,
    Fatal,
}

async fn emit_event(
    tx: &mpsc::Sender<Result<Sample, TransportError>>,
    source: &str,
    event: SourceEvent,
) -> Result<(), ()> {
    tx.send(Ok(Sample {
        source: source.to_string(),
        timestamp_ms: now_ms(),
        payload: SamplePayload::Event(event),
    }))
    .await
    .map_err(|_| ())
}

async fn poll_loop(
    port: &mut dyn Duplex,
    source: &str,
    pids: &[ObdPid],
    poll_interval: Duration,
    query_timeout: Duration,
    dtc_interval: Duration,
    tx: &mpsc::Sender<Result<Sample, TransportError>>,
) -> PollExit {
    let mut interval = tokio::time::interval(poll_interval);
    let mut idx = 0usize;
    let mut last_dtc = Instant::now();

    loop {
        interval.tick().await;

        if last_dtc.elapsed() >= dtc_interval {
            last_dtc = Instant::now();
            match query(port, "03", query_timeout).await {
                Ok(reply) => {
                    let dtcs = decode_dtcs(&reply);
                    let sample = Sample {
                        source: source.to_string(),
                        timestamp_ms: now_ms(),
                        payload: SamplePayload::Obd(ObdValue::Dtcs(dtcs)),
                    };
                    if tx.send(Ok(sample)).await.is_err() {
                        return PollExit::ConsumerGone;
                    }
                }
                Err(TransportError::Timeout) => {
                    debug!("dtc query timed out");
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return PollExit::Fatal;
                }
            }
        }

        if pids.is_empty() {
            continue;
        }
        let pid = pids[idx % pids.len()];
        idx = idx.wrapping_add(1);

        let cmd = format!("{:02X}{:02X}", pid.mode, pid.pid);
        let reply = match query(port, &cmd, query_timeout).await {
            Ok(r) => r,
            Err(TransportError::Timeout) => {
                debug!(%cmd, "pid query timed out");
                continue;
            }
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return PollExit::Fatal;
            }
        };

        let payload = match decode_response(pid, &reply) {
            Some(v) => SamplePayload::Obd(v),
            None => continue,
        };

        let sample = Sample {
            source: source.to_string(),
            timestamp_ms: now_ms(),
            payload,
        };
        if tx.send(Ok(sample)).await.is_err() {
            debug!("elm327 consumer dropped");
            return PollExit::ConsumerGone;
        }
    }
}

/// Send the ELM327 init sequence. Each command expects a `>` prompt back.
async fn init_adapter(port: &mut dyn Duplex, t: Duration) -> Result<(), TransportError> {
    // ATZ: reset. The adapter prints a banner ("ELM327 v1.5") before the prompt.
    // Longer timeout than normal queries — reset can take ~1s on real hardware.
    let _ = timeout(t * 3, send_and_wait(port, "ATZ"))
        .await
        .map_err(|_| TransportError::Timeout)??;
    // Echo off, linefeeds off, headers off, auto-protocol.
    for cmd in ["ATE0", "ATL0", "ATH0", "ATSP0"] {
        send_and_wait(port, cmd).await?;
    }
    Ok(())
}

/// Send `cmd\r` and read until the `>` prompt. Returns the raw reply buffer
/// (without prompt).
async fn query(port: &mut dyn Duplex, cmd: &str, t: Duration) -> Result<String, TransportError> {
    match timeout(t, send_and_wait(port, cmd)).await {
        Ok(r) => r,
        Err(_) => Err(TransportError::Timeout),
    }
}

async fn send_and_wait(port: &mut dyn Duplex, cmd: &str) -> Result<String, TransportError> {
    port.write_all(cmd.as_bytes()).await?;
    port.write_all(b"\r").await?;
    port.flush().await?;

    let mut buf = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    loop {
        let n = port.read(&mut byte).await?;
        if n == 0 {
            return Err(TransportError::Closed);
        }
        if byte[0] == b'>' {
            break;
        }
        buf.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Parse an ELM327 reply and decode to a typed value if we know this PID.
///
/// Replies look like (whitespace varies):
///   "41 0C 1A F8\r\r"  → mode 0x41, pid 0x0C, two data bytes
///   "NO DATA\r\r"      → skip
///   "SEARCHING...\r41 0C 1A F8\r\r" → we tolerate leading noise
fn decode_response(pid: ObdPid, reply: &str) -> Option<ObdValue> {
    let expect_mode = pid.mode | 0x40;

    // Find the line whose first two hex bytes match (mode-echo, pid). Tolerates
    // "SEARCHING...", echoed commands when ATE0 didn't take, adapter noise, and
    // spaceless responses like "410C1AF8".
    let data_line = reply.split(['\r', '\n']).find(|line| {
        let bytes = hex_bytes(line);
        bytes.len() >= 2 && bytes[0] == expect_mode && bytes[1] == pid.pid
    })?;

    let bytes = hex_bytes(data_line);
    // bytes[0] = mode-echo, bytes[1] = pid, bytes[2..] = payload
    let data = &bytes[2..];

    Some(match pid {
        ObdPid::RPM if data.len() >= 2 => {
            // ((A * 256) + B) / 4
            let raw = (u32::from(data[0]) << 8) | u32::from(data[1]);
            ObdValue::Rpm(raw / 4)
        }
        ObdPid::SPEED if !data.is_empty() => ObdValue::SpeedKph(u16::from(data[0])),
        ObdPid::COOLANT_TEMP if !data.is_empty() => ObdValue::CoolantTempC(i16::from(data[0]) - 40),
        ObdPid::INTAKE_TEMP if !data.is_empty() => ObdValue::IntakeTempC(i16::from(data[0]) - 40),
        ObdPid::THROTTLE_POS if !data.is_empty() => {
            // A * 100 / 255
            ObdValue::ThrottlePercent(f32::from(data[0]) * 100.0 / 255.0)
        }
        ObdPid::ENGINE_LOAD if !data.is_empty() => {
            ObdValue::EngineLoadPercent(f32::from(data[0]) * 100.0 / 255.0)
        }
        ObdPid::FUEL_LEVEL if !data.is_empty() => {
            ObdValue::FuelPercent(f32::from(data[0]) * 100.0 / 255.0)
        }
        ObdPid::BATTERY_VOLTAGE if data.len() >= 2 => {
            // ((A * 256) + B) / 1000 volts
            let raw = (u32::from(data[0]) << 8) | u32::from(data[1]);
            ObdValue::BatteryVolts(raw as f32 / 1000.0)
        }
        _ => {
            warn!(?pid, ?data, "no decoder for pid; emitting raw");
            ObdValue::Raw {
                pid,
                bytes: data.to_vec(),
            }
        }
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
/// Extract hex byte pairs from a line. Accepts both spaced (`"41 0C 1A F8"`) and
/// spaceless (`"410C1AF8"`) forms. Ignores non-hex tokens.
fn hex_bytes(line: &str) -> Vec<u8> {
    let compact: String = line.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..compact.len())
        .step_by(2)
        .take(compact.len() / 2)
        .filter_map(|i| u8::from_str_radix(&compact[i..i + 2], 16).ok())
        .collect()
}

/// Decode an OBD-II "supported PIDs" bitmap into the list of concrete PIDs in
/// this range plus whether the next range is queryable.
///
/// Bit layout (OBD-II / SAE J1979): bit 7 of byte 0 = PID `base+1`, bit 6 of
/// byte 0 = PID `base+2`, ..., bit 1 of byte 3 = PID `base+31`. The lowest bit
/// of byte 3 (bit index 31 of the 32-bit map) is the "continues" flag: when
/// set, the next range's metadata PID (`0120`, `0140`, ...) is answerable. It
/// is treated purely as a continuation signal, not as a polling PID of its
/// own — querying 0x20/0x40/0x60/0x80 returns more metadata, not vehicle data.
fn supported_pids_from_bitmap(base: u8, bytes: [u8; 4]) -> (Vec<ObdPid>, bool) {
    let mut out = Vec::new();
    let bitmap = u32::from_be_bytes(bytes);
    // bit_index 0..=30: regular data PIDs. bit_index 31: continuation only.
    for bit_index in 0u32..31 {
        let mask = 1u32 << (31 - bit_index);
        if bitmap & mask != 0 {
            out.push(ObdPid {
                mode: 0x01,
                pid: base + (bit_index as u8) + 1,
            });
        }
    }
    let continues = bytes[3] & 0x01 != 0;
    (out, continues)
}

/// Query a single supported-PIDs metadata PID (`0100`, `0120`, ...) and return
/// the 4 data bytes, or None if the query failed or the reply was malformed.
async fn query_supported_bitmap(port: &mut dyn Duplex, base: u8, t: Duration) -> Option<[u8; 4]> {
    let cmd = format!("01{:02X}", base);
    // 0100 on a fresh connection often triggers a "SEARCHING..." ECU probe
    // that delays the first real reply by a second or two. Use a longer
    // window than normal PID polling so we don't time out mid-search and
    // leave a buffered reply that poisons the next read.
    let reply = query(port, &cmd, t * 4).await.ok()?;
    let line = reply.split(['\r', '\n']).find(|l| {
        let b = hex_bytes(l);
        b.len() >= 2 && b[0] == 0x41 && b[1] == base
    })?;
    let bytes = hex_bytes(line);
    if bytes.len() < 6 {
        return None;
    }
    Some([bytes[2], bytes[3], bytes[4], bytes[5]])
}

/// Query the ECU's supported-PID bitmaps and return the set of PIDs that
/// will actually answer. Returns `None` on any failure (engine off, ECU
/// silent, malformed reply) so the caller can fall back to the configured
/// list rather than polling an incorrect subset.
async fn discover_supported_pids(
    port: &mut dyn Duplex,
    t: Duration,
) -> Option<std::collections::HashSet<ObdPid>> {
    let mut set = std::collections::HashSet::new();
    // Stop after 0x60: 0x61-0x80 is the last range we probe. Bit 0 of that
    // reply would indicate 0x81+ is also supported, but we do not go there.
    for base in [0x00u8, 0x20, 0x40, 0x60] {
        let bytes = query_supported_bitmap(port, base, t).await?;
        let (pids, continues) = supported_pids_from_bitmap(base, bytes);
        set.extend(pids);
        if !continues {
            break;
        }
    }
    Some(set)
}

/// Apply PID discovery on top of the configured list. Runs on every
/// (re)connect because which PIDs the ECU reports can change when the engine
/// is re-initialized (e.g. after an ignition cycle). Never panics; always
/// returns a usable polling list.
async fn resolve_active_pids(
    port: &mut dyn Duplex,
    pids_configured: &[ObdPid],
    discover: bool,
    query_timeout: Duration,
) -> Vec<ObdPid> {
    if !discover {
        return pids_configured.to_vec();
    }
    if pids_configured.is_empty() {
        warn!("configured pids list is empty; skipping discovery filtering");
        return pids_configured.to_vec();
    }
    match discover_supported_pids(port, query_timeout).await {
        Some(set) => {
            let filtered: Vec<ObdPid> = pids_configured
                .iter()
                .copied()
                .filter(|p| set.contains(p))
                .collect();
            if filtered.is_empty() {
                warn!(
                    "ECU supports {} PIDs but none overlap with configured list; falling back to configured list",
                    set.len()
                );
                pids_configured.to_vec()
            } else {
                info!(
                    "ECU supports {} PIDs; polling {} of them",
                    set.len(),
                    filtered.len()
                );
                filtered
            }
        }
        None => {
            warn!("PID discovery failed; using configured list as-is");
            pids_configured.to_vec()
        }
    }
}

/// Parse a mode-03 reply into a deduplicated list of DTCs.
///
/// Replies look like:
///   "43 03 01\r\r"             → [P0301]
///   "43 00 00\r\r"             → []  (no codes / padding)
///   "43 03 01 43 02 01\r\r"    → [P0301, P0201]  (two ECUs same line)
///   "43 03 01\r43 00 00\r\r"   → [P0301]  (multi-line, second empty)
///
/// Each `0x43` byte starts a new ECU group. A `(0x00, 0x00)` pair within a
/// group marks padding/end of that group. Trailing odd bytes are ignored.
fn decode_dtcs(reply: &str) -> Vec<Dtc> {
    use std::collections::HashSet;
    let mut out = Vec::new();
    let mut seen: HashSet<Dtc> = HashSet::new();
    for line in reply.split(['\r', '\n']) {
        let bytes = hex_bytes(line);
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != 0x43 {
                i += 1;
                continue;
            }
            i += 1;
            while i + 1 < bytes.len() && bytes[i] != 0x43 {
                let (a, b) = (bytes[i], bytes[i + 1]);
                if a == 0 && b == 0 {
                    i += 2;
                    while i < bytes.len() && bytes[i] != 0x43 {
                        i += 1;
                    }
                    break;
                }
                let dtc = Dtc::from_bytes(a, b);
                if seen.insert(dtc) {
                    out.push(dtc);
                }
                i += 2;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rpm() {
        // 0x1AF8 / 4 = 1726
        let v = decode_response(ObdPid::RPM, "41 0C 1A F8\r\r").unwrap();
        assert_eq!(v, ObdValue::Rpm(1726));
    }

    #[test]
    fn decodes_speed() {
        let v = decode_response(ObdPid::SPEED, "41 0D 3C\r\r").unwrap();
        assert_eq!(v, ObdValue::SpeedKph(60));
    }

    #[test]
    fn decodes_spaceless_response() {
        let v = decode_response(ObdPid::RPM, "410C1AF8\r\r").unwrap();
        assert_eq!(v, ObdValue::Rpm(1726));
    }

    #[test]
    fn decodes_coolant_below_zero() {
        // raw 0x1E = 30 → 30 - 40 = -10 C
        let v = decode_response(ObdPid::COOLANT_TEMP, "41 05 1E\r\r").unwrap();
        assert_eq!(v, ObdValue::CoolantTempC(-10));
    }

    #[test]
    fn tolerates_searching_prefix() {
        let reply = "SEARCHING...\r41 0D 64\r\r";
        let v = decode_response(ObdPid::SPEED, reply).unwrap();
        assert_eq!(v, ObdValue::SpeedKph(100));
    }

    #[test]
    fn rejects_wrong_pid() {
        // reply is for PID 0x0C but we queried 0x0D
        assert!(decode_response(ObdPid::SPEED, "41 0C 1A F8\r\r").is_none());
    }

    #[test]
    fn rejects_no_data() {
        assert!(decode_response(ObdPid::RPM, "NO DATA\r\r").is_none());
    }

    #[test]
    fn decodes_single_dtc() {
        let v = decode_dtcs("43 03 01\r\r");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].to_obd_code(), "P0301");
    }

    #[test]
    fn decodes_empty_dtc_response() {
        assert!(decode_dtcs("43 00 00\r\r").is_empty());
    }

    #[test]
    fn decodes_two_ecus_same_line() {
        let v = decode_dtcs("43 03 01 43 02 01\r\r");
        let codes: Vec<String> = v.iter().map(|d| d.to_obd_code()).collect();
        assert_eq!(codes, vec!["P0301", "P0201"]);
    }

    #[test]
    fn decodes_multi_line_second_empty() {
        let v = decode_dtcs("43 03 01\r43 00 00\r\r");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].to_obd_code(), "P0301");
    }

    #[test]
    fn decodes_dedupe_across_ecus() {
        // Same code reported by two ECUs must appear once.
        let v = decode_dtcs("43 03 01\r43 03 01\r\r");
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut d = RECONNECT_BACKOFF_INITIAL;
        assert_eq!(d, Duration::from_millis(500));
        d = next_backoff(d);
        assert_eq!(d, Duration::from_secs(1));
        d = next_backoff(d);
        assert_eq!(d, Duration::from_secs(2));
        for _ in 0..20 {
            d = next_backoff(d);
        }
        assert_eq!(d, RECONNECT_BACKOFF_MAX);
        assert_eq!(d, Duration::from_secs(30));
        // Stays capped.
        assert_eq!(next_backoff(d), RECONNECT_BACKOFF_MAX);
    }

    #[test]
    fn bitmap_decodes_spotcheck() {
        // Canonical example from the OBD-II spec: 0100 -> BE 1F A8 13.
        let (pids, cont) = supported_pids_from_bitmap(0x00, [0xBE, 0x1F, 0xA8, 0x13]);
        let set: std::collections::HashSet<u8> = pids.iter().map(|p| p.pid).collect();
        // Every PID must be mode 01.
        assert!(pids.iter().all(|p| p.mode == 0x01));
        // Byte 0 = 0xBE = 10111110 -> PIDs 0x01, 0x03, 0x04, 0x05, 0x06, 0x07.
        for pid in [0x01u8, 0x03, 0x04, 0x05, 0x06, 0x07] {
            assert!(set.contains(&pid), "missing PID {:#04x}", pid);
        }
        assert!(!set.contains(&0x02));
        assert!(!set.contains(&0x08));
        // Byte 1 = 0x1F = 00011111 -> PIDs 0x0C, 0x0D, 0x0E, 0x0F, 0x10.
        for pid in [0x0Cu8, 0x0D, 0x0E, 0x0F, 0x10] {
            assert!(set.contains(&pid), "missing PID {:#04x}", pid);
        }
        // Byte 2 bit 7 = 1 -> PID 0x11.
        assert!(set.contains(&0x11));
        // 0x20 is the continuation bit, not a pollable PID -> excluded.
        assert!(!set.contains(&0x20));
        assert!(cont, "bit 0 of byte 3 is set -> next range queryable");
    }

    #[test]
    fn bitmap_all_zero_no_pids_no_continue() {
        let (pids, cont) = supported_pids_from_bitmap(0x00, [0x00, 0x00, 0x00, 0x00]);
        assert!(pids.is_empty());
        assert!(!cont);
    }

    #[test]
    fn bitmap_single_pid_and_continue() {
        let (pids, cont) = supported_pids_from_bitmap(0x00, [0x80, 0x00, 0x00, 0x01]);
        assert_eq!(pids.len(), 1);
        assert_eq!(
            pids[0],
            ObdPid {
                mode: 0x01,
                pid: 0x01
            }
        );
        assert!(cont);
    }

    #[test]
    fn bitmap_continue_only_bit_yields_no_pids() {
        // Base 0x20, only the continuation bit set. Bit index 31 must not be
        // added as a data PID even though the bit is 1.
        let (pids, cont) = supported_pids_from_bitmap(0x20, [0x00, 0x00, 0x00, 0x01]);
        assert!(pids.is_empty(), "got unexpected PIDs: {:?}", pids);
        assert!(cont);
    }

    #[test]
    fn with_discovery_flows_through() {
        let s = Elm327Source::new("/dev/null");
        assert!(s.discover, "default must be discovery on");
        let s = s.with_discovery(false);
        assert!(!s.discover);
        let s = s.with_discovery(true);
        assert!(s.discover);
    }
}
