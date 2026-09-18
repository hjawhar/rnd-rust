use std::env;

pub struct Config {
    pub app_env: String,
    pub log_level: String,
    pub port: u16,
    pub db_pool_size: usize,
    pub max_concurrent_requests: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".into());
        let is_prod = app_env == "production";

        let required = [
            "DATABASE_URL",
            "REDIS_URL",
            "AES256_GCM_KEY",
            "ED25519_KEY",
        ];
        let missing: Vec<&str> = required
            .iter()
            .filter(|k| env::var(k).is_err())
            .copied()
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "FATAL: missing required environment variables: {}",
                missing.join(", ")
            );
            std::process::exit(1);
        }

        Self {
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| {
                if is_prod { "info" } else { "debug" }.into()
            }),
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            db_pool_size: env::var("DB_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_prod { 50 } else { 5 }),
            app_env,
            max_concurrent_requests: env::var("MAX_CONCURRENT_HANDLERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_prod { 200 } else { 50 }),
        }
    }
}
