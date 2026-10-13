//! The rendering worker.
//!
//! One thread owns the [`Reader`] (and with it the font system, layout caches
//! and the reading cursor). The UI thread only sends jobs and receives frames,
//! so a long chapter layout can never stall the window.
//!
//! Jobs are coalesced: when several arrive while one is being handled (held
//! arrow keys, a window resize in progress), every command is applied but only
//! the last one is painted.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use folio_core::book::TocItem;
use folio_core::layout::raster::LinkRegion;
use folio_core::reader::{Command, Reader, SearchHit, View};
use folio_core::state::{Position, RecentBook, Settings};
use folio_core::store::{Store, now_secs};
use folio_core::theme::{Theme, ThemePreset};

/// Minimum gap between position writes while turning pages.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);
/// Longest edge of a cached cover thumbnail.
const COVER_EDGE: u32 = 420;

pub enum Job {
    Open {
        generation: u64,
        path: PathBuf,
        settings: Settings,
        theme: Theme,
        viewport: (u32, u32),
    },
    Apply {
        generation: u64,
        command: Command,
    },
    Search {
        generation: u64,
        query: String,
    },
    /// Ask for the list of installed font families.
    Fonts,
    /// Flush the reading position to disk now.
    Flush,
    Shutdown,
}

/// A painted page plus everything the chrome shows alongside it.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub page: usize,
    pub page_count: usize,
    pub chapter_label: String,
    pub toc_index: Option<usize>,
    pub progress: f32,
    pub focus: bool,
    pub spotlight: bool,
    pub can_go_back: bool,
    pub font_px: f32,
    pub links: Vec<LinkRegion>,
}

impl Frame {
    fn from_view(view: View) -> Self {
        Self {
            width: view.canvas.width,
            height: view.canvas.height,
            rgba: view.canvas.rgba,
            page: view.page,
            page_count: view.page_count,
            chapter_label: view.chapter_label,
            toc_index: view.toc_index,
            progress: view.progress,
            focus: view.focus,
            spotlight: view.spotlight,
            can_go_back: view.can_go_back,
            font_px: view.font_px,
            links: view.links,
        }
    }
}

