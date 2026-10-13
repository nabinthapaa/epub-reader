//! Persistence: settings, recents and cached covers under the XDG directories.
//!
//! Writes go to a temporary file and are renamed into place, so a crash or a
//! full disk cannot leave a half-written settings file behind.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::Result;
use crate::state::{Position, RecentBook, Settings};

const MAX_RECENTS: usize = 24;

/// Resolved on-disk locations.
#[derive(Clone, Debug)]
pub struct Store {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl Store {
    /// Locate the app's directories, creating them if needed.
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "folio");
        let (config_dir, data_dir) = match dirs {
            Some(dirs) => (
                dirs.config_dir().to_path_buf(),
                dirs.data_dir().to_path_buf(),
            ),
            None => (PathBuf::from(".folio"), PathBuf::from(".folio")),
        };
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(data_dir.join("covers"))?;
        Ok(Self {
            config_dir,
            data_dir,
        })
    }

    pub fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn recents_path(&self) -> PathBuf {
        self.data_dir.join("recents.json")
    }

    pub fn covers_dir(&self) -> PathBuf {
        self.data_dir.join("covers")
    }

    /// Load settings, falling back to defaults when the file is absent or
    /// unreadable: a corrupt settings file must never stop the app opening.
    pub fn load_settings(&self) -> Settings {
        read_json::<Settings>(&self.settings_path())
            .unwrap_or_default()
            .sanitize()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_atomic(
            &self.settings_path(),
            serde_json::to_vec_pretty(settings)?.as_slice(),
        )
    }

    pub fn load_recents(&self) -> Vec<RecentBook> {
        read_json(&self.recents_path()).unwrap_or_default()
    }

    /// Whether the recents file exists but could not be read.
    ///
    /// An unreadable shelf must not be overwritten: doing so would throw away
    /// every saved reading position the moment a format change slipped in.
    fn recents_are_damaged(&self) -> bool {
        let path = self.recents_path();
        path.exists() && read_json::<Vec<RecentBook>>(&path).is_none()
    }

    pub fn save_recents(&self, recents: &[RecentBook]) -> Result<()> {
        write_atomic(
            &self.recents_path(),
            serde_json::to_vec_pretty(recents)?.as_slice(),
        )
    }

    /// Record (or update) a book on the shelf, newest first.
    pub fn touch_recent(&self, entry: RecentBook) -> Result<Vec<RecentBook>> {
        if self.recents_are_damaged() {
            tracing::warn!("recents file is unreadable; leaving it alone");
            return Ok(vec![entry]);
        }
        let mut recents = self.load_recents();
        recents.retain(|r| r.path != entry.path);
        recents.insert(0, entry);
        recents.truncate(MAX_RECENTS);
        self.save_recents(&recents)?;
        Ok(recents)
    }

    /// Update just the stored position and progress for a book.
    pub fn update_position(
        &self,
        book_path: &str,
        position: Position,
        progress: f32,
    ) -> Result<()> {
        if self.recents_are_damaged() {
            return Ok(());
        }
        let mut recents = self.load_recents();
        if let Some(entry) = recents.iter_mut().find(|r| r.path == book_path) {
            entry.position = position;
            entry.progress = progress;
            entry.opened_at = now_secs();
            self.save_recents(&recents)?;
        }
        Ok(())
    }

    pub fn position_for(&self, book_path: &str) -> Option<Position> {
        self.load_recents()
            .into_iter()
            .find(|r| r.path == book_path)
            .map(|r| r.position)
    }

    /// Store a cover image and return the file name to reference it by.
    pub fn save_cover(&self, book_path: &str, png: &[u8]) -> Result<String> {
        let name = format!("{}.png", hash_name(book_path));
        write_atomic(&self.covers_dir().join(&name), png)?;
        Ok(name)
    }
}

/// Seconds since the Unix epoch, saturating at 0 before it.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Short, stable file name for a book path.
fn hash_name(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(input.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "ignoring unreadable state file");
            None
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temp, path)?;
    Ok(())
}
