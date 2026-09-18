//! Translates platform commands to Bambu MQTT command format.

use futures_util::StreamExt;
use rumqttc::{AsyncClient, QoS};
use serde::Deserialize;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
pub struct CommandRequest {
    pub command: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Spawn a task that listens for commands on NATS and forwards them to the printer via MQTT.
pub async fn spawn_command_handler(
    nats: async_nats::Client,
    mqtt: AsyncClient,
    printer_id: &str,
    serial: &str,
) -> anyhow::Result<()> {
    let subject = format!("printers.{printer_id}.cmd");
    let mqtt_topic = format!("device/{serial}/request");
    let mut sub = nats.subscribe(subject.clone()).await?;

    info!(subject, "listening for commands");

    tokio::spawn(async move {
        let mut seq_id: u64 = 1000;

        while let Some(msg) = sub.next().await {
            let cmd: CommandRequest = match serde_json::from_slice(&msg.payload) {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "invalid command payload");
                    continue;
                }
            };

            seq_id += 1;
            let seq = seq_id.to_string();

            let mqtt_payload = match cmd.command.as_str() {
                "pause" => serde_json::json!({
                    "print": {
                        "sequence_id": seq,
                        "command": "pause",
                        "param": ""
                    }
                }),
                "resume" => serde_json::json!({
                    "print": {
                        "sequence_id": seq,
                        "command": "resume",
                        "param": ""
                    }
                }),
                "stop" => serde_json::json!({
                    "print": {
                        "sequence_id": seq,
                        "command": "stop",
                        "param": ""
                    }
                }),
                "set_speed" => {
                    let level = cmd
                        .params
                        .get("level")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(2);
                    serde_json::json!({
                        "print": {
                            "sequence_id": seq,
                            "command": "print_speed",
                            "param": level.to_string()
                        }
                    })
                }
                "set_light" => {
                    let node = cmd
                        .params
                        .get("node")
                        .and_then(|v| v.as_str())
                        .unwrap_or("chamber_light");
                    let mode = cmd
                        .params
                        .get("mode")
                        .and_then(|v| v.as_str())
                        .unwrap_or("on");
                    serde_json::json!({
                        "system": {
                            "sequence_id": seq,
                            "command": "ledctrl",
                            "led_node": node,
                            "led_mode": mode,
                            "led_on_time": 500,
                            "led_off_time": 500,
                            "loop_times": 0,
                            "interval_time": 0
                        }
                    })
                }
                "home" => serde_json::json!({
                    "print": {
                        "sequence_id": seq,
                        "command": "gcode_line",
                        "param": "G28\n"
                    }
                }),
                "upgrade" => {
                    let module = cmd.params.get("module")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ota");
                    let url = cmd.params.get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let version = cmd.params.get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    serde_json::json!({
                        "upgrade": {
                            "sequence_id": seq,
                            "command": "start",
                            "src_id": 1,
                            "url": url,
                            "module": module,
                            "version": version
                        }
                    })
                }
                "gcode" => {
                    let code = cmd.params.get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    serde_json::json!({
                        "print": {
                            "sequence_id": seq,
                            "command": "gcode_line",
                            "param": format!("{code}\n")
                        }
                    })
                }
                "print_start" => {
                    let filename = cmd.params.get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let plate_number = cmd.params.get("plate")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1);
                    serde_json::json!({
                        "print": {
                            "sequence_id": seq,
                            "command": "project_file",
                            "param": format!("Metadata/plate_{}.gcode", plate_number),
                            "subtask_name": filename,
                            "url": format!("ftp://{}", filename),
                            "bed_type": "auto",
                            "timelapse": false,
                            "bed_leveling": true,
                            "flow_cali": true,
                            "vibration_cali": true,
                            "layer_inspect": false,
                            "use_ams": true
                        }
                    })
                }
                "print_stop" => serde_json::json!({
                    "print": {
                        "sequence_id": seq,
                        "command": "stop",
                        "param": ""
                    }
                }),
                other => {
                    warn!(command = other, "unknown command, ignoring");
                    continue;
                }
            };

            let payload_str = serde_json::to_string(&mqtt_payload).unwrap_or_default();
            info!(command = %cmd.command, "forwarding command to printer");

            if let Err(e) = mqtt
                .publish(
                    &mqtt_topic,
                    QoS::AtLeastOnce,
                    false,
                    payload_str.as_bytes().to_vec(),
                )
                .await
            {
                error!(error = %e, "failed to publish command to printer MQTT");
            }
        }
    });

    Ok(())
}
