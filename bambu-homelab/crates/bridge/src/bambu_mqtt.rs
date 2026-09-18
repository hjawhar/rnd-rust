//! MQTT connection to a Bambu Lab printer on the LAN.

use std::sync::Arc;

use anyhow::Context;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::DigitallySignedStruct;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::bambu_types::BambuReport;

/// A TLS certificate verifier that accepts any certificate.
/// Required because the X1C uses a self-signed cert from BBL CA.
#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub async fn connect(
    ip: &str,
    port: u16,
    serial: &str,
    access_code: &str,
) -> anyhow::Result<(AsyncClient, mpsc::Receiver<BambuReport>)> {
    let client_id = format!("bambu-bridge-{}", &serial[serial.len().saturating_sub(6)..]);

    let mut mqtt_opts = MqttOptions::new(&client_id, ip, port);
    mqtt_opts.set_credentials("bblp", access_code);
    mqtt_opts.set_keep_alive(std::time::Duration::from_secs(30));
    // The X1C sends ~15KB status reports; rumqttc defaults to 10KB max.
    mqtt_opts.set_max_packet_size(256 * 1024, 256 * 1024);

    // Build rustls config that skips cert verification.
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();

    mqtt_opts.set_transport(Transport::tls_with_config(
        rumqttc::TlsConfiguration::Rustls(Arc::new(tls_config)),
    ));

    let (client, mut eventloop) = AsyncClient::new(mqtt_opts, 128);

    let topic = format!("device/{serial}/report");
    client
        .subscribe(&topic, QoS::AtMostOnce)
        .await
        .context("failed to subscribe to printer report topic")?;

    info!(topic, "subscribed to printer MQTT");

    let (tx, rx) = mpsc::channel::<BambuReport>(64);

    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    match serde_json::from_slice::<BambuReport>(&publish.payload) {
                        Ok(report) => {
                            if report.print.is_some() {
                                if tx.send(report).await.is_err() {
                                    info!("report channel closed, stopping MQTT reader");
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "ignoring non-report MQTT message");
                        }
                    }
                }
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    info!(code = ?ack.code, "MQTT connected to printer");
                }
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e, "MQTT connection error, will retry");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    Ok((client, rx))
}
