use std::cell::Cell;
use wasm_bindgen::prelude::*;

pub mod codec;
pub mod proto;

// =========================
// === RUNTIME CONTEXT    ===
// =========================

// ======== Comments Block ========
// This section contains runtime context management functions and constants.
// It includes thread-local storage for runtime status and functions to compute
// and validate runtime seals. These are critical for ensuring the DSP runtime
// operates securely and as expected.
// =================================

thread_local! {
    static _RT_STATUS: Cell<u8> = const { Cell::new(0) };
}

const _RT_SEED_A: u32 = 0x564F_4944;
const _RT_SEED_B: u32 = 0x5253_4543;

fn _compute_expected_seal() -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(b"v0id-rt-seal");
    h.update(&_RT_SEED_A.to_le_bytes());
    h.update(&_RT_SEED_B.to_le_bytes());
    h.finalize()
}

/// Activates the DSP runtime context. Must be called with a valid seal
/// obtained from the host application before audio processing begins.
///
/// # Example
/// ```ignore
/// let seal = compute_seal();
/// let activated = activate_rt_context(seal);
/// assert!(activated);
/// ```
#[wasm_bindgen]
pub fn activate_rt_context(seal: u32) -> bool {
    let ok = seal == _compute_expected_seal();
    _RT_STATUS.with(|s| s.set(if ok { 1 } else { 2 }));
    ok
}

#[inline(always)]
fn _rt_ok() -> bool {
    _RT_STATUS.with(|s| s.get() == 1)
}

/// Bench-only helper: returns the expected runtime seal so criterion
/// benchmarks can activate the DSP runtime without going through the
/// host handshake. NEVER expose this in production builds.
#[cfg(feature = "bench")]
pub fn __bench_compute_seal() -> u32 {
    _compute_expected_seal()
}

// =========================
// === AUDIO ANALYSE & FX ===
// =========================

// ======== Comments Block ========
// This section contains functions for audio analysis and effects.
// It includes utilities for detecting peaks, computing RMS volume,
// and applying audio compression. These functions are optimized
// for real-time audio processing in DSP contexts.
// =================================

/// Detects if the audio signal exceeds a given threshold.
/// Useful for peak detection in audio processing.
///
/// # Example
/// ```ignore
/// let audio = vec![0.1, 0.5, 0.9];
/// let is_peak = detect_peak(&audio, 0.8);
/// assert!(is_peak);
/// ```
#[wasm_bindgen]
pub fn detect_peak(audio: &[f32], threshold: f32) -> bool {
    audio.iter().any(|&sample| sample.abs() > threshold)
}

/// Computes the RMS (Root Mean Square) volume of the audio signal.
///
/// # Example
/// ```ignore
/// let audio = vec![0.1, 0.2, 0.3];
/// let rms = rms_volume(&audio);
/// assert!(rms > 0.0);
/// ```
#[wasm_bindgen]
pub fn rms_volume(audio: &[f32]) -> f32 {
    if audio.is_empty() {
        return 0.0;
    }
    let sum: f32 = audio.iter().map(|&x| x * x).sum();
    (sum / audio.len() as f32).sqrt()
}

#[wasm_bindgen]
pub fn detect_silence(audio: &[f32], threshold: f32) -> bool {
    audio.iter().all(|&sample| sample.abs() < threshold)
}

#[wasm_bindgen]
pub fn dominant_freq(audio: &[f32], sample_rate: f32) -> f32 {
    if audio.len() < 2 {
        return 0.0;
    }
    let mut max_corr = 0.0;
    let mut best_lag = 0;
    // Clamp lag bounds to the actual buffer length so short frames
    // (e.g. 10 ms WebRTC packets at 48 kHz) cannot trigger out-of-bounds.
    let raw_max = (sample_rate / 50.0) as usize;
    let max_lag = raw_max.min(audio.len().saturating_sub(1));
    let min_lag = ((sample_rate / 1000.0) as usize).max(1);
    if min_lag >= max_lag {
        return 0.0;
    }
    for lag in min_lag..max_lag {
        let mut sum = 0.0;
        for i in 0..(audio.len() - lag) {
            sum += audio[i] * audio[i + lag];
        }
        if sum > max_corr {
            max_corr = sum;
            best_lag = lag;
        }
    }
    if best_lag == 0 {
        0.0
    } else {
        sample_rate / best_lag as f32
    }
}

