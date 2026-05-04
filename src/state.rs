use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

use crate::notifications::NotificationEvent;

#[derive(Clone)]
pub struct AppState {
    pub db: tokio_rusqlite::Connection,
    pub http_client: reqwest::Client,
    pub config_cache: Arc<RwLock<HashMap<String, String>>>,
    pub notify_tx: mpsc::Sender<NotificationEvent>,
    pub secure_cookies: bool,
}