pub enum Event {
    Opened {
        generation: u64,
        title: String,
        toc: Vec<TocItem>,
        recents: Vec<RecentBook>,
    },
    Frame {
        generation: u64,
        frame: Box<Frame>,
    },
    SearchDone {
        generation: u64,
        hits: Vec<SearchHit>,
    },
    Fonts {
        families: Vec<String>,
        /// The family actually being rendered with right now.
        current: Option<String>,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

/// Handle used by the UI thread to talk to the worker.
pub struct Worker {
    sender: Sender<Job>,
}

impl Worker {
    pub fn send(&self, job: Job) {
        if let Err(error) = self.sender.send(job) {
            tracing::error!(%error, "render worker is gone");
        }
    }
}

/// Start the worker. `emit` is called from the worker thread for every event.
pub fn spawn(emit: impl Fn(Event) + Send + 'static) -> Worker {
    let (sender, receiver) = channel();
    std::thread::Builder::new()
        .name("folio-render".into())
        .spawn(move || run(receiver, emit))
        .expect("failed to start the render worker");
    Worker { sender }
}

struct State {
    reader: Option<Reader>,
    /// Installed families, resolved once and reused.
    families: Option<Vec<String>>,
    store: Option<Store>,
    last_save: Instant,
    dirty_position: Option<(String, Position, f32)>,
}

fn run(receiver: Receiver<Job>, emit: impl Fn(Event)) {
    let mut state = State {
        reader: None,
        families: None,
        store: Store::new().ok(),
        last_save: Instant::now(),
        dirty_position: None,
    };
    let mut pending: VecDeque<Job> = VecDeque::new();

    loop {
        let job = match pending.pop_front() {
            Some(job) => job,
            None => match receiver.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
        };
        // Drain whatever else is queued so we can skip intermediate frames.
        loop {
            match receiver.try_recv() {
                Ok(job) => pending.push_back(job),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let is_last = pending.is_empty();

        match job {
            Job::Shutdown => {
                flush(&mut state, true);
                break;
            }
            Job::Flush => flush(&mut state, true),
            Job::Open {
                generation,
                path,
                settings,
                theme,
                viewport,
            } => {
                flush(&mut state, true);
                match open_book(&mut state, &path, &settings, theme, viewport) {
                    Ok((view, title, toc, recents)) => {
                        emit(Event::Opened {
                            generation,
                            title,
                            toc,
                            recents,
                        });
                        // The resolved reading face is only known once a book,
                        // and with it a font system, exists.
                        let families = families(&mut state);
                        let current = state.reader.as_ref().map(|reader| reader.body_family());
                        emit(Event::Fonts { families, current });
                        emit(Event::Frame {
                            generation,
                            frame: Box::new(Frame::from_view(view)),
                        });
                        if let Some(reader) = state.reader.as_mut() {
                            reader.prefetch_neighbours();
                        }
                    }
                    Err(message) => emit(Event::Failed {
                        generation,
                        message,
                    }),
                }
            }
            Job::Apply {
                generation,
                command,
            } => {
                let Some(reader) = state.reader.as_mut() else {
                    continue;
                };
                match reader.apply(command) {
                    Ok(view) => {
                        state.dirty_position =
                            Some((reader.book_path().to_string(), view.position, view.progress));
                        if is_last {
                            emit(Event::Frame {
                                generation,
                                frame: Box::new(Frame::from_view(view)),
                            });
                            reader.prefetch_neighbours();
                        }
                        flush(&mut state, false);
                    }
                    Err(error) => emit(Event::Failed {
                        generation,
                        message: error.to_string(),
                    }),
                }
            }
            Job::Fonts => {
                let families = families(&mut state);
                let current = state.reader.as_ref().map(|reader| reader.body_family());
                emit(Event::Fonts { families, current });
            }
            Job::Search { generation, query } => {
                let Some(reader) = state.reader.as_mut() else {
                    continue;
                };
                let hits = reader.search(&query, 200);
                emit(Event::SearchDone { generation, hits });
            }
        }
    }
    flush(&mut state, true);
}

/// Installed font families, from the open book's font system when there is one
/// and from a throwaway system font database otherwise.
fn families(state: &mut State) -> Vec<String> {
    if let Some(families) = &state.families {
        return families.clone();
    }
    let families = match state.reader.as_ref() {
        Some(reader) => reader.font_families(),
        None => folio_core::layout::installed_families(&cosmic_text::FontSystem::new()),
    };
    state.families = Some(families.clone());
    families
}

type OpenResult = Result<(View, String, Vec<TocItem>, Vec<RecentBook>), String>;

fn open_book(
    state: &mut State,
    path: &Path,
    settings: &Settings,
    theme: Theme,
    viewport: (u32, u32),
) -> OpenResult {
    let book_path = path.to_string_lossy().into_owned();
    let start = state
        .store
        .as_ref()
        .and_then(|store| store.position_for(&book_path));

    let mut reader = Reader::open(path, settings, theme.preset, viewport, start)
        .map_err(|error| error.to_string())?;
    let view = reader
        .apply(Command::SetTheme(theme))
        .map_err(|error| error.to_string())?;

    let title = reader.meta().display_line();
    let toc = reader.toc().to_vec();

    let mut recents = Vec::new();
    if let Some(store) = state.store.as_ref() {
        let cover = reader
            .cover()
            .and_then(|bytes| thumbnail_png(&bytes))
            .and_then(|png| store.save_cover(&book_path, &png).ok());
        let entry = RecentBook {
            path: book_path,
            title: reader.meta().title.clone(),
            author: reader.meta().authors.first().cloned().unwrap_or_default(),
            position: view.position,
            progress: view.progress,
            cover,
            opened_at: now_secs(),
        };
        match store.touch_recent(entry) {
            Ok(list) => recents = list,
            Err(error) => tracing::warn!(%error, "could not update the recents list"),
        }
    }

    state.reader = Some(reader);
    Ok((view, title, toc, recents))
}

/// Write the pending reading position, honoring the debounce unless forced.
fn flush(state: &mut State, force: bool) {
    let Some((path, position, progress)) = state.dirty_position.clone() else {
        return;
    };
    if !force && state.last_save.elapsed() < SAVE_DEBOUNCE {
        return;
    }
    if let Some(store) = state.store.as_ref()
        && let Err(error) = store.update_position(&path, position, progress)
    {
        tracing::warn!(%error, "could not save the reading position");
    }
    state.last_save = Instant::now();
    state.dirty_position = None;
}

/// Scale a cover down and re-encode it as PNG for the shelf.
fn thumbnail_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let decoded = folio_core::layout::DecodedImage::decode(bytes)?;
    let (width, height) = if decoded.width >= decoded.height {
        let width = decoded.width.min(COVER_EDGE);
        (
            width,
            (decoded.height * width / decoded.width.max(1)).max(1),
        )
    } else {
        let height = decoded.height.min(COVER_EDGE);
        (
            (decoded.width * height / decoded.height.max(1)).max(1),
            height,
        )
    };
    let pixels = decoded.resized(width, height)?;
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(png)
}

/// Theme preset cycling order used by the toolbar button and the `t` key.
pub fn next_preset(current: ThemePreset) -> ThemePreset {
    match current {
        ThemePreset::Light => ThemePreset::Sepia,
        ThemePreset::Sepia => ThemePreset::Dark,
        ThemePreset::Dark => ThemePreset::Light,
    }
}