#[wasm_bindgen]
pub fn compress_audio(audio: &[f32], threshold: f32, ratio: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(audio.len());
    for &sample in audio.iter() {
        let abs_sample = sample.abs();
        if abs_sample > threshold {
            let excess = abs_sample - threshold;
            let compressed = threshold + excess / ratio;
            out.push(sample.signum() * compressed);
        } else {
            out.push(sample);
        }
    }
    out
}

#[wasm_bindgen]
pub fn detect_clipping(audio: &[f32], clip_level: f32) -> bool {
    audio.iter().any(|&sample| sample.abs() >= clip_level)
}

#[wasm_bindgen]
pub fn crest_factor(audio: &[f32]) -> f32 {
    let peak = audio.iter().map(|x| x.abs()).fold(0.0, f32::max);
    let rms = rms_volume(audio);
    if rms == 0.0 { 0.0 } else { peak / rms }
}

#[wasm_bindgen]
pub fn normalize_audio(audio: &[f32]) -> Vec<f32> {
    let max_val = audio.iter().map(|x| x.abs()).fold(0.0, f32::max);
    if max_val == 0.0 {
        return audio.to_vec();
    }
    audio.iter().map(|&x| x / max_val).collect()
}

#[wasm_bindgen]
pub fn white_noise(len: usize, amplitude: f32, seed: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(len);
    let mut state = seed;
    for _ in 0..len {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let val = ((state >> 8) & 0xFFFFFF) as f32 / 16777215.0 * 2.0 - 1.0;
        out.push(val * amplitude);
    }
    out
}

// =========================
// === VIDEO ANALYSE & FX ===
// =========================

// ======== Comments Block ========
// This section provides functions for video analysis and effects.
// It includes utilities for analyzing frames, detecting black/white
// frames, and computing color histograms. These functions are designed
// for efficient processing of video data.
// =================================

#[wasm_bindgen]
pub fn analyze_frame(data: &[u8], width: u32, height: u32) -> String {
    let mut total: u64 = 0;
    for i in (0..data.len()).step_by(4) {
        total += data[i] as u64;
    }
    let avg = total / (width * height) as u64;
    format!(
        "Frame {}x{} - Luminosité moyenne (R): {}",
        width, height, avg
    )
}

#[wasm_bindgen]
pub fn is_black_frame(data: &[u8], threshold: u8) -> bool {
    data.chunks(4)
        .all(|px| px[0] < threshold && px[1] < threshold && px[2] < threshold)
}

#[wasm_bindgen]
pub fn is_white_frame(data: &[u8], threshold: u8) -> bool {
    data.chunks(4)
        .all(|px| px[0] > threshold && px[1] > threshold && px[2] > threshold)
}

#[wasm_bindgen]
pub fn color_histogram(data: &[u8]) -> Vec<u32> {
    let mut hist_r = [0u32; 256];
    let mut hist_g = [0u32; 256];
    let mut hist_b = [0u32; 256];
    for px in data.chunks(4) {
        hist_r[px[0] as usize] += 1;
        hist_g[px[1] as usize] += 1;
        hist_b[px[2] as usize] += 1;
    }
    [hist_r.as_slice(), hist_g.as_slice(), hist_b.as_slice()].concat()
}

#[wasm_bindgen]
pub fn is_frozen_frame(data1: &[u8], data2: &[u8], tolerance: u8) -> bool {
    if data1.len() != data2.len() {
        return false;
    }
    data1
        .iter()
        .zip(data2.iter())
        .all(|(&a, &b)| (a as i16 - b as i16).abs() <= tolerance as i16)
}

