//! WASAPI loopback capture: what the speakers play, no microphone involved.
//!
//! The capture thread is event-driven. During silence the audio engine
//! delivers no loopback packets, so the thread stays blocked in
//! `WaitForMultipleObjects` and costs nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};

use crate::com::ComGuard;

struct Event(HANDLE);
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Control {
    wake: Event,
    stop: AtomicBool,
    paused: AtomicBool,
}

pub struct LoopbackCapture {
    control: Arc<Control>,
    join: Option<JoinHandle<()>>,
}

impl LoopbackCapture {
    /// `on_samples(mono, sample_rate)` runs on the capture thread.
    pub fn spawn(mut on_samples: impl FnMut(&[f32], u32) + Send + 'static) -> std::io::Result<Self> {
        let wake = unsafe { CreateEventW(None, false, false, None) }.map_err(std::io::Error::other)?;
        let control =
            Arc::new(Control { wake: Event(wake), stop: AtomicBool::new(false), paused: AtomicBool::new(false) });
        let c = control.clone();
        let join = std::thread::Builder::new().name("audio-loopback".into()).spawn(move || {
            let _com = ComGuard::mta();
            while !c.stop.load(Ordering::Acquire) {
                if c.paused.load(Ordering::Acquire) {
                    unsafe { WaitForMultipleObjects(&[c.wake.0], false, INFINITE) };
                    continue;
                }
                match unsafe { capture(&c, &mut on_samples) } {
                    Ok(()) => {}
                    Err(e) => {
                        // Device unplugged / default device switched: retry
                        // after a short back-off instead of spinning.
                        log::info!("loopback capture restarting: {e}");
                        unsafe { WaitForMultipleObjects(&[c.wake.0], false, 1500) };
                    }
                }
            }
        })?;
        Ok(Self { control, join: Some(join) })
    }

    /// Paused capture releases the device and sleeps (calm mode, battery saver).
    pub fn set_paused(&self, paused: bool) {
        if self.control.paused.swap(paused, Ordering::AcqRel) != paused {
            unsafe {
                let _ = SetEvent(self.control.wake.0);
            }
        }
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        unsafe {
            let _ = SetEvent(self.control.wake.0);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[derive(Clone, Copy)]
enum SampleFormat {
    F32,
    I16,
    I32,
}

unsafe fn capture(c: &Control, on_samples: &mut dyn FnMut(&[f32], u32)) -> windows::core::Result<()> {
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let fmt_ptr = unsafe { client.GetMixFormat()? };
    let fmt: WAVEFORMATEX = unsafe { *fmt_ptr };
    let channels = fmt.nChannels.max(1) as usize;
    let rate = fmt.nSamplesPerSec;
    let format = match (fmt.wFormatTag as u32, fmt.wBitsPerSample) {
        (WAVE_FORMAT_IEEE_FLOAT, 32) => SampleFormat::F32,
        (WAVE_FORMAT_EXTENSIBLE, bits) => {
            let ext = unsafe { &*(fmt_ptr as *const WAVEFORMATEXTENSIBLE) };
            let sub = ext.SubFormat;
            match (sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, bits) {
                (true, 32) => SampleFormat::F32,
                (false, 16) => SampleFormat::I16,
                (false, 32) => SampleFormat::I32,
                _ => SampleFormat::F32,
            }
        }
        (_, 16) => SampleFormat::I16,
        _ => SampleFormat::I32,
    };

    // 20 ms shared-mode buffer.
    let init = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            200_000,
            0,
            fmt_ptr,
            None,
        )
    };
    unsafe { CoTaskMemFree(Some(fmt_ptr as *const _)) };
    init?;

    let ready = Event(unsafe { CreateEventW(None, false, false, None)? });
    unsafe { client.SetEventHandle(ready.0)? };
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    unsafe { client.Start()? };

    let mut mono: Vec<f32> = Vec::with_capacity(4096);
    let result = loop {
        let r = unsafe { WaitForMultipleObjects(&[ready.0, c.wake.0], false, INFINITE) };
        if r != WAIT_OBJECT_0 {
            // Woken by control: stop or pause.
            break Ok(());
        }
        let drained: windows::core::Result<()> = (|| loop {
            let packet = unsafe { capture.GetNextPacketSize()? };
            if packet == 0 {
                return Ok(());
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? };
            mono.clear();
            let n = frames as usize;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                mono.resize(n, 0.0);
            } else {
                unsafe { downmix(data, n, channels, format, &mut mono) };
            }
            unsafe { capture.ReleaseBuffer(frames)? };
            on_samples(&mono, rate);
        })();
        if let Err(e) = drained {
            break Err(e);
        }
    };
    unsafe {
        let _ = client.Stop();
    }
    result
}

unsafe fn downmix(data: *const u8, frames: usize, channels: usize, format: SampleFormat, out: &mut Vec<f32>) {
    let inv = 1.0 / channels as f32;
    match format {
        SampleFormat::F32 => {
            let s = unsafe { std::slice::from_raw_parts(data as *const f32, frames * channels) };
            out.extend(s.chunks_exact(channels).map(|f| f.iter().sum::<f32>() * inv));
        }
        SampleFormat::I16 => {
            let s = unsafe { std::slice::from_raw_parts(data as *const i16, frames * channels) };
            out.extend(s.chunks_exact(channels).map(|f| f.iter().map(|v| *v as f32 / 32768.0).sum::<f32>() * inv));
        }
        SampleFormat::I32 => {
            let s = unsafe { std::slice::from_raw_parts(data as *const i32, frames * channels) };
            out.extend(
                s.chunks_exact(channels).map(|f| f.iter().map(|v| *v as f32 / 2_147_483_648.0).sum::<f32>() * inv),
            );
        }
    }
}
