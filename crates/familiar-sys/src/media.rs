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
    /// Playback position and length, when the app reports a timeline.
    pub timeline: Option<Timeline>,
}

/// Where playback was when the session was read. The live position is
/// extrapolated with [`Timeline::position_now`] while playing.
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub read_at: std::time::Instant,
}

/// Two reads describe the same playback when the track length matches and
/// the extrapolated positions agree, so the periodic timeline events apps
/// send while playing do not count as changes.
impl PartialEq for Timeline {
    fn eq(&self, other: &Self) -> bool {
        self.duration_ms == other.duration_ms && self.position_now(true).abs_diff(other.position_now(true)) < 1500
    }
}

impl Timeline {
    pub fn position_now(&self, playing: bool) -> u64 {
        let pos = if playing { self.position_ms + self.read_at.elapsed().as_millis() as u64 } else { self.position_ms };
        pos.min(self.duration_ms)
    }
}

/// `m:ss` (or `h:mm:ss`) for a position in milliseconds.
pub fn format_time(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
    /// Jump to this position (milliseconds from the start).
    Seek(u64),
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
    timeline_token: i64,
}

impl Drop for Watched {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props_token);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_token);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline_token);
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
                    let c = tx.clone();
                    let props_token = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = a.send(Msg::Refresh);
                        Ok(())
                    }))?;
                    let playback_token = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = b.send(Msg::Refresh);
                        Ok(())
                    }))?;
                    let timeline_token = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = c.send(Msg::Refresh);
                        Ok(())
                    }))?;
                    watched = Some(Watched { session, props_token, playback_token, timeline_token });
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
                        // WinRT positions are in 100 ns ticks.
                        MediaCommand::Seek(ms) => w.session.TryChangePlaybackPositionAsync(ms as i64 * 10_000),
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
        timeline: read_timeline(session),
    })
}

/// 100 ns ticks since 1601, the epoch of WinRT `DateTime`.
fn now_ticks() -> i64 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn read_timeline(session: &Session) -> Option<Timeline> {
    let t = session.GetTimelineProperties().ok()?;
    let start = t.StartTime().ok()?.Duration;
    let end = t.EndTime().ok()?.Duration;
    let position = t.Position().ok()?.Duration;
    let duration = end - start;
    if duration <= 0 {
        return None;
    }
    // The position is as of LastUpdatedTime; bring it up to now when the
    // app only reports it occasionally (Spotify does).
    let playing = session.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).is_ok_and(|s| s == Status::Playing);
    let stale = t.LastUpdatedTime().map(|u| (now_ticks() - u.UniversalTime).max(0)).unwrap_or(0);
    let pos = (position - start + if playing { stale } else { 0 }).clamp(0, duration);
    Some(Timeline {
        position_ms: (pos / 10_000) as u64,
        duration_ms: (duration / 10_000) as u64,
        read_at: std::time::Instant::now(),
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

#[cfg(test)]
mod tests {
    use super::{Timeline, format_time};

    #[test]
    fn times_are_formatted() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(61_500), "1:01");
        assert_eq!(format_time(3_725_000), "1:02:05");
    }

    #[test]
    fn timeline_never_runs_past_the_end() {
        let t = Timeline {
            position_ms: 9_000,
            duration_ms: 10_000,
            read_at: std::time::Instant::now() - std::time::Duration::from_secs(5),
        };
        assert_eq!(t.position_now(true), 10_000);
        assert_eq!(t.position_now(false), 9_000);
    }
}
