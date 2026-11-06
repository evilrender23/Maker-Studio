//! Consola del motor: un registro global que leen el editor y el reproductor.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub level: Level,
    pub source: String,
    pub text: String,
    /// Segundos desde la época Unix.
    pub at: f64,
}

static LOG: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
const MAX: usize = 5000;

pub fn push(level: Level, source: &str, text: impl Into<String>) {
    let text = text.into();
    match level {
        Level::Info => log::info!("[{source}] {text}"),
        Level::Warn => log::warn!("[{source}] {text}"),
        Level::Error => log::error!("[{source}] {text}"),
    }
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if l.len() >= MAX {
        l.drain(..MAX / 5);
    }
    l.push(Entry { level, source: source.into(), text, at });
}

pub fn info(source: &str, text: impl Into<String>) {
    push(Level::Info, source, text)
}
pub fn warn(source: &str, text: impl Into<String>) {
    push(Level::Warn, source, text)
}
pub fn error(source: &str, text: impl Into<String>) {
    push(Level::Error, source, text)
}

/// Copia de las entradas a partir de `from` (para leer solo lo nuevo).
pub fn since(from: usize) -> Vec<Entry> {
    let l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    l.get(from.min(l.len())..).map(|s| s.to_vec()).unwrap_or_default()
}

pub fn len() -> usize {
    LOG.lock().map(|l| l.len()).unwrap_or(0)
}

pub fn clear() {
    if let Ok(mut l) = LOG.lock() {
        l.clear();
    }
}
