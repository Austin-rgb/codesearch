use crate::types::{Config, UserId};

pub fn load_config() -> Config {
    Config {
        name: "production".to_string(),
    }
}

pub fn default_config() -> Config {
    Config::default()
}

pub fn log_out(id: UserId) {
    println!("user {} logged out", id.0);
}
