use std::path::PathBuf;

pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    pub web_dist: PathBuf,
}

pub fn from_env() -> Config {
    Config {
        bind: std::env::var("OPENCODE2API_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into()),
        data_dir: std::env::var("OPENCODE2API_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data")),
        web_dist: std::env::var("OPENCODE2API_WEB_DIST")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("frontend/dist")),
    }
}
