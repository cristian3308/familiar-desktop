//! WASAPI loopback → FFT (on the capture thread) → UI at ≤30 fps.
//!
//! Capture only runs while something is playing and the shell is not in
//! calm mode. When packets stop arriving the bars decay on a UI timer that
//! stops itself as soon as everything is dark.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use familiar_signal::SpectrumAnalyzer;
use familiar_sys::audio::LoopbackCapture;
use slint::{Timer, TimerMode};

use crate::app::{App, post, with_app};

const FRAME: Duration = Duration::from_millis(33);

pub struct AudioPipe {
    capture: RefCell<Option<LoopbackCapture>>,
    active: Cell<bool>,
    bands: RefCell<Vec<f32>>,
    last_frame: Cell<Instant>,
    decay: Timer,
}

impl AudioPipe {
    pub fn new() -> Self {
        Self {
            capture: RefCell::new(None),
            active: Cell::new(false),
            bands: RefCell::new(Vec::new()),
            last_frame: Cell::new(Instant::now()),
            decay: Timer::default(),
        }
    }

    pub fn set_active(&self, on: bool) {
        if self.active.replace(on) == on {
            return;
        }
        log::debug!("audio capture {}", if on { "on" } else { "paused" });
        let mut cap = self.capture.borrow_mut();
        match (&*cap, on) {
            (Some(c), _) => c.set_paused(!on),
            (None, true) => match spawn_capture() {
                Ok(c) => *cap = Some(c),
                Err(e) => log::warn!("loopback capture unavailable: {e}"),
            },
            (None, false) => {}
        }
    }

    pub fn on_bands(&self, app: &App, bands: Vec<f32>) {
        if !self.active.get() {
            return;
        }
        self.last_frame.set(Instant::now());
        app.render_audio(&bands);
        *self.bands.borrow_mut() = bands;
        if !self.decay.running() {
            self.decay.start(TimerMode::Repeated, FRAME, || {
                with_app(|a| a.audio.decay_tick(a));
            });
        }
    }

    fn decay_tick(&self, app: &App) {
        if self.last_frame.get().elapsed() < FRAME * 3 {
            return;
        }
        let mut bands = self.bands.borrow_mut();
        let mut lit = false;
        for b in bands.iter_mut() {
            *b *= 0.8;
            if *b < 0.004 {
                *b = 0.0;
            } else {
                lit = true;
            }
        }
        app.render_audio(&bands);
        if !lit {
            self.decay.stop();
        }
    }

    pub fn shutdown(&self) {
        self.decay.stop();
        self.capture.borrow_mut().take();
    }
}

fn spawn_capture() -> std::io::Result<LoopbackCapture> {
    let mut analyzer: Option<(u32, SpectrumAnalyzer)> = None;
    let mut last_sent = Instant::now();
    LoopbackCapture::spawn(move |samples, rate| {
        let an = match &mut analyzer {
            Some((r, an)) if *r == rate => an,
            _ => &mut analyzer.insert((rate, SpectrumAnalyzer::for_sample_rate(rate))).1,
        };
        if an.push(samples) && last_sent.elapsed() >= FRAME {
            last_sent = Instant::now();
            let bands = an.bands().to_vec();
            post(move |app| app.audio.on_bands(app, bands));
        }
    })
}