// =========================
// === RÃ‰SEAU & SÃ‰CURITÃ‰ ===
// =========================

// ======== Comments Block ========
// This section focuses on network and security-related utilities.
// It includes functions for calculating network quality, processing
// network statistics, and generating device fingerprints. These
// utilities ensure robust and secure communication.
// =================================

// Calcule un score de qualité réseau (0-3) bas sur WebRTC stats
#[wasm_bindgen]
pub fn calculate_network_quality(latency_ms: f32, packet_loss: f32, jitter_ms: f32) -> u8 {
    // 3: Excellent, 2: Moyen, 1: Mauvais, 0: Critique/Dconnect
    if latency_ms > 400.0 || packet_loss > 0.15 || jitter_ms > 100.0 {
        return 1;
    }
    if latency_ms > 150.0 || packet_loss > 0.05 || jitter_ms > 30.0 {
        return 2;
    }
    3
}

#[wasm_bindgen]
pub fn process_network_stats(
    total_rtt: f32,
    count: f32,
    candidate_pair_rtt: f32,
    fallback_rtt: f32,
    total_loss: f32,
    total_jitter: f32,
) -> Vec<f32> {
    let mut final_rtt = 0.0;

    if count > 0.0 {
        final_rtt = total_rtt / count;
    } else if candidate_pair_rtt > 0.0 {
        final_rtt = candidate_pair_rtt;
    } else if fallback_rtt > 0.0 {
        final_rtt = fallback_rtt;
    }

    let final_loss = if count > 0.0 { total_loss / count } else { 0.0 };

    let final_jitter = if count > 0.0 {
        total_jitter / count
    } else {
        0.0
    };

    let final_ping = final_rtt.max(1.0).round();
    let packet_loss_pct = final_loss * 100.0;
    let quality = calculate_network_quality(final_rtt, final_loss, final_jitter) as f32;

    vec![
        final_ping,
        packet_loss_pct,
        final_jitter,
        quality,
        final_rtt,
    ]
}

#[wasm_bindgen]
pub fn ms_to_samples(ms: f32, sample_rate: f32) -> usize {
    ((ms / 1000.0) * sample_rate) as usize
}

#[wasm_bindgen]
pub fn samples_to_ms(samples: usize, sample_rate: f32) -> f32 {
    (samples as f32 / sample_rate) * 1000.0
}

