use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use crate::models::{DeviceInfo, DeviceResponse, LocalSendDevice};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use super::utils::{get_device_ip_addr, BUFFER_SIZE, DEVICE_MODEL, DEVICE_TYPE, NUM_REPEAT};

impl LocalSendDevice {
    pub fn new(
        device_alias: String,
        interface_addr: Ipv4Addr,
        multicast_addr: Ipv4Addr,
        multicast_port: u16,
    ) -> Self {
        let fingerprint = Uuid::new_v4();
        let ip_addr = get_device_ip_addr().unwrap_or(IpAddr::V4([0, 0, 0, 0].into()));

        let device_info = DeviceInfo {
            alias: device_alias,
            device_type: DEVICE_TYPE.to_string(),
            device_model: Some(DEVICE_MODEL.to_string()),
            ip: ip_addr.to_string(),
            port: multicast_port,
            ip_ending: Some(ip_addr.to_string().split(".").last().unwrap().to_string()),
        };
        let this_device = DeviceResponse {
            device_info,
            announcement: true,
            fingerprint: fingerprint.to_string(),
        };

        Self {
            socket: None,
            this_device,
            devices: vec![],
            interface_addr,
            multicast_addr,
            multicast_port,
        }
    }

    pub async fn connect(&mut self) {
        let socket = Arc::new(
            UdpSocket::bind((self.interface_addr, self.multicast_port))
                .await
                .expect("couldn't bind to address"),
        );
        self.socket = Some(socket);
    }

    pub async fn announce(
        send_socket: &Arc<UdpSocket>,
        announcement_msg: &str,
        addr: (Ipv4Addr, u16),
    ) {
        // TODO(notjedi): any other way to not accept addr as argument
        send_socket
            .send_to(announcement_msg.as_bytes(), addr)
            .await
            .unwrap();
    }

    pub async fn announce_repeat(
        send_socket: Arc<UdpSocket>,
        announcement_msg: String,
        addr: (Ipv4Addr, u16),
    ) {
        // TODO(notjedi): any other way to not accept addr as argument
        loop {
            for _ in 0..NUM_REPEAT {
                Self::announce(&send_socket, announcement_msg.as_str(), addr).await;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    pub async fn listen_and_announce_multicast(
        &mut self,
        socket: Arc<UdpSocket>,
        sender: Sender<Vec<DeviceInfo>>,
    ) {
        // https://gist.github.com/pusateri/df98511b88e9000f388d344a1f3db9e7
        socket
            .join_multicast_v4(self.multicast_addr, self.interface_addr)
            .expect("failed to join multicast");

        self.this_device.announcement = true;
        let send_socket = socket.clone();
        let announce_msg = serde_json::to_string(&self.this_device).unwrap();
        tokio::spawn(Self::announce_repeat(
            send_socket,
            announce_msg,
            (self.multicast_addr, self.multicast_port),
        ));

        self.this_device.announcement = false;
        let reply_announce_msg = serde_json::to_string(&self.this_device).unwrap();

        let mut buf = [0u8; BUFFER_SIZE as usize];
        loop {
            if let Ok((amt, src)) = socket.recv_from(&mut buf).await {
                let mut device_response: DeviceResponse =
                    serde_json::from_slice(&buf[..amt]).unwrap();
                (
                    device_response.device_info.ip,
                    device_response.device_info.port,
                ) = (src.ip().to_string(), src.port());

                if device_response == self.this_device {
                    continue;
                }

                if device_response.announcement {
                    Self::announce(
                        &socket,
                        reply_announce_msg.as_str(),
                        (self.multicast_addr, self.multicast_port),
                    )
                    .await;
                }

                if !self.devices.contains(&device_response.device_info) {
                    device_response.device_info.ip_ending = Some(src.ip().to_string().split(".").last().unwrap().to_string());
                    self.devices.push(device_response.device_info);
                    println!("{:#?}", &self.devices);
                    println!("{:#?}", &self.devices.len());
                    let _ = sender.send(self.devices.clone()).await;
                }
            }
        }
    }
}
