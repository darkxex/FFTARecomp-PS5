//! The audio of the PC build, driven from the emulation loop instead of a sound-card callback.
//!
//! `SincResampler`, `FifoInterp`, `DcBlock`, the constants and the engine detection are the code of
//! `crates/recomp/src/main.rs` unchanged; `Mixer::render` is the body of its output callback (the faithful
//! mix, the per-channel "enhanced" path, the crossfades, the volume) producing 48 kHz stereo i16, and
//! `Mixer::feed` is what the play loop hands over after each emulated frame.
#![allow(dead_code)]
use gba_core::mem::MemMap;
use gba_core::{Bus as _, Machine};
use std::collections::VecDeque;

const SINC_TAPS: usize = 24;
const SINC_PHASES: usize = 128;
/// Ring fill the rate control steers toward (~62 ms at the 65536 Hz tap).
const RING_TARGET: usize = 4096;
/// Hardware clip rail in the float mix domain (±0x200 units = ±0.5).
const RAIL: f32 = 0.5;
/// Output gain calibrating the rail toward device full scale. 1.6 puts
/// the rail ~2 dB under full scale — the headroom the enhanced path's
/// soft knee lands over-rail peaks in.
const OUT_GAIN: f32 = 1.6;

/// Stage-2 soft-knee saturator (enhanced path only): exact identity
/// through the hardware rail, then a tanh knee compressing the over-rail
/// region (the six-channel sum can reach 3x the rail) into the headroom
/// above it, asymptoting at full scale after OUT_GAIN. C1 at the knee.
/// Faithful mode keeps the hard rail — clipping is sometimes the
/// intended texture (the research note's "GBA speaker" mono examples).
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= RAIL {
        return x;
    }
    let h = 1.0 / OUT_GAIN - RAIL; // headroom above the rail
    (RAIL + h * ((a - RAIL) / h).tanh()).copysign(x)
}

/// Polyphase interpolation table: (SINC_PHASES + 1) rows of SINC_TAPS
/// coefficients, Hann-windowed sinc at cutoff `fc` (cycles per source
/// sample), each row normalized to unity DC gain.
fn build_sinc_table(fc: f64) -> Vec<f32> {
    use std::f64::consts::PI;
    let center = (SINC_TAPS / 2 - 1) as f64;
    let half = SINC_TAPS as f64 / 2.0;
    let mut table = Vec::with_capacity((SINC_PHASES + 1) * SINC_TAPS);
    for p in 0..=SINC_PHASES {
        let phi = p as f64 / SINC_PHASES as f64;
        let mut row = [0.0f64; SINC_TAPS];
        let mut sum = 0.0;
        for (k, c) in row.iter_mut().enumerate() {
            let t = k as f64 - center - phi;
            let sinc = if t.abs() < 1e-9 {
                2.0 * fc
            } else {
                (2.0 * PI * fc * t).sin() / (PI * t)
            };
            let win = if t.abs() < half {
                0.5 * (1.0 + (PI * t / half).cos())
            } else {
                0.0
            };
            *c = sinc * win;
            sum += *c;
        }
        table.extend(row.iter().map(|c| (c / sum) as f32));
    }
    table
}

/// One-pole DC blocker, fc ~ 10 Hz: y[n] = x[n] - x[n-1] + r*y[n-1].
/// Kills bias offsets, held-DAC ledges, and underrun pops.
struct DcBlock {
    r: f32,
    x1: f32,
    y1: f32,
}

impl DcBlock {
    fn new(out_hz: f64) -> DcBlock {
        DcBlock {
            r: 1.0 - (2.0 * std::f64::consts::PI * 10.0 / out_hz) as f32,
            x1: 0.0,
            y1: 0.0,
        }
    }

