use tabled::{Table, Tabled};

use crate::models::{Docker, Pm2};

#[derive(Tabled)]
struct ResponseTable {
    pub platform: String,
    pub uptime: String,
    pub name: String,
    pub status: String,
    pub pid: String,
    pub id: String,
}

pub fn display_pm2(input: &Vec<Pm2>) {
    let mapped: Vec<ResponseTable> = input
        .iter()
        .map(|x| ResponseTable {
            platform: "pm2".to_string(),
            uptime: x.pm2_env.pm_uptime.to_string(),
            status: x.pm2_env.status.clone(),
            pid: x.pid.to_string(),
            name: x.name.clone(),
            id: x.pm_id.to_string(),
        })
        .clone()
        .collect();
    println!("{}", Table::new(mapped).to_string());
}

pub fn display_docker(input: &Vec<Docker>) {
    let mapped: Vec<ResponseTable> = input
        .iter()
        .map(|x| ResponseTable {
            platform: "docker".to_string(),
            uptime: x.RunningFor.clone(),
            status: x.Status.clone(),
            pid: "".to_string(),
            name: x.Names.clone(),
            id: x.ID.to_string(),
        })
        .clone()
        .collect();
    println!("{}", Table::new(mapped).to_string());
}
