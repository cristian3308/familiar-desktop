//! Now Playing via WinRT `GlobalSystemMediaTransportControlsSessionManager`
//! (the same source as the Windows volume flyout: Spotify, browsers, ...).
//!
//! WinRT raises change events on its own thread pool; they only post a
//! message to our worker thread, which blocks on a channel when idle.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{Buffer, DataReader, IRandomAccessStreamReference, InputStreamOptions};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};

const THUMB_SIZE: u32 = 160;

#[derive(Clone, Debug, PartialEq)]
pub struct Artwork {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub source_app: String,
    pub artwork: Option<std::sync::Arc<Artwork>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
}

enum Msg {
    SessionChanged,
    Refresh,
    Command(MediaCommand),
    Quit,
}

pub struct MediaService {
    tx: Sender<Msg>,
    join: Option<JoinHandle<()>>,
}

impl MediaService {
    /// `on_change(None)` means nothing is playing (no media session).
    pub fn spawn(on_change: impl Fn(Option<NowPlaying>) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let worker_tx = tx.clone();
        let join = std::thread::Builder::new().name("media".into()).spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            if let Err(e) = run(rx, worker_tx, &on_change) {
                log::warn!("media session service stopped: {e}");
            }
            unsafe { RoUninitialize() };
        })?;
        Ok(Self { tx, join: Some(join) })
    }

    pub fn send(&self, cmd: MediaCommand) {
        let _ = self.tx.send(Msg::Command(cmd));
    }
}

impl Drop for MediaService {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Watched {
    session: Session,
    props_token: i64,
    playback_token: i64,
}

impl Drop for Watched {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props_token);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_token);
    }
}

fn run(rx: Receiver<Msg>, tx: Sender<Msg>, on_change: &dyn Fn(Option<NowPlaying>)) -> windows::core::Result<()> {
    let manager = Manager::RequestAsync()?.join()?;
    let session_tx = tx.clone();
    let token = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = session_tx.send(Msg::SessionChanged);
        Ok(())
    }))?;

    let mut watched: Option<Watched> = None;
    let mut last: Option<NowPlaying> = None;
    // Artwork is decoded once per track, not on every play/pause.
    let mut art_key = String::new();
    let mut art: Option<std::sync::Arc<Artwork>> = None;

    let mut msg = Msg::SessionChanged;
    loop {
        match msg {
            Msg::Quit => break,
            Msg::SessionChanged => {
                watched = None;
                if let Ok(session) = manager.GetCurrentSession() {
                    let a = tx.clone();
                    let b = tx.clone();
                    let props_token = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = a.send(Msg::Refresh);
                        Ok(())
                    }))?;
                    let playback_token = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = b.send(Msg::Refresh);
                        Ok(())
                    }))?;
                    watched = Some(Watched { session, props_token, playback_token });
                }
                let _ = tx.send(Msg::Refresh);
            }
            Msg::Refresh => {
                let now = watched.as_ref().and_then(|w| read_state(&w.session, &mut art_key, &mut art).ok());
                if now != last {
                    last = now.clone();
                    on_change(now);
                }
            }
            Msg::Command(cmd) => {
                if let Some(w) = &watched {
                    let op = match cmd {
                        MediaCommand::PlayPause => w.session.TryTogglePlayPauseAsync(),
                        MediaCommand::Next => w.session.TrySkipNextAsync(),
                        MediaCommand::Previous => w.session.TrySkipPreviousAsync(),
                    };
                    if let Ok(op) = op {
                        let _ = op.join();
                    }
                }
            }
        }
        // Coalesce bursts: apps fire several property events per track change.
        msg = match rx.recv() {
            Ok(Msg::Refresh) => {
                let mut next = Msg::Refresh;
                while let Ok(m) = rx.recv_timeout(std::time::Duration::from_millis(60)) {
                    if !matches!(m, Msg::Refresh) {
                        next = m;
                        break;
                    }
                }
                if !matches!(next, Msg::Refresh) {
                    // Handle the refresh first, then the other message.
                    let _ = tx.send(next);
                }
                Msg::Refresh
            }
            Ok(m) => m,
            Err(_) => break,
        };
    }
    let _ = manager.RemoveCurrentSessionChanged(token);
    Ok(())
}

fn read_state(
    session: &Session,
    art_key: &mut String,
    art: &mut Option<std::sync::Arc<Artwork>>,
) -> windows::core::Result<NowPlaying> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();
    let key = format!("{title}\u{1}{artist}\u{1}{album}");
    if *art_key != key {
        *art_key = key;
        *art = props.Thumbnail().ok().and_then(|t| read_artwork(&t).ok()).map(std::sync::Arc::new);
    }
    Ok(NowPlaying {
        title,
        artist,
        album,
        playing,
        source_app: session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default(),
        artwork: art.clone(),
    })
}

fn read_artwork(reference: &IRandomAccessStreamReference) -> windows::core::Result<Artwork> {
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?.min(8 * 1024 * 1024) as u32;
    let buffer = Buffer::Create(size)?;
    let filled = stream.ReadAsync(&buffer, size, InputStreamOptions::None)?.join()?;
    let reader = DataReader::FromBuffer(&filled)?;
    let mut bytes = vec![0u8; filled.Length()? as usize];
    reader.ReadBytes(&mut bytes)?;
    let img = image::load_from_memory(&bytes).map_err(|_| windows::core::Error::empty())?;
    let thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgba8();
    Ok(Artwork { width: thumb.width(), height: thumb.height(), rgba: thumb.into_raw() })
}