    fn next(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Fixed-ratio polyphase windowed-sinc resampler over an interleaved
/// stereo queue, with gentle (max ±2%) consumption-rate control against
/// the queue fill. Band-limits to the narrower of the two Nyquists, so
/// downsampling to a 44.1/48 kHz device does not fold images the way
/// the zero-order-hold path does. Used for the PSG grid stream.
struct SincResampler {
    table: Vec<f32>,
    fast_tables: [Vec<f32>; 3],
    /// Last SINC_TAPS source samples per side, oldest first.
    hist: [[f32; SINC_TAPS]; 2],
    ratio: f64,
    frac: f64,
}

impl SincResampler {
    fn new(src_hz: f64, out_hz: f64) -> SincResampler {
        // 10% under the narrower Nyquist leaves the window transition
        // band room.
        SincResampler {
            table: build_sinc_table(0.45 * (out_hz / src_hz).min(1.0)),
            fast_tables: [2.0, 4.0, 8.0]
                .map(|speed| build_sinc_table(0.45 * (out_hz / (src_hz * speed)).min(1.0))),
            hist: [[0.0; SINC_TAPS]; 2],
            ratio: src_hz / out_hz,
            frac: 0.0,
        }
    }

    fn next(&mut self, q: &mut std::collections::VecDeque<f32>) -> (f32, f32) {
        self.next_with_speed(q, 1.0)
    }

    fn next_with_speed(
        &mut self,
        q: &mut std::collections::VecDeque<f32>,
        speed: f64,
    ) -> (f32, f32) {
        // Steer consumption toward the target fill; an underrun repeats
        // the newest pair (a one-sample ZOH the DC blocker smooths).
        let pairs = (q.len() / 2) as f64;
        let target = RING_TARGET as f64 * speed;
        let err = (pairs - target) / target;
        self.frac += self.ratio * speed * (1.0 + 0.02 * err.clamp(-1.0, 1.0));
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            let (l, r) = match (q.pop_front(), q.pop_front()) {
                (Some(l), Some(r)) => (l, r),
                _ => (self.hist[0][SINC_TAPS - 1], self.hist[1][SINC_TAPS - 1]),
            };
            for (h, s) in self.hist.iter_mut().zip([l, r]) {
                h.copy_within(1.., 0);
                h[SINC_TAPS - 1] = s;
            }
        }
        let table = match speed as u32 {
            2 => &self.fast_tables[0],
            4 => &self.fast_tables[1],
            8 => &self.fast_tables[2],
            _ => &self.table,
        };
        let row = &table[(self.frac * SINC_PHASES as f64) as usize * SINC_TAPS..][..SINC_TAPS];
        (
            row.iter()
                .zip(self.hist[0].iter())
                .map(|(c, s)| c * s)
                .sum(),
            row.iter()
                .zip(self.hist[1].iter())
                .map(|(c, s)| c * s)
                .sum(),
        )
    }
}

/// Renders one Direct Sound channel as band-limited steps (the blip-buf
/// idea): each DAC transition deposits a windowed-sinc impulse — the
/// step's derivative — into a small future ring at its exact fractional
/// output position, and a leaky integrator reconstructs the waveform.
/// Edges land jitter-free at the channel's own timer rate (which may
/// change at any event), and crucially the DAC staircase spectrum — the
/// ZOH images above the mix rate's Nyquist — survives up to the device
/// Nyquist. That brightness is part of the hardware's canon sound;
/// smooth sample interpolation removes it and audibly dulls transients
/// (reference-trace comparison on a square-timbre chime). Channels
/// driven above the device rate degrade gracefully: overlapping
/// deposits sum to a proper low-pass.
struct FifoInterp {
    /// Windowed-sinc impulse table (rows sum to 1 = unit step).
    table: Vec<f32>,
    /// Future deposits, ring-indexed by output sample.
    fut: [f32; SINC_TAPS * 2],
    pos: usize,
    /// Leaky integrator state. The tiny leak only bounds drift; real
    /// output coupling is the shared DC blocker downstream.
    y: f32,
    /// Current DAC level, for edge deltas.
    level: f32,
    /// Master cycles until the next DAC event is due.
    t_next: f64,
    /// Current source sample spacing in master cycles.
    period: f64,
    /// Master cycles per output sample.
    out_step: f64,
}

impl FifoInterp {
    fn next_with_speed(
        &mut self,
        q: &mut std::collections::VecDeque<(i8, u32)>,
        speed: f64,
    ) -> f32 {
        let base = self.out_step;
        self.out_step = base * speed;
        let sample = self.next(q);
        self.out_step = base;
        sample
    }
    fn new(out_hz: f64) -> FifoInterp {
        FifoInterp {
            table: build_sinc_table(0.45),
            fut: [0.0; SINC_TAPS * 2],
            pos: 0,
            y: 0.0,
            level: 0.0,
            t_next: 512.0,
            period: 512.0, // placeholder until the first DAC event
            out_step: (1u64 << 24) as f64 / out_hz,
        }
    }

