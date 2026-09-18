//! Serde types matching the Bambu Lab X1C MQTT JSON report format.

use serde::Deserialize;

/// Top-level MQTT message envelope.
#[derive(Debug, Deserialize)]
pub struct BambuReport {
    #[serde(default)]
    pub print: Option<PrintStatus>,
}

/// The `print` object inside the report.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct PrintStatus {
    pub command: String,
    pub nozzle_temper: f64,
    pub nozzle_target_temper: f64,
    pub bed_temper: f64,
    pub bed_target_temper: f64,
    pub gcode_state: String,
    pub mc_percent: u32,
    pub mc_remaining_time: u32,
    pub layer_num: u32,
    pub total_layer_num: u32,
    pub subtask_name: String,
    pub gcode_file: String,
    pub spd_lvl: u32,
    pub spd_mag: u32,
    pub cooling_fan_speed: String,
    pub big_fan1_speed: String,
    pub big_fan2_speed: String,
    pub heatbreak_fan_speed: String,
    pub fan_gear: u32,
    pub wifi_signal: String,
    pub ams: Option<AmsStatus>,
    #[serde(default)]
    pub lights_report: Vec<LightReport>,
    pub ipcam: Option<IpcamStatus>,
    pub err: String,
    pub print_error: u32,
    pub mc_print_error_code: String,
    #[serde(default)]
    pub hms: Vec<serde_json::Value>,
    pub nozzle_diameter: String,
    pub nozzle_type: String,
    pub sdcard: bool,
    pub info: Option<InfoBlock>,
    pub device: Option<DeviceBlock>,
    pub state: u32,
    pub sequence_id: String,
    pub task_id: String,
    pub subtask_id: String,
    pub xcam: Option<XcamSettings>,
    pub upgrade_state: Option<UpgradeStateBlock>,
    /// Printer firmware version as string (e.g. "20000").
    pub ver: String,
    pub net: Option<NetBlock>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AmsStatus {
    pub ams: Vec<AmsUnit>,
    pub tray_now: String,
    pub tray_tar: String,
    pub ams_exist_bits: String,
    pub tray_exist_bits: String,
    pub tray_is_bbl_bits: String,
    pub insert_flag: bool,
    pub version: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AmsUnit {
    pub id: String,
    pub humidity: String,
    pub temp: String,
    pub tray: Vec<AmsTray>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AmsTray {
    pub id: String,
    pub tray_type: String,
    pub tray_color: String,
    pub tray_sub_brands: String,
    pub tray_id_name: String,
    pub nozzle_temp_min: String,
    pub nozzle_temp_max: String,
    pub remain: u32,
    pub tag_uid: String,
    pub tray_uuid: String,
    pub tray_diameter: String,
    pub tray_weight: String,
    pub bed_temp: String,
    pub drying_temp: String,
    pub drying_time: String,
}

#[derive(Debug, Deserialize)]
pub struct LightReport {
    pub node: String,
    pub mode: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct IpcamStatus {
    pub rtsp_url: String,
    pub resolution: String,
    pub timelapse: String,
    pub ipcam_record: String,
    pub ipcam_dev: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct InfoBlock {
    pub temp: i32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct DeviceBlock {
    pub ctc: Option<CtcBlock>,
    pub nozzle: Option<NozzleBlock>,
    pub plate: Option<PlateBlock>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct CtcBlock {
    pub info: Option<InfoBlock>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct XcamSettings {
    pub spaghetti_detector: bool,
    pub print_halt: bool,
    pub first_layer_inspector: bool,
    pub buildplate_marker_detector: bool,
    pub printing_monitor: bool,
    pub allow_skip_parts: bool,
    pub halt_print_sensitivity: String,
}


#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpgradeStateBlock {
    pub ota_new_version_number: String,
    pub ams_new_version_number: String,
    pub status: String,
    pub progress: String,
    pub force_upgrade: bool,
    pub new_version_state: u32,
    pub err_code: u32,
    pub module: String,
    pub message: String,
    pub sn: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct NozzleBlock {
    pub info: Vec<NozzleInfoBlock>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct NozzleInfoBlock {
    #[serde(rename = "type")]
    pub nozzle_type: String,
    pub diameter: f64,
    pub wear: u32,
    pub id: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct PlateBlock {
    pub base: u32,
    pub mat: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct NetBlock {
    pub info: Vec<NetInfoBlock>,
    pub conf: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct NetInfoBlock {
    pub ip: i64,
    pub mask: i64,
}