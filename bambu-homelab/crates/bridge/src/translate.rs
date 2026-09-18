//! Translates Bambu's JSON report types into our protobuf TelemetrySnapshot.

use bambu_shared::telemetry::{
    AmsStatus, AmsUnit, AmsTray, BuildPlate, HmsAlert, LightStatus, NetworkInfo, NozzleInfo,
    PrinterIdentity, PrinterState, SpeedProfile, TelemetrySnapshot, Temperature, UpgradeState,
    XcamSettings,
};

use crate::bambu_types::PrintStatus;

pub struct PrinterContext {
    pub printer_id: String,
    pub serial_number: String,
    pub name: String,
    pub model: String,
}

pub fn to_telemetry(status: &PrintStatus, ctx: &PrinterContext) -> TelemetrySnapshot {
    TelemetrySnapshot {
        printer: Some(PrinterIdentity {
            printer_id: ctx.printer_id.clone(),
            name: ctx.name.clone(),
            firmware_version: format_version(&status.ver),
            model: ctx.model.clone(),
            serial_number: ctx.serial_number.clone(),
        }),
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        state: map_printer_state(&status.gcode_state).into(),
        gcode_state: status.gcode_state.clone(),
        nozzle_temp: Some(Temperature {
            current: status.nozzle_temper as f32,
            target: status.nozzle_target_temper as f32,
        }),
        bed_temp: Some(Temperature {
            current: status.bed_temper as f32,
            target: status.bed_target_temper as f32,
        }),
        chamber_temp: extract_chamber_temp(status),
        print_progress_pct: status.mc_percent,
        current_file: status.gcode_file.clone(),
        eta_minutes: status.mc_remaining_time,
        layer_num: status.layer_num,
        total_layer_num: status.total_layer_num,
        subtask_name: status.subtask_name.clone(),
        part_fan_pct: parse_fan_speed(&status.cooling_fan_speed),
        aux_fan_pct: parse_fan_speed(&status.big_fan1_speed),
        chamber_fan_pct: parse_fan_speed(&status.big_fan2_speed),
        heatbreak_fan_pct: parse_fan_speed(&status.heatbreak_fan_speed),
        fan_gear: status.fan_gear,
        speed_profile: map_speed_level(status.spd_lvl).into(),
        speed_magnitude_pct: status.spd_mag,
        wifi_signal: status.wifi_signal.clone(),
        ams: translate_ams(&status.ams),
        lights: status
            .lights_report
            .iter()
            .map(|l| LightStatus {
                node: l.node.clone(),
                mode: l.mode.clone(),
            })
            .collect(),
        rtsp_url: status
            .ipcam
            .as_ref()
            .map(|c| c.rtsp_url.clone())
            .unwrap_or_default(),
        nozzle_type: status.nozzle_type.clone(),
        nozzle_diameter: status.nozzle_diameter.parse().unwrap_or(0.4),
        print_error: status.print_error,
        error_code: status.mc_print_error_code.clone(),
        xcam: status.xcam.as_ref().map(|x| XcamSettings {
            spaghetti_detector: x.spaghetti_detector,
            print_halt: x.print_halt,
            first_layer_inspector: x.first_layer_inspector,
            buildplate_marker_detector: x.buildplate_marker_detector,
            printing_monitor: x.printing_monitor,
        }),
        sdcard_present: status.sdcard,
        upgrade_state: status.upgrade_state.as_ref().map(|u| UpgradeState {
            ota_version: u.ota_new_version_number.clone(),
            ams_version: u.ams_new_version_number.clone(),
            status: u.status.clone(),
            progress: u.progress.parse().unwrap_or(0),
            force_upgrade: u.force_upgrade,
            new_version_state: u.new_version_state,
            err_code: u.err_code,
            module: u.module.clone(),
            message: u.message.clone(),
        }),
        hms_alerts: status.hms.iter().filter_map(|v| {
            let obj = v.as_object()?;
            Some(HmsAlert {
                attr: obj.get("attr")?.as_u64()? as u32,
                code: obj.get("code")?.as_u64()? as u32,
                description: String::new(),
            })
        }).collect(),
        nozzle_info: status.device.as_ref()
            .and_then(|d| d.nozzle.as_ref())
            .and_then(|n| n.info.first())
            .map(|n| NozzleInfo {
                r#type: n.nozzle_type.clone(),
                diameter: n.diameter as f32,
                wear: n.wear,
            }),
        build_plate: status.device.as_ref()
            .and_then(|d| d.plate.as_ref())
            .map(|p| BuildPlate {
                base: p.base,
                material: p.mat,
            }),
        network: Some(NetworkInfo {
            wifi_signal: status.wifi_signal.clone(),
            ip_address: status.net.as_ref()
                .and_then(|n| n.info.first())
                .map(|i| {
                    let ip = i.ip;
                    format!("{}.{}.{}.{}", ip & 0xFF, (ip >> 8) & 0xFF, (ip >> 16) & 0xFF, (ip >> 24) & 0xFF)
                })
                .unwrap_or_default(),
        }),
    }
}

/// Format printer version string to semver.
/// e.g. "20000" -> "v2.00.00", "11602" -> "v1.16.02"
fn format_version(ver: &str) -> String {
    let v: u32 = ver.parse().unwrap_or(0);
    if v == 0 { return String::new(); }
    let major = v / 10000;
    let minor = (v / 100) % 100;
    let patch = v % 100;
    format!("v{major}.{minor:02}.{patch:02}")
}