    fn next(&mut self, q: &mut std::collections::VecDeque<(i8, u32)>) -> f32 {
        // Steer toward ~62 ms queued at the channel's current rate.
        let target = ((1u64 << 24) as f64 / self.period / 16.0).max(64.0);
        let err = (q.len() as f64 - target) / target;
        let stepc = self.out_step * (1.0 + 0.02 * err.clamp(-1.0, 1.0));

        self.t_next -= stepc;
        while self.t_next <= 0.0 {
            // The event fires inside this output sample, at fraction phi
            // of the sample period.
            let phi = (1.0 + self.t_next / stepc).clamp(0.0, 1.0);
            let new_level = match q.pop_front() {
                Some((s, p)) => {
                    self.period = (p as f64).max(1.0);
                    s as f32 * (64.0 / 32768.0)
                }
                None => self.level, // DAC holds on underrun
            };
            let delta = new_level - self.level;
            self.level = new_level;
            if delta != 0.0 {
                let row =
                    &self.table[(phi * SINC_PHASES as f64) as usize * SINC_TAPS..][..SINC_TAPS];
                for (k, c) in row.iter().enumerate() {
                    self.fut[(self.pos + k) % (SINC_TAPS * 2)] += delta * c;
                }
            }
            self.t_next += self.period;
        }

        self.y = self.y * 0.9999 + self.fut[self.pos];
        self.fut[self.pos] = 0.0;
        self.pos = (self.pos + 1) % (SINC_TAPS * 2);
        self.y
    }
}

pub fn arm_audio_hle(m: &mut Machine) -> String {
    let pin: Option<&str> = None;
    match pin {
        Some("off") => return "engine HLE pinned off by package".into(),
        Some("m4a") => {
            let sigs = gba_core::mp2k::detect(&m.bus.rom);
            if sigs.is_empty() {
                return "DEGRADED: package pins engine-hle = m4a but no driver signature found \
                        — per-channel enhancement active"
                    .into();
            }
            m.bus.mp2k = Some(Box::new(gba_core::mp2k::Mp2kHle::new(&sigs)));
            return "M4A/MP2K — HLE shadow armed (pinned by package)".into();
        }
        Some("gax") => {
            // v1 detects statically; the banner-era shadow self-validates
            // against the 'GAX3' work-block magic at runtime, so arming it
            // blind under a pin is safe — it simply never engages on a
            // mismatch.
            if let Some(sig) = gba_core::gax::detect_v1(&m.bus.rom) {
                m.bus.gax = Some(Box::new(gba_core::gax::GaxHle::new(sig)));
            } else {
                m.bus.gax = Some(Box::new(gba_core::gax::GaxHle::new_v3()));
            }
            return "GAX — HLE shadow armed (pinned by package)".into();
        }
        Some("rdrv") => {
            if let Some(sig) = gba_core::rdrv::detect(&m.bus.rom) {
                m.bus.rdrv = Some(Box::new(gba_core::rdrv::RdrvHle::new(sig)));
                return "in-house (R) — HLE shadow armed (pinned by package)".into();
            }
            return "DEGRADED: package pins engine-hle = rdrv but no driver found — \
                    per-channel enhancement active"
                .into();
        }
        Some(other) => {
            eprintln!("DEGRADED: unknown engine-hle pin {other:?} — auto-detecting");
        }
        None => {}
    }
    match gba_core::engine::classify(&m.bus.rom) {
        gba_core::engine::Engine::M4a(sigs) => {
            let desc = format!(
                "M4A/MP2K — HLE shadow armed (SoundMainRAM {})",
                sigs.iter()
                    .map(|s| format!("{:#010x}", s.sound_main_ram & !1))
                    .collect::<Vec<_>>()
                    .join("/")
            );
            m.bus.mp2k = Some(Box::new(gba_core::mp2k::Mp2kHle::new(&sigs)));
            desc
        }
        gba_core::engine::Engine::Gax(ver) => {
            if let Some(sig) = gba_core::gax::detect_v1(&m.bus.rom) {
                let desc = "GAX v1 lineage — HLE shadow armed (state block located at runtime)"
                    .to_string();
                m.bus.gax = Some(Box::new(gba_core::gax::GaxHle::new(sig)));
                desc
            } else if ver.is_some() {
                // Banner era (v2/v3): the work block self-identifies at
                // runtime ('GAX3' magic + structural validation).
                m.bus.gax = Some(Box::new(gba_core::gax::GaxHle::new_v3()));
                format!(
                    "GAX {} — HLE shadow armed (work block located at runtime)",
                    ver.as_deref().unwrap_or("?")
                )
            } else {
                "GAX (early, unrecognized revision) — per-channel enhancement active".into()
            }
        }
        gba_core::engine::Engine::Rdiag => {
            if let Some(sig) = gba_core::rdrv::detect(&m.bus.rom) {
                m.bus.rdrv = Some(Box::new(gba_core::rdrv::RdrvHle::new(sig)));
                "in-house (R) — HLE shadow armed (mixer blob located at runtime)".into()
            } else {
                "in-house (R, unrecognized revision) — per-channel enhancement active".into()
            }
        }
        other => format!(
            "{} — per-channel enhancement active (engine HLE not yet available)",
            other.describe()
        ),
    }
}

struct Enhanced {
    psg: SincResampler,
    fifo: [FifoInterp; 2],
    /// MP2K HLE stream resampler, crossfaded against `fifo` as the shadow mixer proves/pauses.
    hle: SincResampler,
    xfade: f32,
    xfade_k: f32,
    dc: [DcBlock; 2],
}

/// "normal" fast-forward audio: at most this many seconds wait to be played.
const NORMAL_BACKLOG_PAIRS: usize = 48_000 * 4;

pub struct Mixer {
    mixed: VecDeque<i16>,
    psg: VecDeque<f32>,
    fifo: [VecDeque<(i8, u32)>; 2],
    hle: VecDeque<f32>,
    hle_on: bool,
    route: [u8; 2],
    vol_half: [bool; 2],
    /// The menu's Enhanced Audio.
    pub enhanced_on: bool,
    /// Fraction removed from full volume (0 = full, 1 = silent).
    pub vol_cut: f32,
    muted: bool,
    playback_speed: f64,
    normal: Option<VecDeque<(i16, i16)>>,
    ff: (bool, String),
    eng: Enhanced,
    mode_x: f32,
    mode_k: f32,
    step: f64,
    frac: f64,
    last: (i16, i16),
    faithful_dc: [DcBlock; 2],
    vol_cur: f32,
    vol_k: f32,
    out_acc: f64,
}

impl Mixer {
    pub fn new(rate: f64, enhanced_on: bool, vol_cut: f32) -> Mixer {
        let src = gba_core::mem::AUDIO_RATE_HZ as f64;
        Mixer {
            mixed: VecDeque::new(),
            psg: VecDeque::new(),
            fifo: [VecDeque::new(), VecDeque::new()],
            hle: VecDeque::new(),
            hle_on: false,
            route: [0; 2],
            vol_half: [false; 2],
            enhanced_on,
            vol_cut,
            muted: false,
            playback_speed: 1.0,
            normal: None,
            ff: (false, String::new()),
            eng: Enhanced {
                psg: SincResampler::new(src, rate),
                fifo: [FifoInterp::new(rate), FifoInterp::new(rate)],
                hle: SincResampler::new(src, rate),
                xfade: 0.0,
                xfade_k: (1.0 / (0.040 * rate)) as f32,
                dc: [DcBlock::new(rate), DcBlock::new(rate)],
            },
            mode_x: if enhanced_on { 1.0 } else { 0.0 },
            mode_k: (1.0 / (0.040 * rate)) as f32,
            step: src / rate,
            frac: 0.0,
            last: (0, 0),
            faithful_dc: [DcBlock::new(rate), DcBlock::new(rate)],
            vol_cur: 1.0 - vol_cut,
            vol_k: (1.0 / (0.020 * rate)) as f32,
            out_acc: 0.0,
        }
    }

