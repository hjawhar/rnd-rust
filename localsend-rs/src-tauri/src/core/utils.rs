use std::{error::Error, fs::File, io::Write, net::{IpAddr, Ipv4Addr}, process::Command};

use futures_util::future;
use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use rcgen::{Certificate, CertificateParams, DnType, DnValue, KeyPair};

pub const BUFFER_SIZE: u16 = 2048;
pub const NUM_REPEAT: u8 = 2;
pub const DEVICE_MODEL: &str = "linux";
pub const DEVICE_TYPE: &str = "desktop";

pub const ALIAS: &str = "rustsend";
pub const INTERFACE_ADDR: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);
pub const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
pub const MULTICAST_PORT: u16 = 53317;

pub fn get_current_device_ips() -> Vec<IpAddr> {
    let mut my_ips: Vec<IpAddr> = vec![];
    for network_interface in NetworkInterface::show().unwrap_or(vec![]).iter() {
        for address in network_interface.addr.iter() {
            if address.ip().is_ipv4() && !address.ip().is_loopback() {
                my_ips.push(address.ip());
            }
        }
    }
    my_ips
}

pub async fn scan_network() -> Vec<String> {
    let mut tasks = vec![];

    for i in 100..=150 {
        let ip = format!("192.168.1.{}", i);
        let clone_ip = ip.clone();

        tasks.push(tokio::spawn(async move {
            println!("scan_network # begin: {}", &ip);
            let output = Command::new("ping")
                .args(["-c", "1", "-W", "1", &ip])
                .output()
                .expect("Failed to execute command");
            let stdout = String::from_utf8_lossy(&output.stdout);
            // println!("{:#?}", stdout);
            println!("scan_network # end: {}", &ip);
            if stdout.contains("1 received") || stdout.contains("1 packets received") {
                // if is_server_available(&clone_ip.as_str()).await {
                println!("scan_network # add {}", clone_ip);
                return Some(clone_ip);
                // }
            }
            None
        }));
    }

    let results = future::join_all(tasks).await;
    let mut available_ips = Vec::new();
    for res in results {
        if let Some(ip) = res.unwrap() {
            available_ips.push(ip)
        }
    }

    available_ips
}

pub fn get_device_ip_addr() -> Option<IpAddr> {
    for network_interface in NetworkInterface::show().unwrap_or(vec![]).iter() {
        match network_interface.addr.first() {
            Some(addr) => {
                if addr.ip().is_loopback() {
                    continue;
                } else {
                    return Some(addr.ip());
                }
            }
            None => continue,
        };
    }
    None
}

pub fn generate_cert_and_write() -> Result<String, Box<dyn Error>> {
    let key_pair: KeyPair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut params: CertificateParams = Default::default();
    params.distinguished_name.push(
        DnType::CommonName,
        DnValue::PrintableString("Localsend client".try_into().unwrap()),
    );
    params
        .distinguished_name
        .push(DnType::OrganizationName, "".to_string());
    params
        .distinguished_name
        .push(DnType::OrganizationalUnitName, "".to_string());
    params
        .distinguished_name
        .push(DnType::LocalityName, "".to_string());
    params
        .distinguished_name
        .push(DnType::StateOrProvinceName, "".to_string());
    params
        .distinguished_name
        .push(DnType::CountryName, "".to_string());

    // Generate the CSR
    let csr = params.serialize_request(&key_pair)?;

    // Generate CSR (PKCS#10) in PEM format
    let csr_pem = csr.pem()?;

    // Save CSR to file
    let mut csr_file = File::create("certificate.csr")?;
    csr_file.write_all(csr_pem.as_bytes())?;

    println!("{}", csr_pem);

    // Get the PrivateKey in PEM format
    let private_key_pem = key_pair.serialize_pem();

    // Save PrivateKey  to file
    let mut key_file = File::create("private_key.pem")?;
    key_file.write_all(private_key_pem.as_bytes())?;

    println!("{}", private_key_pem);

    println!("CSR saved to certificate.csr");
    println!("Private key saved to private_key.pem");
    Ok(private_key_pem)
}

pub fn generate_cert() -> (Certificate, KeyPair) {
    let mut params: CertificateParams = Default::default();
    params.distinguished_name.push(
        DnType::CommonName,
        DnValue::PrintableString("Localsend client".try_into().unwrap()),
    );
    params
        .distinguished_name
        .push(DnType::OrganizationName, "".to_string());
    params
        .distinguished_name
        .push(DnType::OrganizationalUnitName, "".to_string());
    params
        .distinguished_name
        .push(DnType::LocalityName, "".to_string());
    params
        .distinguished_name
        .push(DnType::StateOrProvinceName, "".to_string());
    params
        .distinguished_name
        .push(DnType::CountryName, "".to_string());

    let key_pair = KeyPair::generate().unwrap();
    (params.self_signed(&key_pair).unwrap(), key_pair)
}
