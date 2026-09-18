//! Shared data model for makana.
//!
//! Every transport produces `Sample`s. The backend broadcasts them. The firmware
//! serializes them to JSON over WebSocket. One type, one representation.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch. Portable across desktop and embedded.
pub type Millis = u64;

/// A raw CAN frame. `id` is 11-bit (standard) or 29-bit (extended).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanFrame {
    pub id: u32,
    pub extended: bool,
    pub data: Vec<u8>,
}

/// Identifies an OBD-II Parameter ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObdPid {
    /// OBD mode (aka service). 0x01 = current data, 0x03 = stored DTCs, etc.
    pub mode: u8,
    /// PID number. Some modes (e.g. 0x03) have no PID; use 0 there.
    pub pid: u8,
}

impl ObdPid {
    pub const ENGINE_LOAD: Self = Self {
        mode: 0x01,
        pid: 0x04,
    };
    pub const COOLANT_TEMP: Self = Self {
        mode: 0x01,
        pid: 0x05,
    };
    pub const RPM: Self = Self {
        mode: 0x01,
        pid: 0x0C,
    };
    pub const SPEED: Self = Self {
        mode: 0x01,
        pid: 0x0D,
    };
    pub const INTAKE_TEMP: Self = Self {
        mode: 0x01,
        pid: 0x0F,
    };
    pub const THROTTLE_POS: Self = Self {
        mode: 0x01,
        pid: 0x11,
    };
    pub const FUEL_LEVEL: Self = Self {
        mode: 0x01,
        pid: 0x2F,
    };
    pub const BATTERY_VOLTAGE: Self = Self {
        mode: 0x01,
        pid: 0x42,
    };
}

/// A decoded OBD-II reading. Variants carry their unit in the name so the
/// frontend never has to guess.
///
/// Anything we haven't taught the decoder yet lands in `Raw` with original
/// bytes preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ObdValue {
    EngineLoadPercent(f32),
    CoolantTempC(i16),
    Rpm(u32),
    SpeedKph(u16),
    IntakeTempC(i16),
    ThrottlePercent(f32),
    FuelPercent(f32),
    BatteryVolts(f32),
    /// Response whose semantics the decoder doesn't know yet.
    Raw {
        pid: ObdPid,
        bytes: Vec<u8>,
    },
    /// Stored diagnostic trouble codes (mode 03). Empty Vec = no codes.
    Dtcs(Vec<Dtc>),
}

/// OBD-II DTC system letter. Top two bits of the first DTC byte per SAE J2012.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DtcSystem {
    Powertrain,
    Chassis,
    Body,
    Network,
}

/// A diagnostic trouble code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dtc {
    pub system: DtcSystem,
    /// 4-hex-digit numeric portion. 0x0301 prints as "0301" in "P0301".
    pub code: u16,
}

impl Dtc {
    /// Decode a DTC from the two bytes emitted by OBD-II mode 03 (SAE J2012).
    pub fn from_bytes(a: u8, b: u8) -> Self {
        let system = match (a >> 6) & 0b11 {
            0 => DtcSystem::Powertrain,
            1 => DtcSystem::Chassis,
            2 => DtcSystem::Body,
            _ => DtcSystem::Network,
        };
        let digit1 = u16::from((a >> 4) & 0b11);
        let digit2 = u16::from(a & 0x0F);
        let digit3 = u16::from((b >> 4) & 0x0F);
        let digit4 = u16::from(b & 0x0F);
        let code = (digit1 << 12) | (digit2 << 8) | (digit3 << 4) | digit4;
        Self { system, code }
    }

    /// Format as a 5-char OBD-II code, e.g. "P0301".
    pub fn to_obd_code(&self) -> String {
        let letter = match self.system {
            DtcSystem::Powertrain => 'P',
            DtcSystem::Chassis => 'C',
            DtcSystem::Body => 'B',
            DtcSystem::Network => 'U',
        };
        format!("{letter}{:04X}", self.code)
    }
}

/// Unified envelope. Every transport emits these; every consumer reads these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Source identifier, e.g. "elm327:/dev/ttys003" or "esp32:garage". Lets
    /// the UI show which feed a data point came from when multiple sources run.
    pub source: String,
    pub timestamp_ms: Millis,
    pub payload: SamplePayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SamplePayload {
    Obd(ObdValue),
    Can(CanFrame),
    /// Source-level events (connect, disconnect, error). Useful for UI liveness.
    Event(SourceEvent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SourceEvent {
    Connected,
    Disconnected,
    Error { message: String },
}

/// Error type shared across transports. Each transport wraps its own lower-level
/// errors into this.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol: {0}")]
    Protocol(String),

    #[error("timeout")]
    Timeout,

    #[error("closed")]
    Closed,

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_roundtrips_through_json() {
        let s = Sample {
            source: "mock".into(),
            timestamp_ms: 1_700_000_000_000,
            payload: SamplePayload::Obd(ObdValue::Rpm(3500)),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Sample = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn can_frame_roundtrips() {
        let f = CanFrame {
            id: 0x7E8,
            extended: false,
            data: vec![0x41, 0x0C, 0x1A, 0xF8],
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: CanFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn dtc_powertrain() {
        let d = Dtc::from_bytes(0x03, 0x01);
        assert_eq!(d.system, DtcSystem::Powertrain);
        assert_eq!(d.to_obd_code(), "P0301");
    }

    #[test]
    fn dtc_chassis() {
        let d = Dtc::from_bytes(0x43, 0x01);
        assert_eq!(d.system, DtcSystem::Chassis);
        assert_eq!(d.to_obd_code(), "C0301");
    }

    #[test]
    fn dtc_body() {
        let d = Dtc::from_bytes(0x83, 0x01);
        assert_eq!(d.system, DtcSystem::Body);
        assert_eq!(d.to_obd_code(), "B0301");
    }

    #[test]
    fn dtc_network() {
        let d = Dtc::from_bytes(0xC3, 0x01);
        assert_eq!(d.system, DtcSystem::Network);
        assert_eq!(d.to_obd_code(), "U0301");
    }

    #[test]
    fn dtc_zero() {
        assert_eq!(Dtc::from_bytes(0x00, 0x00).to_obd_code(), "P0000");
    }

    #[test]
    fn dtcs_roundtrip() {
        let v = ObdValue::Dtcs(vec![
            Dtc::from_bytes(0x03, 0x01),
            Dtc::from_bytes(0xC1, 0x23),
        ]);
        let json = serde_json::to_string(&v).unwrap();
        let back: ObdValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
        assert!(json.contains("\"kind\":\"dtcs\""));
        assert!(json.contains("\"system\":\"powertrain\""));
    }

    #[test]
    fn dtcs_empty_roundtrip() {
        let v = ObdValue::Dtcs(vec![]);
        let json = serde_json::to_string(&v).unwrap();
        let back: ObdValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
