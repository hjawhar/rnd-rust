use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pm2Monit {
    pub memory: f32,
    pub cpu: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pm2Env {
    pub status: String,
    pub pm_uptime: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pm2 {
    pub pid: u32,
    pub name: String,
    pub pm_id: u32,
    pub monit: Pm2Monit,
    pub pm2_env: Pm2Env,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Docker {
    pub Command: String,
    pub CreatedAt: String,
    pub ID: String,
    pub Image: String,
    pub Labels: String,
    pub LocalVolumes: String,
    pub Mounts: String,
    pub Names: String,
    pub Networks: String,
    pub Ports: String,
    pub RunningFor: String,
    pub Size: String,
    pub State: String,
    pub Status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Systemctl {
    unit: String,
    load: String,
    active: String,
    sub: Option<String>,
}
