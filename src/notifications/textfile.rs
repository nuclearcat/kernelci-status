use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

pub async fn append(path: &str, message: &str) -> Result<(), String> {
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let line = format!("[{timestamp}] {message}\n");

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("Failed to open text file {path}: {e}"))?;

    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to write to text file: {e}"))?;
    Ok(())
}
