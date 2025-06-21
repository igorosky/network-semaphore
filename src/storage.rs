use std::{env::home_dir, io::Error};
use std::path::PathBuf;
use tokio::fs::{create_dir_all, File};

async fn config_dir() -> PathBuf {
  let dir = home_dir()
    .expect("Failed to get home directory")
    .join(".network_semaphore");
  if dir.exists() && !dir.is_dir() {
    panic!("Configuration path exists but is not a directory: {:?}", dir);
  }
  if !dir.is_dir() {
    create_dir_all(&dir).await
      .expect("Failed to create configuration directory");
  }
  dir
}

pub async fn clear() -> std::io::Result<()> {
  tokio::fs::remove_dir_all(config_dir().await).await
}

pub async fn get_data<T: serde::de::DeserializeOwned>(path: &str) -> Option<T> {
  let path = config_dir().await.join(path);
  if !path.is_file() {
    return None;
  }
  serde_json::from_reader(
    File::open(path).await.ok()?
      .into_std().await
  ).ok()
}

pub async fn save_data<T: serde::Serialize>(path: &str, data: &T) -> std::io::Result<()> {
  let path = config_dir().await.join(path);
  serde_json::to_writer(File::create(path).await?
    .into_std().await,
    data)
  .map_err(|e| Error::other(format!("Failed to write data: {}", e)))
}
