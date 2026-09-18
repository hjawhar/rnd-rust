pub mod config;
pub mod error;

/// Generated protobuf types for the bambu.v1 package.
pub mod proto {
    pub mod bambu {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/bambu.v1.rs"));
        }
    }
}

/// Re-export at a convenient depth.
pub use proto::bambu::v1 as telemetry;
pub use error::BambuError;