    fn clear_rings(&mut self) {
        self.mixed.clear();
        self.psg.clear();
        self.hle.clear();
        self.fifo[0].clear();
        self.fifo[1].clear();
    }

    /// `set_fast_forward` of the play loop: called when fast forward or its audio mode changes.
    pub fn set_fast_forward(&mut self, on: bool, mode: &str, speed: u32) {
        if self.ff == (on, mode.to_string()) {
            return;
        }
        self.ff = (on, mode.to_string());
        self.muted = on && mode == "silent";
        self.playback_speed = if on && mode == "fast" { speed as f64 } else { 1.0 };
        self.normal = (on && mode == "normal").then(VecDeque::new);
        self.clear_rings();
    }

    /// `handoff_audio` of the play loop: this frame's audio taps go to the rings.
    pub fn feed(&mut self, bus: &mut MemMap) {
        let ff_silent = self.muted;
        if ff_silent {
            bus.audio_buf.clear();
            bus.psg_tap.clear();
            bus.hle_tap.clear();
            bus.fifo_tap[0].clear();
            bus.fifo_tap[1].clear();
            return;
        }
        if let Some(queue) = self.normal.as_mut() {
            for pair in bus.audio_buf.chunks_exact(2) {
                queue.push_back((pair[0], pair[1]));
            }
            while queue.len() > NORMAL_BACKLOG_PAIRS {
                queue.pop_front();
            }
            bus.audio_buf.clear();
            bus.psg_tap.clear();
            bus.hle_tap.clear();
            bus.fifo_tap[0].clear();
            bus.fifo_tap[1].clear();
            return;
        }
        let cnt_h = bus.read16(0x0400_0082);
        let audio_capacity = 65536 * self.playback_speed.max(1.0) as usize;
        self.route = [(cnt_h >> 8 & 3) as u8, (cnt_h >> 12 & 3) as u8];
        self.vol_half = [cnt_h & 4 == 0, cnt_h & 8 == 0];
        for pair in bus.audio_buf.chunks_exact(2) {
            if self.mixed.len() < audio_capacity {
                self.mixed.push_back(pair[0]);
                self.mixed.push_back(pair[1]);
            }
        }
        bus.audio_buf.clear();
        for pair in bus.psg_tap.chunks_exact(2) {
            if self.psg.len() < audio_capacity {
                self.psg.push_back(pair[0] as f32 / 32768.0);
                self.psg.push_back(pair[1] as f32 / 32768.0);
            }
        }
        bus.psg_tap.clear();
        let live = bus.mp2k.as_deref().is_some_and(|h| h.live())
            || bus.gax.as_deref().is_some_and(|g| g.live())
            || bus.rdrv.as_deref().is_some_and(|r| r.live());
        // the pauses of the engine HLE are diagnostics on PC; here they only have to be consumed
        if let Some(h) = bus.mp2k.as_deref_mut() {
            h.vf.reverted.take();
        } else if let Some(g) = bus.gax.as_deref_mut() {
            g.vf.reverted.take();
        } else if let Some(r) = bus.rdrv.as_deref_mut() {
            r.vf.reverted.take();
        }
        if live {
            for pair in bus.hle_tap.chunks_exact(2) {
                if self.hle.len() < audio_capacity {
                    self.hle.push_back(pair[0]);
                    self.hle.push_back(pair[1]);
                }
            }
        } else {
            self.hle.clear();
        }
        bus.hle_tap.clear();
        if !live {
            self.hle_on = false;
        } else if self.hle.len() / 2 >= RING_TARGET / 2 {
            self.hle_on = true;
        }
        for f in 0..2 {
            let events = std::mem::take(&mut bus.fifo_tap[f]);
            for e in events {
                if self.fifo[f].len() < audio_capacity / 2 {
                    self.fifo[f].push_back(e);
                }
            }
        }
    }