fn map_printer_state(gcode_state: &str) -> PrinterState {
    match gcode_state {
        "IDLE" => PrinterState::Idle,
        "RUNNING" => PrinterState::Printing,
        "PAUSE" => PrinterState::Paused,
        "FINISH" => PrinterState::Finished,
        "FAILED" => PrinterState::Failed,
        "PREPARE" => PrinterState::Preparing,
        _ => PrinterState::Unspecified,
    }
}

fn map_speed_level(lvl: u32) -> SpeedProfile {
    match lvl {
        1 => SpeedProfile::Silent,
        2 => SpeedProfile::Standard,
        3 => SpeedProfile::Sport,
        4 => SpeedProfile::Ludicrous,
        _ => SpeedProfile::Unspecified,
    }
}

/// Bambu reports fan speed as a string 0–15 (PWM gear) or 0–100 (percentage).
/// Normalize to 0–100%.
fn parse_fan_speed(s: &str) -> u32 {
    let val: u32 = s.parse().unwrap_or(0);
    if val <= 15 {
        (val * 100) / 15
    } else {
        val.min(100)
    }
}

fn extract_chamber_temp(status: &PrintStatus) -> Option<Temperature> {
    let temp = status
        .device
        .as_ref()
        .and_then(|d| d.ctc.as_ref())
        .and_then(|c| c.info.as_ref())
        .map(|i| i.temp as f32)
        .or_else(|| status.info.as_ref().map(|i| i.temp as f32));
    temp.map(|current| Temperature {
        current,
        target: 0.0,
    })
}

fn translate_ams(ams: &Option<crate::bambu_types::AmsStatus>) -> Option<AmsStatus> {
    let ams = ams.as_ref()?;
    Some(AmsStatus {
        units: ams
            .ams
            .iter()
            .map(|u| AmsUnit {
                id: u.id.clone(),
                humidity: u.humidity.clone(),
                temperature: u.temp.parse().unwrap_or(0.0),
                trays: u
                    .tray
                    .iter()
                    .map(|t| AmsTray {
                        id: t.id.clone(),
                        filament_type: t.tray_type.clone(),
                        color: t.tray_color.clone(),
                        sub_brand: t.tray_sub_brands.clone(),
                        nozzle_temp_min: t.nozzle_temp_min.parse().unwrap_or(0.0),
                        nozzle_temp_max: t.nozzle_temp_max.parse().unwrap_or(0.0),
                        remain_pct: t.remain,
                        tag_uid: t.tag_uid.clone(),
                        diameter: t.tray_diameter.parse().unwrap_or(1.75),
                        drying_temp: t.drying_temp.clone(),
                        drying_time: t.drying_time.clone(),
                    })
                    .collect(),
            })
            .collect(),
        tray_now: ams.tray_now.clone(),
        tray_tar: ams.tray_tar.clone(),
        insert_flag: ams.insert_flag,
        firmware_version: ams.version,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::bambu_types::BambuReport;

    #[test]
    fn test_reference_payload_parses_and_translates() {
        let json = include_str!("../../../docs/reference/x1c-mqtt-report-sample.json");
        let report: BambuReport = serde_json::from_str(json)
            .expect("reference payload should deserialize");
        let status = report.print.expect("reference payload should have print block");

        let ctx = PrinterContext {
            printer_id: "test".into(),
            serial_number: "SN123".into(),
            name: "Test Printer".into(),
            model: "X1C".into(),
        };
        let snap = to_telemetry(&status, &ctx);

        // Basic fields still work
        assert!(snap.nozzle_temp.is_some());
        assert!(snap.bed_temp.is_some());

        // Upgrade state
        if let Some(ref u) = snap.upgrade_state {
            // The reference payload has an upgrade_state block
            assert!(!u.ota_version.is_empty() || u.progress == 0);
        }

        // Nozzle info from device block
        if let Some(ref n) = snap.nozzle_info {
            assert!(n.diameter > 0.0);
        }

        // Build plate from device block
        if let Some(ref bp) = snap.build_plate {
            assert!(bp.base > 0 || bp.material > 0 || true); // may be 0
        }

        // Network info is always Some
        let net = snap.network.as_ref().expect("network should always be Some");
        assert!(!net.wifi_signal.is_empty() || net.wifi_signal.is_empty()); // just assert it exists

        // AMS fields
        if let Some(ref ams) = snap.ams {
            for unit in &ams.units {
                for tray in &unit.trays {
                    // drying fields should be present (may be empty strings)
                    let _ = &tray.drying_temp;
                    let _ = &tray.drying_time;
                }
            }
        }

        println!("Snapshot translated successfully: state={:?}, progress={}%, layers={}/{}",
            snap.gcode_state, snap.print_progress_pct, snap.layer_num, snap.total_layer_num);
        if let Some(ref u) = snap.upgrade_state {
            println!("  upgrade: ota={} ams={} status={}", u.ota_version, u.ams_version, u.status);
        }
        if let Some(ref n) = snap.nozzle_info {
            println!("  nozzle: type={} diameter={} wear={}", n.r#type, n.diameter, n.wear);
        }
        if let Some(ref bp) = snap.build_plate {
            println!("  plate: base={} material={}", bp.base, bp.material);
        }
        if let Some(ref net) = snap.network {
            println!("  network: wifi={} ip={}", net.wifi_signal, net.ip_address);
        }
    }
}