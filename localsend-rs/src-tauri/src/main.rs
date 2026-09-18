// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod core;
pub mod models;

use core::{
    server::Server,
    utils::{ALIAS, INTERFACE_ADDR, MULTICAST_ADDR, MULTICAST_PORT},
};
use std::{collections::HashMap, fmt::Write, sync::Arc};

use console::style;
use dialoguer::{theme::ColorfulTheme, MultiSelect};
use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use models::{
    AppState, ClientMessage, DeviceInfo, FileInfo, LocalSendDevice, Receiver, Sender, ServerMessage,
};
use tokio::sync::{mpsc, Mutex};
use tracing::info;

struct State {
    multi_progress: MultiProgress,
    files: HashMap<String, FileInfo>,
    progress_map: HashMap<String, ProgressBar>,
}

use tauri::Manager;

#[tauri::command]
async fn get_nearby_devices(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<DeviceInfo>, ()> {
    let state = state.lock().await;
    let devices = state.device.devices.clone();
    Ok(devices)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[tokio::main]
async fn main() {
    let device = LocalSendDevice::new(
        ALIAS.to_string(),
        INTERFACE_ADDR,
        MULTICAST_ADDR,
        MULTICAST_PORT,
    );
    let (tx_task, rx_task) = mpsc::channel::<Vec<DeviceInfo>>(1000);
    let (server_tx, server_rx) = mpsc::unbounded_channel();
    let (client_tx, client_rx) = mpsc::unbounded_channel();
    let app_state = Arc::new(Mutex::new(AppState {
        device,
        server_tx,
        client_rx,
        receive_session: None,
    }));

    let device_app_state = app_state.clone();
    let devices_app_state = app_state.clone();
    let tauri_app_state = app_state.clone();
    tokio::spawn(async move {
        let mut receiver = rx_task;
        while let Some(incoming_event) = receiver.recv().await {
            println!("{:#?}", incoming_event);
            let mut state = devices_app_state.lock().await;
            state.device.devices = incoming_event;
        }
    });

    tokio::spawn(async move {
        let app_state = device_app_state.lock().await;
        let mut device = app_state.device.clone();
        drop(app_state);
        device.connect().await;
        device
            .listen_and_announce_multicast(device.socket.clone().unwrap(), tx_task)
            .await;
    });

    tokio::spawn(handle_server_msgs(server_rx, client_tx));
    tokio::spawn(async {
        let server = Server::new(INTERFACE_ADDR, MULTICAST_PORT);
        server.start_server(app_state).await;
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![get_nearby_devices])
        .setup(|app| {
            app.manage(tauri_app_state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn handle_server_msgs(
    mut server_rx: Receiver<ServerMessage>,
    client_tx: Sender<ClientMessage>,
) {
    // TODO: set this back to None when we are done with a session
    let mut client_state: Option<State> = None;

    while let Some(server_message) = server_rx.recv().await {
        // println!("{:?}", &server_message);
        match server_message {
            ServerMessage::SendRequest(send_request) => {
                println!(
                    "{} wants to send you the following files:\n",
                    style(send_request.device_info.alias).bold().magenta()
                );

                let selections = MultiSelect::with_theme(&ColorfulTheme::default())
                    .with_prompt("Select the files you want to receive")
                    .items(
                        &send_request
                            .files
                            .values()
                            .map(|file_info| file_info.file_name.as_str())
                            .collect::<Vec<&str>>(),
                    )
                    .defaults(vec![true; send_request.files.len()].as_slice())
                    .interact()
                    .unwrap();

                if selections.is_empty() {
                    let _ = client_tx.send(ClientMessage::Decline);
                } else {
                    let file_ids = send_request
                        .files
                        .keys()
                        .map(|file_id| file_id.as_str())
                        .collect::<Vec<&str>>();

                    let selected_file_ids = selections
                        .into_iter()
                        .map(|idx| String::from(file_ids[idx]))
                        .collect::<Vec<_>>();
                    let _ = client_tx.send(ClientMessage::Allow(selected_file_ids));

                    let multi_progress = MultiProgress::new();
                    let progress_map = send_request
                            .files
                            .clone()
                            .into_iter()
                            .map(|(file_id, file_info)| {
                                // TODO(notjedi): change length ot size of file
                                let pb =
                                    multi_progress.add(ProgressBar::new(file_info.size as u64));

                                pb.set_style(ProgressStyle::with_template("{spinner:.green} [{msg}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                                    .unwrap()
                                    .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
                                    .progress_chars("#>-"));

                                pb.set_message(file_info.file_name);
                                (file_id, pb)
                            })
                            .collect::<HashMap<String, ProgressBar>>();

                    client_state = Some(State {
                        files: send_request.files,
                        multi_progress,
                        progress_map,
                    });
                }
            }
            ServerMessage::SendFileRequest((file_id, size)) => match client_state.as_ref() {
                Some(state) => {
                    state.progress_map[&file_id].inc(size as u64);
                    if state.progress_map[&file_id].position()
                        == (state.files[&file_id].size as u64)
                    {
                        state.progress_map[&file_id].finish_and_clear();
                        state
                            .multi_progress
                            .println(format!("Received {}", state.files[&file_id].file_name))
                            .unwrap();
                    }
                }
                None => {
                    info!("client_state is None. this shouldn't be happening as this block is unreachable.")
                }
            },
            ServerMessage::CancelSession => match client_state.as_ref() {
                // TODO(notjedi): handle cancel request when in send request phase
                Some(state) => {
                    for (file_id, pb) in &state.progress_map {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                            state
                                .multi_progress
                                .println(format!(
                                    "{} finished with error",
                                    state.files[file_id.as_str()].file_name
                                ))
                                .unwrap();
                        }
                    }
                    client_state = None;
                }
                None => {
                    info!("client_state is None. this shouldn't be happening as this block is unreachable.")
                }
            },
        }
    }
}