    /// The output of one frame of the screen (about 803 sample pairs at 48 kHz), 16-bit stereo, into `out`.
    pub fn render(&mut self, out: &mut Vec<i16>) {
        self.out_acc += 48_000.0 / (16_777_216.0 / 280_896.0);
        let frames = self.out_acc as usize;
        self.out_acc -= frames as f64;
        let playback_speed = self.playback_speed.max(1.0);
        let normal_audio = self.normal.is_some();
        for _ in 0..frames {
            // Enhanced output (always computed so the ring stays drained and the path is warm for a live switch).
            let (el, er) = {
                let e = &mut self.eng;
                let (pl, pr) = e.psg.next_with_speed(&mut self.psg, playback_speed);
                let mut l = pl;
                let mut r = pr;
                let a = e.fifo[0].next_with_speed(&mut self.fifo[0], playback_speed) * if self.vol_half[0] { 0.5 } else { 1.0 };
                let b = e.fifo[1].next_with_speed(&mut self.fifo[1], playback_speed) * if self.vol_half[1] { 0.5 } else { 1.0 };
                let (hl, hr) = e.hle.next_with_speed(&mut self.hle, playback_speed);
                let hr = hr * if self.vol_half[0] { 0.5 } else { 1.0 };
                let hl = hl * if self.vol_half[1] { 0.5 } else { 1.0 };
                let target = if self.hle_on { 1.0 } else { 0.0 };
                e.xfade += (target - e.xfade) * e.xfade_k;
                let xf = e.xfade;
                let mut fl = 0.0;
                let mut fr = 0.0;
                if self.route[0] & 2 != 0 {
                    fl += a;
                }
                if self.route[0] & 1 != 0 {
                    fr += a;
                }
                if self.route[1] & 2 != 0 {
                    fl += b;
                }
                if self.route[1] & 1 != 0 {
                    fr += b;
                }
                let mut gl = 0.0;
                let mut gr = 0.0;
                if self.route[0] & 2 != 0 || self.route[1] & 2 != 0 {
                    gl = hl;
                }
                if self.route[0] & 1 != 0 || self.route[1] & 1 != 0 {
                    gr = hr;
                }
                l += fl * (1.0 - xf) + gl * xf;
                r += fr * (1.0 - xf) + gr * xf;
                (e.dc[0].next(soft_clip(l)), e.dc[1].next(soft_clip(r)))
            };
            // Faithful output (also always computed, also drained).
            let (fl, fr) = {
                let pairs = (self.mixed.len() / 2) as f64;
                let target_pairs = RING_TARGET as f64 * playback_speed;
                let err = if normal_audio { 0.0 } else { (pairs - target_pairs) / target_pairs };
                self.frac += self.step * playback_speed * (1.0 + 0.02 * err.clamp(-1.0, 1.0));
                while self.frac >= 1.0 {
                    self.frac -= 1.0;
                    match self.normal.as_mut() {
                        Some(queue) => {
                            if let Some(pair) = queue.pop_front() {
                                self.last = pair;
                            }
                        }
                        None => {
                            if let (Some(l), Some(r)) = (self.mixed.pop_front(), self.mixed.pop_front()) {
                                self.last = (l, r);
                            }
                        }
                    }
                }
                (
                    self.faithful_dc[0].next(self.last.0 as f32 / 32768.0),
                    self.faithful_dc[1].next(self.last.1 as f32 / 32768.0),
                )
            };
            let target = if self.enhanced_on && !normal_audio { 1.0 } else { 0.0 };
            self.mode_x += (target - self.mode_x) * self.mode_k;
            let l = fl + (el - fl) * self.mode_x;
            let r = fr + (er - fr) * self.mode_x;
            let vol_target = if self.muted { 0.0 } else { 1.0 - self.vol_cut };
            self.vol_cur += (vol_target - self.vol_cur) * self.vol_k;
            let l = (l * OUT_GAIN * self.vol_cur).clamp(-1.0, 1.0);
            let r = (r * OUT_GAIN * self.vol_cur).clamp(-1.0, 1.0);
            out.push((l * 32767.0) as i16);
            out.push((r * 32767.0) as i16);
        }
    }
}
