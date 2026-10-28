use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

pub const DEFAULT_BANDS: usize = 64;
const MIN_FREQ: f32 = 40.0;
const MAX_FREQ: f32 = 16_000.0;
const FLOOR_DB: f32 = -70.0;

/// Streaming FFT analyser producing log-spaced band levels in `0.0..=1.0`.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    size: usize,
    hop: usize,
    window: Vec<f32>,
    ring: Vec<f32>,
    write: usize,
    since_last: usize,
    scratch: Vec<Complex32>,
    fft_scratch: Vec<Complex32>,
    /// FFT bin range `[start, end)` for every band.
    band_bins: Vec<(usize, usize)>,
    levels: Vec<f32>,
    attack: f32,
    release: f32,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: u32, fft_size: usize, bands: usize) -> Self {
        assert!(fft_size.is_power_of_two() && fft_size >= 64);
        assert!(bands > 0);
        let fft = FftPlanner::new().plan_fft_forward(fft_size);
        let window = (0..fft_size)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / (fft_size - 1) as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        let fft_scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        Self {
            band_bins: band_bins(sample_rate, fft_size, bands),
            fft,
            size: fft_size,
            hop: fft_size / 2,
            window,
            ring: vec![0.0; fft_size],
            write: 0,
            since_last: 0,
            scratch: vec![Complex32::default(); fft_size],
            fft_scratch,
            levels: vec![0.0; bands],
            attack: 0.65,
            release: 0.12,
        }
    }

    /// Analyser tuned for the shell: 2048-point FFT, 64 bands.
    pub fn for_sample_rate(sample_rate: u32) -> Self {
        Self::new(sample_rate, 2048, DEFAULT_BANDS)
    }

    pub fn bands(&self) -> &[f32] {
        &self.levels
    }

    /// Feeds mono samples. Returns `true` when at least one new frame was
    /// analysed and [`bands`](Self::bands) changed.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        let mut updated = false;
        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % self.size;
            self.since_last += 1;
            if self.since_last >= self.hop {
                self.since_last = 0;
                self.analyse();
                updated = true;
            }
        }
        updated
    }

    /// Lets the bars fall when no audio arrives (loopback capture delivers no
    /// packets during silence). Returns `true` while anything is still lit.
    pub fn decay(&mut self, factor: f32) -> bool {
        let mut any = false;
        for l in &mut self.levels {
            *l *= factor;
            if *l < 0.004 {
                *l = 0.0;
            } else {
                any = true;
            }
        }
        any
    }

    fn analyse(&mut self) {
        // Oldest sample sits at `write` in the ring buffer.
        for i in 0..self.size {
            let s = self.ring[(self.write + i) % self.size];
            self.scratch[i] = Complex32::new(s * self.window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.scratch, &mut self.fft_scratch);

        // Hann window coherent gain is 0.5; normalise so a full-scale sine
        // reads ~0 dB.
        let norm = 2.0 / (self.size as f32 * 0.5);
        for (band, &(start, end)) in self.band_bins.iter().enumerate() {
            let peak = self.scratch[start..end].iter().map(|c| c.norm() * norm).fold(0.0f32, f32::max);
            let db = 20.0 * peak.max(1e-9).log10();
            let target = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
            let level = &mut self.levels[band];
            let k = if target > *level { self.attack } else { self.release };
            *level += (target - *level) * k;
        }
    }
}

fn band_bins(sample_rate: u32, fft_size: usize, bands: usize) -> Vec<(usize, usize)> {
    let nyquist_bin = fft_size / 2;
    let hz_per_bin = sample_rate as f32 / fft_size as f32;
    let max_freq = MAX_FREQ.min(sample_rate as f32 / 2.0);
    let ratio = (max_freq / MIN_FREQ).ln();
    let mut out = Vec::with_capacity(bands);
    let mut prev_end = 1usize;
    for b in 0..bands {
        let lo = MIN_FREQ * (ratio * b as f32 / bands as f32).exp();
        let hi = MIN_FREQ * (ratio * (b + 1) as f32 / bands as f32).exp();
        let start = ((lo / hz_per_bin).floor() as usize).max(prev_end.saturating_sub(1)).max(1);
        let end = ((hi / hz_per_bin).ceil() as usize).max(start + 1).min(nyquist_bin);
        let start = start.min(end - 1);
        out.push((start, end));
        prev_end = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin()).collect()
    }

    fn loudest(bands: &[f32]) -> usize {
        bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0
    }

    #[test]
    fn bands_are_ordered_and_in_range() {
        let bins = band_bins(48_000, 2048, 64);
        assert_eq!(bins.len(), 64);
        for w in bins.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
        assert!(bins.iter().all(|(s, e)| s < e && *e <= 1024));
    }

    #[test]
    fn low_tone_lights_low_bands_high_tone_high_bands() {
        let mut a = SpectrumAnalyzer::for_sample_rate(48_000);
        assert!(a.push(&sine(100.0, 48_000, 8192)));
        let low = loudest(a.bands());
        let mut b = SpectrumAnalyzer::for_sample_rate(48_000);
        b.push(&sine(8_000.0, 48_000, 8192));
        let high = loudest(b.bands());
        assert!(low < 16, "low tone peaked at band {low}");
        assert!(high > 48, "high tone peaked at band {high}");
        assert!(a.bands()[low] > 0.8);
    }

    #[test]
    fn silence_is_dark_and_decay_reaches_zero() {
        let mut a = SpectrumAnalyzer::for_sample_rate(44_100);
        a.push(&vec![0.0; 4096]);
        assert!(a.bands().iter().all(|l| *l == 0.0));
        a.push(&sine(440.0, 44_100, 8192));
        let mut steps = 0;
        while a.decay(0.8) {
            steps += 1;
            assert!(steps < 100);
        }
        assert!(a.bands().iter().all(|l| *l == 0.0));
    }
}