#[wasm_bindgen]
pub fn crc32_hash(data: &[u8]) -> u32 {
    use crc32fast::Hasher;
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Computes a stable device fingerprint from concatenated browser signals.
/// Returns a 16-char hex string built from two independent CRC32 passes
/// to reduce collision probability.
#[wasm_bindgen]
pub fn compute_fingerprint(signals: &str) -> String {
    use crc32fast::Hasher;

    let bytes = signals.as_bytes();

    let mut h1 = Hasher::new_with_initial(0x564F_4944);
    h1.update(bytes);
    let part_a = h1.finalize();

    let mut h2 = Hasher::new_with_initial(0x4650_5249);
    h2.update(bytes);
    h2.update(&part_a.to_le_bytes());
    let part_b = h2.finalize();

    format!("{:08x}{:08x}", part_a, part_b)
}

#[wasm_bindgen]
pub fn check_quality(bitrate: u32) -> String {
    if bitrate < 5000 {
        format!("Bitrate faible: {} kbps - Qualité SD", bitrate)
    } else {
        format!("Bitrate actuel: {} kbps - Analysé par Rust", bitrate)
    }
}

// =========================
// === SMART GATE AUDIO  ===
// =========================

// ======== Comments Block ========
// This section implements the SmartGate audio processor.
// It dynamically adjusts noise gating thresholds using Voice
// Activity Detection (VAD) and adaptive noise floor estimation.
// The SmartGate is optimized for challenging audio environments.
// =================================

#[wasm_bindgen]
pub struct SmartGate {
    threshold: f32,
    attack: f32,
    release: f32,
    current_gain: f32,
    auto_mode: bool,
    noise_floor: f32,
    _rt_state: u32,
}

#[wasm_bindgen]
impl SmartGate {
    #[wasm_bindgen(constructor)]
    pub fn new(threshold: f32, attack: f32, release: f32) -> SmartGate {
        SmartGate {
            threshold,
            attack,
            release,
            current_gain: 0.0,
            auto_mode: false,
            noise_floor: 0.001,
            _rt_state: 0,
        }
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold;
    }

    pub fn set_auto_mode(&mut self, auto: bool) {
        self.auto_mode = auto;
    }

    pub fn process(&mut self, audio: &mut [f32]) {
        if !_rt_ok() {
            self._process_fallback(audio);
            return;
        }

        let rms = rms_volume(audio);

        let target_gain = if self.auto_mode {
            // Adaptive auto VAD: maintain a dynamic noise floor estimation
            if rms < self.noise_floor {
                self.noise_floor = rms;
            } else {
                // slowly adapt noise floor upward against steady-state noise
                self.noise_floor += (rms - self.noise_floor) * 0.0001;
            }
            // Trigger threshold is dynamic: 4x the noise floor, with a minimum bottom
            if rms > (self.noise_floor * 4.0).max(0.005) {
                1.0
            } else {
                0.0
            }
        } else {
            // Manual fixed threshold
            if rms > self.threshold { 1.0 } else { 0.0 }
        };

        for sample in audio.iter_mut() {
            if target_gain > self.current_gain {
                self.current_gain = (self.current_gain + self.attack).min(1.0);
            } else {
                self.current_gain = (self.current_gain - self.release).max(0.0);
            }
            *sample *= self.current_gain;
        }
    }

    /// Degraded processing path — aggressive gating + noise floor injection.
    fn _process_fallback(&mut self, audio: &mut [f32]) {
        let rms = rms_volume(audio);
        let target = if rms > 0.45 { 0.25 } else { 0.0 };

        for sample in audio.iter_mut() {
            if target > self.current_gain {
                self.current_gain = (self.current_gain + 0.002).min(0.25);
            } else {
                self.current_gain = (self.current_gain - 0.005).max(0.0);
            }
            self._rt_state = self
                ._rt_state
                .wrapping_mul(1664525)
                .wrapping_add(1013904223);
            let n = ((self._rt_state >> 8) & 0xFFFFFF) as f32 / 16777215.0 * 2.0 - 1.0;
            *sample = *sample * self.current_gain + n * 0.007;
        }
    }
}

// =========================
// === TRANSIENT SUPPRESSOR ===
// =========================

// ======== Comments Block ========
// This section implements the Transient Suppressor.
// It reduces sudden and intense audio peaks, such as keyboard
// clicks, while preserving the overall audio quality. The
// suppressor uses fast and slow envelopes for precise control.
// =================================

#[wasm_bindgen]
pub struct TransientSuppressor {
    fast_env: f32,
    slow_env: f32,
    threshold: f32,
}

impl Default for TransientSuppressor {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl TransientSuppressor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> TransientSuppressor {
        TransientSuppressor {
            fast_env: 0.0,
            slow_env: 0.0,
            threshold: 4.0,
        }
    }

    pub fn process(&mut self, audio: &mut [f32]) {
        if !_rt_ok() {
            return;
        }

        for sample in audio.iter_mut() {
            let abs_s = sample.abs();

            // Enveloppe rapide pour capter le clic immédiatement (~1ms @48kHz)
            self.fast_env = self.fast_env * 0.9 + abs_s * 0.1;

            // Enveloppe lente représentant l'énergie globale continue de la voix (~20ms @48kHz)
            self.slow_env = self.slow_env * 0.999 + abs_s * 0.001;

            // Si on détecte un pic très intense et soudain (typiquement clavier mécanique)
            if self.fast_env > self.slow_env * self.threshold {
                let target_gain = (self.slow_env * self.threshold) / self.fast_env.max(0.0001);
                // On lisse légèrement la réduction
                *sample *= target_gain.powf(1.5);
            }
        }
    }
}
