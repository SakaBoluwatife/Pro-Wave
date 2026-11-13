//! Spatial audio: tracks as objects in a 3D space around the listener, mixed into one 7.1.4 bus, which
//! then goes to speakers of any layout from 5.1 up, or to headphones through a binaural render.
//!
//! Tracks that are not objects stay on Stereo Out and are the bed: its left and right. With spatial
//! audio off nothing here runs, and the mix is the plain stereo one.
//!
//! The binaural render models a head rather than using measured responses, so it needs no data set and
//! costs little: the delay between the ears (Woodworth's formula) and the shadow of the head on the
//! far ear (the one-pole one-zero filter of Brown and Duda, "A structural model for binaural sound
//! synthesis", 1998), with a duller rear and a brighter top for the cues a head alone does not give.

use std::f32::consts::PI;

/// The channels of the bus: left, right, centre, LFE, rear left, rear right, side left, side right, top
/// front left, top front right, top rear left, top rear right (the order Windows has for 7.1.4), then
/// wide left, wide right, top middle left and top middle right for 9.1.6 and 5.1.2. A layout uses
/// some of them, and [`SpeakerLayout::device_order`] says which output each goes to.
pub const SPATIAL_CHANNELS: usize = 16;
/// The LFE channel, which is not a place in the room.
pub const LFE: usize = 3;
/// Names of the channels, for meters and files.
pub const CHANNEL_NAMES: [&str; SPATIAL_CHANNELS] = [
    "L", "R", "C", "LFE", "Lrs", "Rrs", "Lss", "Rss", "Ltf", "Rtf", "Ltr", "Rtr", "Lw", "Rw",
    "Ltm", "Rtm",
];
/// The frames the bus holds: a span of the engine.
const BUS_FRAMES: usize = crate::plugin::PLUGIN_BLOCK;
/// The speed of sound (m/s) and the radius of a head (m).
const SOUND: f32 = 343.0;
const HEAD: f32 = 0.0875;
/// The most frames an ear can lag: enough for the far side of a head at 192 kHz.
const DELAY_FRAMES: usize = 256;

/// A speaker layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SpeakerLayout {
    Stereo,
    Surround51,
    Surround512,
    Surround514,
    Surround71,
    Surround712,
    #[default]
    Surround714,
    Surround916,
}

impl SpeakerLayout {
    pub const ALL: [SpeakerLayout; 8] = [
        SpeakerLayout::Stereo,
        SpeakerLayout::Surround51,
        SpeakerLayout::Surround512,
        SpeakerLayout::Surround514,
        SpeakerLayout::Surround71,
        SpeakerLayout::Surround712,
        SpeakerLayout::Surround714,
        SpeakerLayout::Surround916,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Stereo => "Stereo",
            Self::Surround51 => "5.1",
            Self::Surround512 => "5.1.2",
            Self::Surround514 => "5.1.4",
            Self::Surround71 => "7.1",
            Self::Surround712 => "7.1.2",
            Self::Surround714 => "7.1.4",
            Self::Surround916 => "9.1.6",
        }
    }

    /// The bus channels this layout uses, in the order of the device's outputs: the first goes to
    /// output 1, and so on. The LFE is in it (it is not a place, so [`Self::place`] leaves it out).
    pub fn device_order(self) -> &'static [usize] {
        match self {
            Self::Stereo => &[0, 1],
            Self::Surround51 => &[0, 1, 2, 3, 4, 5],
            Self::Surround512 => &[0, 1, 2, 3, 4, 5, 14, 15],
            Self::Surround514 => &[0, 1, 2, 3, 4, 5, 8, 9, 10, 11],
            Self::Surround71 => &[0, 1, 2, 3, 4, 5, 6, 7],
            Self::Surround712 => &[0, 1, 2, 3, 4, 5, 6, 7, 14, 15],
            Self::Surround714 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Self::Surround916 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        }
    }

    /// Whether this layout has bus channel `channel`.
    pub fn has_channel(self, channel: usize) -> bool {
        self.device_order().contains(&channel)
    }

    /// The outputs a device needs for this layout.
    pub fn channels(self) -> usize {
        self.device_order().len()
    }

    /// Where a channel's speaker is: azimuth in degrees (0 ahead, right positive) and height in
    /// degrees. None for what this layout does not have, and for the LFE.
    pub fn place(self, channel: usize) -> Option<(f32, f32)> {
        let five = matches!(self, Self::Surround51 | Self::Surround512 | Self::Surround514);
        let rear = if five { 110.0 } else { 135.0 };
        if channel == LFE || !self.has_channel(channel) {
            return None;
        }
        Some(match channel {
            0 => (-30.0, 0.0),
            1 => (30.0, 0.0),
            2 => (0.0, 0.0),
            4 => (-rear, 0.0),
            5 => (rear, 0.0),
            6 => (-90.0, 0.0),
            7 => (90.0, 0.0),
            8 => (-45.0, 45.0),
            9 => (45.0, 45.0),
            10 => (-135.0, 45.0),
            11 => (135.0, 45.0),
            12 => (-60.0, 0.0),
            13 => (60.0, 0.0),
            14 => (-90.0, 45.0),
            15 => (90.0, 45.0),
            _ => return None,
        })
    }
}

/// How an object's direction becomes speaker gains.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SpatialLaw {
    /// Vector Base Amplitude Panning: the two or three speakers around the object, sharp and exact.
    #[default]
    Vbap,
    /// Every speaker by how close it is to the object: softer. The law of projects made before VBAP.
    Smooth,
}

impl SpatialLaw {
    pub const ALL: [SpatialLaw; 2] = [Self::Vbap, Self::Smooth];

    pub fn name(self) -> &'static str {
        match self {
            Self::Vbap => "VBAP",
            Self::Smooth => "Smooth",
        }
    }
}

/// What the bus goes to. Each but `Off` carries the bed format: the speakers objects are aimed at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SpatialMode {
    /// None of this runs.
    #[default]
    Off,
    /// Any pair of headphones, through a modelled head.
    Binaural(SpeakerLayout),
    /// A device with the outputs for it. With too few, it folds down to stereo.
    Speakers(SpeakerLayout),
    /// Plain stereo speakers, folded down: how the mix sounds to someone without a spatial setup.
    Folded(SpeakerLayout),
    /// Speakers of a smaller layout (the second): the bed downmixed to them, as a listener with that
    /// system hears it.
    Downmix(SpeakerLayout, SpeakerLayout),
}

impl SpatialMode {
    /// The bed format, if spatial audio is on.
    pub fn layout(self) -> Option<SpeakerLayout> {
        match self {
            Self::Off => None,
            Self::Binaural(l) | Self::Speakers(l) | Self::Folded(l) | Self::Downmix(l, _) => Some(l),
        }
    }

    /// The speakers the sound goes out to: the bed's, or the layout downmixed to.
    pub fn output_layout(self) -> Option<SpeakerLayout> {
        match self {
            Self::Downmix(_, to) => Some(to),
            other => other.layout(),
        }
    }
}

/// How far a channel sounds in the headphone render.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Distance {
    /// Not through the head: plainly on its side, as on a stereo pair.
    Off,
    /// Close, with no room.
    Near,
    /// With the room.
    #[default]
    Mid,
    /// Further away: more room, a little quieter.
    Far,
}

impl Distance {
    pub const ALL: [Distance; 4] = [Self::Off, Self::Near, Self::Mid, Self::Far];

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Near => "Near",
            Self::Mid => "Mid",
            Self::Far => "Far",
        }
    }

    /// The direct sound's gain and how much goes to the room.
    fn amounts(self) -> (f32, f32) {
        match self {
            Self::Off | Self::Near => (1.0, 0.0),
            Self::Mid => (1.0, 1.0),
            Self::Far => (0.8, 1.8),
        }
    }
}

/// What the renderer does to each channel: muted, how far it sounds, and its overall level.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ChannelSetup {
    pub muted: [bool; SPATIAL_CHANNELS],
    pub distance: [Distance; SPATIAL_CHANNELS],
    /// The renderer's volume, as a gain.
    pub gain: f32,
    /// The LFE channel's low-pass, in Hz; 0 leaves it as it is.
    pub lfe_hz: f32,
}

impl Default for ChannelSetup {
    fn default() -> Self {
        let mut distance = [Distance::Mid; SPATIAL_CHANNELS];
        distance[LFE] = Distance::Off;
        Self {
            muted: [false; SPATIAL_CHANNELS],
            distance,
            gain: 1.0,
            lfe_hz: 0.0,
        }
    }
}

/// The speakers of the room the mix is heard in: where each one really is (none: where the layout
/// says), and the level and delay that line them up at the listening place. It belongs to the room,
/// not to a project, and is used only when the bus goes straight out to speakers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpeakerSetup {
    pub place: [Option<(f32, f32)>; SPATIAL_CHANNELS],
    pub gain_db: [f32; SPATIAL_CHANNELS],
    pub delay_ms: [f32; SPATIAL_CHANNELS],
}

impl Default for SpeakerSetup {
    fn default() -> Self {
        Self {
            place: [None; SPATIAL_CHANNELS],
            gain_db: [0.0; SPATIAL_CHANNELS],
            delay_ms: [0.0; SPATIAL_CHANNELS],
        }
    }
}

/// A measured head (from a SOFA file, read by the app): for each virtual speaker of a layout, the
/// impulse response to each ear at the engine's rate, and each ear's delay in frames.
#[derive(Clone, Debug, Default)]
pub struct HrtfSet {
    pub speakers: Vec<HrtfSpeaker>,
}

#[derive(Clone, Debug)]
pub struct HrtfSpeaker {
    /// The bus channel it plays.
    pub channel: usize,
    /// Where it is, for when its distance is Off (plainly on its side).
    pub azimuth: f32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub delay_left: usize,
    pub delay_right: usize,
}

/// The longest impulse response used, in frames, and the longest ear delay.
pub const MAX_HRIR: usize = 512;
const MAX_HRIR_DELAY: usize = 256;

/// A virtual speaker through a measured head: its responses back to front (so each output is one
/// dot product over a run of the history) and the history of what it played.
struct HrtfVoice {
    channel: usize,
    azimuth: f32,
    rev_left: Box<[f32]>,
    rev_right: Box<[f32]>,
    delay_left: usize,
    delay_right: usize,
    /// `HISTORY` frames of the past, then room for a span.
    history: Box<[f32]>,
}

const HISTORY: usize = MAX_HRIR + MAX_HRIR_DELAY;

/// A dot product in eight running sums, so the compiler can use vector registers for it.
#[inline]
fn dot8(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut acc = [0f32; 8];
    let whole = n / 8 * 8;
    for (x, y) in a[..whole].chunks_exact(8).zip(b[..whole].chunks_exact(8)) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut sum = ((acc[0] + acc[1]) + (acc[2] + acc[3])) + ((acc[4] + acc[5]) + (acc[6] + acc[7]));
    for k in whole..n {
        sum += a[k] * b[k];
    }
    sum
}

/// The headphone render through a measured head: each virtual speaker convolved with its two
/// responses. Made on the UI thread; allocates nothing as it runs.
struct HrtfRender {
    voices: Vec<HrtfVoice>,
}

impl HrtfRender {
    fn new(set: &HrtfSet) -> Self {
        let voices = set
            .speakers
            .iter()
            .filter(|s| s.channel < SPATIAL_CHANNELS && s.channel != LFE)
            .map(|s| {
                let rev = |ir: &[f32]| -> Box<[f32]> {
                    let n = ir.len().clamp(1, MAX_HRIR);
                    let mut v: Vec<f32> = ir[..n.min(ir.len())].to_vec();
                    v.resize(n, 0.0);
                    v.reverse();
                    v.into_boxed_slice()
                };
                HrtfVoice {
                    channel: s.channel,
                    azimuth: s.azimuth,
                    rev_left: rev(&s.left),
                    rev_right: rev(&s.right),
                    delay_left: s.delay_left.min(MAX_HRIR_DELAY),
                    delay_right: s.delay_right.min(MAX_HRIR_DELAY),
                    history: vec![0.0; HISTORY + BUS_FRAMES].into_boxed_slice(),
                }
            })
            .collect();
        Self { voices }
    }

    /// Adds the ears' sound for the frames of `bus` to `left` and `right`, each channel as its
    /// distance says (Off: plainly on its side; otherwise through the head at the direct level).
    fn render(
        &mut self,
        bus: &[[f64; BUS_FRAMES]; SPATIAL_CHANNELS],
        distance: &[Distance; SPATIAL_CHANNELS],
        left: &mut [f64],
        right: &mut [f64],
    ) {
        let n = left.len().min(BUS_FRAMES);
        for v in &mut self.voices {
            let input = &bus[v.channel];
            let how = distance[v.channel];
            if how == Distance::Off {
                let side = (v.azimuth / 90.0).clamp(-1.0, 1.0);
                let angle = (side + 1.0) * std::f32::consts::FRAC_PI_4;
                let (to_l, to_r) = (f64::from(angle.cos()), f64::from(angle.sin()));
                for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate().take(n) {
                    *l += input[j] * to_l;
                    *r += input[j] * to_r;
                }
                continue;
            }
            let direct = f64::from(how.amounts().0);
            for j in 0..n {
                v.history[HISTORY + j] = input[j] as f32;
            }
            let (taps_l, taps_r) = (v.rev_left.len(), v.rev_right.len());
            for j in 0..n {
                let newest = HISTORY + j;
                let from_l = newest + 1 - taps_l - v.delay_left;
                let from_r = newest + 1 - taps_r - v.delay_right;
                let a = dot8(&v.history[from_l..from_l + taps_l], &v.rev_left);
                let b = dot8(&v.history[from_r..from_r + taps_r], &v.rev_right);
                left[j] += f64::from(a) * direct;
                right[j] += f64::from(b) * direct;
            }
            v.history.copy_within(n..n + HISTORY, 0);
        }
        // The LFE to both ears, as it has no place.
        for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate().take(n) {
            *l += bus[LFE][j];
            *r += bus[LFE][j];
        }
    }
}

/// The most spatial buses a project has.
pub const MAX_SPATIAL_BUSES: usize = 8;

/// Where a spatial bus puts its channels in the room, as a surround panner on a multichannel bus does:
/// each channel of its layout is a point, turned by `direction`, raised by `elevation`, its angles
/// scaled by `spread`, and widened by `size`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BusPlace {
    /// Degrees the whole layout is turned, right positive.
    pub direction: f32,
    /// Degrees it is raised (or lowered).
    pub elevation: f32,
    /// 0..=1: 1 keeps the layout's own angles, 0 brings every channel to `direction`.
    pub spread: f32,
    /// 0..=1: how much each channel spreads over the speakers around it.
    pub size: f32,
    /// 0..=1: the level of its centre channel, and of its LFE.
    pub centre: f32,
    pub lfe: f32,
    /// Its level, as a gain.
    pub gain: f32,
    pub muted: bool,
}

impl Default for BusPlace {
    fn default() -> Self {
        Self {
            direction: 0.0,
            elevation: 0.0,
            spread: 1.0,
            size: 0.0,
            centre: 1.0,
            lfe: 1.0,
            gain: 1.0,
            muted: false,
        }
    }
}

/// A spatial bus: what the objects routed to it sum to, in its own layout, and how that feeds the
/// main bus.
struct SpatialBus {
    layout: SpeakerLayout,
    places: [Option<(f32, f32)>; SPATIAL_CHANNELS],
    vbap: Option<Box<crate::vbap::Vbap>>,
    buf: Box<[[f64; BUS_FRAMES]; SPATIAL_CHANNELS]>,
    matrix: [[f32; SPATIAL_CHANNELS]; SPATIAL_CHANNELS],
    used: bool,
    /// Its effects, in order: each runs once per pair of its channels (front left and right,
    /// centre and LFE, the surrounds, the heights...), so the layout keeps its places.
    fx: Vec<Vec<crate::fx::Fx>>,
    /// Its channels in pairs, as its layout orders them; an odd one out goes alone.
    pairs: Vec<(usize, Option<usize>)>,
    /// Its plugins, after its effects: each one surround plugin in its layout, or linked stereo
    /// copies (one per pair) for a plugin that is stereo only.
    plugins: Box<Vec<BusPlugin>>,
    /// Lines it up with the slowest bus.
    delay: Box<DelayRing>,
}

/// The longest a spatial bus's plugins may delay it and still be lined up with the rest, in frames.
const BUS_DELAY_MAX: usize = 8192;

/// A delay for every channel of a bus, so the buses whose plugins are slower and the rest of the
/// mix stay in time (delay compensation). Made off the audio thread.
struct DelayRing {
    data: Vec<Vec<f32>>,
    pos: usize,
}

impl DelayRing {
    fn new() -> Self {
        Self {
            data: (0..SPATIAL_CHANNELS).map(|_| vec![0.0; BUS_DELAY_MAX]).collect(),
            pos: 0,
        }
    }

    /// Delays the first `n` frames of every channel of `buf` by `delay` frames.
    fn run(&mut self, buf: &mut [[f64; BUS_FRAMES]; SPATIAL_CHANNELS], n: usize, delay: usize) {
        let delay = delay.min(BUS_DELAY_MAX - 1);
        for j in 0..n {
            let at = (self.pos + j) % BUS_DELAY_MAX;
            let from = (at + BUS_DELAY_MAX - delay) % BUS_DELAY_MAX;
            for (c, ring) in self.data.iter_mut().enumerate() {
                ring[at] = buf[c][j] as f32;
                buf[c][j] = f64::from(ring[from]);
            }
        }
        self.pos = (self.pos + n) % BUS_DELAY_MAX;
    }
}

/// A plugin on a spatial bus. Made off the audio thread with its buffers, so it never allocates.
pub struct BusPlugin {
    /// One instance in the bus's layout (`native`), or one per pair of its channels.
    units: Vec<Box<dyn crate::plugin::PluginProcessor>>,
    native: bool,
    /// The bus channel of each of the native instance's channels, as it names its speakers.
    map: Option<Vec<usize>>,
    bypass: bool,
    /// Buffers for a block of every channel, in and out.
    inb: Vec<Vec<f32>>,
    outb: Vec<Vec<f32>>,
}

impl BusPlugin {
    /// `units`: one instance set up in the bus's layout (`native`), or one stereo copy per pair.
    pub fn new(units: Vec<Box<dyn crate::plugin::PluginProcessor>>, native: bool, bypass: bool) -> Self {
        let buffers = || (0..SPATIAL_CHANNELS).map(|_| vec![0.0f32; BUS_FRAMES]).collect::<Vec<_>>();
        let map = if native { units.first().and_then(|u| u.channel_map()) } else { None };
        Self {
            units,
            native,
            map,
            bypass,
            inb: buffers(),
            outb: buffers(),
        }
    }

    /// The delay it puts on the bus, in samples.
    pub fn latency_samples(&self) -> usize {
        self.units.iter().map(|u| u.latency_samples()).max().unwrap_or(0)
    }

    /// Runs the bus's channels (`order`, in its layout) through it, `n` frames.
    fn run(
        &mut self,
        buf: &mut [[f64; BUS_FRAMES]; SPATIAL_CHANNELS],
        order: &[usize],
        pairs: &[(usize, Option<usize>)],
        n: usize,
        transport: &crate::plugin::PluginTransport,
    ) {
        if self.bypass || self.units.is_empty() {
            return;
        }
        let block = crate::plugin::plugin_block().clamp(1, BUS_FRAMES);
        let mut start = 0;
        while start < n {
            let m = block.min(n - start);
            if self.native {
                // In the order the plugin names its speakers, where it says.
                let order: &[usize] = self.map.as_deref().unwrap_or(order);
                let ch = order.len().min(SPATIAL_CHANNELS);
                for (k, &c) in order.iter().take(ch).enumerate() {
                    for j in 0..m {
                        self.inb[k][j] = buf[c][start + j] as f32;
                    }
                }
                let inb = &self.inb;
                let ins: [&[f32]; SPATIAL_CHANNELS] = std::array::from_fn(|k| &inb[k][..m]);
                let mut it = self.outb.iter_mut();
                let mut outs: [&mut [f32]; SPATIAL_CHANNELS] = std::array::from_fn(|_| match it.next() {
                    Some(v) => &mut v[..m],
                    None => &mut [],
                });
                if let Some(unit) = self.units.first_mut() {
                    unit.process_multi(&ins[..ch], &mut outs[..ch], transport);
                }
                for (k, &c) in order.iter().take(ch).enumerate() {
                    for j in 0..m {
                        buf[c][start + j] = f64::from(self.outb[k][j]);
                    }
                }
            } else {
                for (unit, &(lc, rc)) in self.units.iter_mut().zip(pairs) {
                    let rc_or = rc.unwrap_or(lc);
                    for j in 0..m {
                        self.inb[0][j] = buf[lc][start + j] as f32;
                        self.inb[1][j] = buf[rc_or][start + j] as f32;
                    }
                    let (o0, rest) = self.outb.split_at_mut(1);
                    let (Some(o0), Some(o1)) = (o0.first_mut(), rest.first_mut()) else {
                        return;
                    };
                    unit.process(
                        [&self.inb[0][..m], &self.inb[1][..m]],
                        [&mut o0[..m], &mut o1[..m]],
                        &[],
                        transport,
                    );
                    for j in 0..m {
                        buf[lc][start + j] = f64::from(o0[j]);
                        if let Some(c) = rc {
                            buf[c][start + j] = f64::from(o1[j]);
                        }
                    }
                }
            }
            start += m;
        }
    }
}

/// The most a speaker can be delayed, in milliseconds.
pub const MAX_SPEAKER_DELAY_MS: f32 = 50.0;
/// Frames of a speaker's delay line: 50 ms at 192 kHz, and a little over.
const DELAY_RING: usize = 16_384;

/// How a layout's channels feed a smaller one: for each channel of `from`, its gain into each channel
/// of `to`. A channel `to` has goes straight across; any other is panned (by VBAP) to where it would
/// be among `to`'s speakers, power kept. The LFE goes only to an LFE.
pub fn downmix_matrix(from: SpeakerLayout, to: SpeakerLayout) -> [[f32; SPATIAL_CHANNELS]; SPATIAL_CHANNELS] {
    let mut m = [[0f32; SPATIAL_CHANNELS]; SPATIAL_CHANNELS];
    let vbap = crate::vbap::Vbap::new(to);
    for &c in from.device_order() {
        if to.has_channel(c) {
            m[c][c] = 1.0;
        } else if c == LFE {
            continue;
        } else if let Some((az, el)) = from.place(c) {
            m[c] = vbap.gains(az, el, 1.0);
        }
    }
    m
}

/// A second-order low-pass (Butterworth), for the LFE.
#[derive(Clone, Copy, Debug, Default)]
struct LowPass {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl LowPass {
    fn new(hz: f64, sample_rate: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * (hz / sample_rate).clamp(1e-5, 0.49);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - c) / 2.0 / a0,
            b1: (1.0 - c) / a0,
            b2: (1.0 - c) / 2.0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
            ..Self::default()
        }
    }

    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A place in the room for an object.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ObjectPlace {
    /// Degrees, 0 ahead, right positive, -180..=180.
    pub azimuth: f32,
    /// Degrees, 0 at the ear, 90 straight up.
    pub elevation: f32,
    /// How wide it is, 0 a point, 1 all around.
    pub size: f32,
    /// How much of it goes to the LFE as well, 0..=1.
    pub lfe: f32,
    /// How far apart its left and right are, 0 together, 1 turned 45 degrees each way.
    pub spread: f32,
    /// How much of it the centre speaker plays, 0..=1: 0 leaves the centre out.
    pub center: f32,
    /// How far it sounds in the headphone render, as Logic lets each object choose; `None` takes
    /// the setting of the channels it is panned to.
    pub distance: Option<Distance>,
    /// How far out it is, 0 at the listener to 1 at the walls (the speakers). Inside the room it
    /// spreads over more speakers, all of them at the centre.
    pub radius: f32,
    /// Plays on the one speaker nearest to where it is, not between speakers.
    pub snap: bool,
    /// How much of it is split to two phantom places 45 degrees either side, 0..=1.
    pub divergence: f32,
    /// The spatial bus it goes through, from 1 (see [`BusPlace`]); 0 is straight to the main bus.
    pub bus: u8,
}

impl Default for ObjectPlace {
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            elevation: 0.0,
            size: 0.0,
            lfe: 0.0,
            spread: 0.5,
            center: 1.0,
            distance: None,
            radius: 1.0,
            snap: false,
            divergence: 0.0,
            bus: 0,
        }
    }
}

/// What an automation lane of an object moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ObjectAxis {
    /// Degrees around the listener.
    Around,
    /// Degrees up.
    Height,
    Size,
    Lfe,
    /// The room as a box from -1 to 1: left to right, back to front, and the floor of the ears (0) to
    /// the ceiling (1). Another way of moving Around, Height and Distance.
    X,
    Y,
    Z,
    /// 0 at the listener, 1 at the walls.
    Distance,
    Divergence,
}

impl ObjectAxis {
    pub const ALL: [ObjectAxis; 9] = [
        Self::Around,
        Self::Height,
        Self::Size,
        Self::Lfe,
        Self::X,
        Self::Y,
        Self::Z,
        Self::Distance,
        Self::Divergence,
    ];

    /// The values it can have.
    pub fn range(self) -> (f32, f32) {
        match self {
            Self::Around => (-180.0, 180.0),
            Self::Height => (0.0, 90.0),
            Self::Size | Self::Lfe | Self::Z | Self::Distance | Self::Divergence => (0.0, 1.0),
            Self::X | Self::Y => (-1.0, 1.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Around => "Around",
            Self::Height => "Height",
            Self::Size => "Size",
            Self::Lfe => "LFE",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::Distance => "Distance",
            Self::Divergence => "Divergence",
        }
    }
}

impl ObjectPlace {
    pub fn get(self, axis: ObjectAxis) -> f32 {
        match axis {
            ObjectAxis::Around => self.azimuth,
            ObjectAxis::Height => self.elevation,
            ObjectAxis::Size => self.size,
            ObjectAxis::Lfe => self.lfe,
            ObjectAxis::X => self.xyz()[0],
            ObjectAxis::Y => self.xyz()[1],
            ObjectAxis::Z => self.xyz()[2],
            ObjectAxis::Distance => self.radius,
            ObjectAxis::Divergence => self.divergence,
        }
    }

    pub fn set(&mut self, axis: ObjectAxis, value: f32) {
        let (low, high) = axis.range();
        let value = if value.is_finite() { value.clamp(low, high) } else { low.max(0.0) };
        match axis {
            ObjectAxis::Around => self.azimuth = value,
            ObjectAxis::Height => self.elevation = value,
            ObjectAxis::Size => self.size = value,
            ObjectAxis::Lfe => self.lfe = value,
            ObjectAxis::Distance => self.radius = value,
            ObjectAxis::Divergence => self.divergence = value,
            ObjectAxis::X | ObjectAxis::Y | ObjectAxis::Z => {
                let mut p = self.xyz();
                let k = match axis {
                    ObjectAxis::X => 0,
                    ObjectAxis::Y => 1,
                    _ => 2,
                };
                p[k] = value;
                self.set_xyz(p);
            }
        }
    }

    /// Where it is in the room as a box: x left (-1) to right (1), y back (-1) to front (1), z the
    /// ears (0) to the ceiling (1). The walls are where the speakers are; `radius` scales from the
    /// listener out to them.
    pub fn xyz(self) -> [f32; 3] {
        let radius = if self.radius.is_finite() { self.radius.clamp(0.0, 1.0) } else { 1.0 };
        let d = crate::vbap::direction(self.azimuth, self.elevation);
        let wall = d[0].abs().max(d[1].abs()).max(d[2].abs()).max(1e-6);
        [d[0] / wall * radius, d[1] / wall * radius, d[2] / wall * radius]
    }

    /// Puts it at a point of the box (see [`Self::xyz`]). At the listener the direction stays as it was.
    pub fn set_xyz(&mut self, p: [f32; 3]) {
        let p = p.map(|v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 });
        let p = [p[0], p[1], p[2].max(0.0)];
        let radius = p[0].abs().max(p[1].abs()).max(p[2].abs());
        self.radius = radius.min(1.0);
        if radius < 1e-4 {
            return;
        }
        let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        self.azimuth = p[0].atan2(p[1]).to_degrees();
        self.elevation = (p[2] / length).clamp(-1.0, 1.0).asin().to_degrees().clamp(0.0, 90.0);
    }
}

/// How an object's left and right feed each channel of the bus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ObjectGains {
    pub left: [f32; SPATIAL_CHANNELS],
    pub right: [f32; SPATIAL_CHANNELS],
}

fn direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [a.sin() * e.cos(), a.cos() * e.cos(), e.sin()]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How a single point feeds the speakers of `layout`, in constant power: each speaker by how close it
/// is, sharply for a small object and evenly for a wide one.
pub fn point_gains(layout: SpeakerLayout, azimuth: f32, elevation: f32, size: f32) -> [f32; SPATIAL_CHANNELS] {
    point_gains_with(layout, azimuth, elevation, size, 1.0)
}

/// [`point_gains`], with the centre speaker's part scaled by `center` (0 leaves it out), the rest made
/// up so the power stays the same.
pub fn point_gains_with(
    layout: SpeakerLayout,
    azimuth: f32,
    elevation: f32,
    size: f32,
    center: f32,
) -> [f32; SPATIAL_CHANNELS] {
    let center = if center.is_finite() { center.clamp(0.0, 1.0) } else { 1.0 };
    let there = direction(azimuth, elevation);
    let size = if size.is_finite() { size.clamp(0.0, 1.0) } else { 0.0 };
    let sharp = 24.0 + (1.5 - 24.0) * size;
    let mut gains = [0f32; SPATIAL_CHANNELS];
    let mut power = 0.0;
    let mut nearest = (0, f32::MIN);
    for (c, gain) in gains.iter_mut().enumerate() {
        let Some((az, el)) = layout.place(c) else {
            continue;
        };
        let close = dot(there, direction(az, el));
        let weight = if c == 2 { center } else { 1.0 };
        if close > nearest.1 && weight > 0.0 {
            nearest = (c, close);
        }
        // Halfway around is where a speaker stops counting.
        *gain = ((close + 1.0) / 2.0).powf(sharp) * weight;
        power += *gain * *gain;
    }
    if power <= 1e-12 {
        gains = [0.0; SPATIAL_CHANNELS];
        gains[nearest.0] = 1.0;
        return gains;
    }
    let scale = power.sqrt().recip();
    gains.map(|g| g * scale)
}

/// How an object's two channels feed the bus. Its left and right spread apart with its size, and as a
/// stereo strip at the centre has both at full level, each gives half the power.
pub fn object_gains(layout: SpeakerLayout, place: ObjectPlace) -> ObjectGains {
    let azimuth = if place.azimuth.is_finite() { place.azimuth } else { 0.0 };
    let elevation = if place.elevation.is_finite() {
        place.elevation.clamp(-90.0, 90.0)
    } else {
        0.0
    };
    let size = if place.size.is_finite() { place.size.clamp(0.0, 1.0) } else { 0.0 };
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let spread = if place.spread.is_finite() { place.spread.clamp(0.0, 1.0) } else { 0.5 };
    let side = |offset: f32| {
        point_gains_with(layout, azimuth + offset * spread * 45.0, elevation, size, place.center)
            .map(|g| g * half)
    };
    let lfe = if place.lfe.is_finite() { place.lfe.clamp(0.0, 1.0) } else { 0.0 };
    let mut gains = ObjectGains {
        left: side(-1.0),
        right: side(1.0),
    };
    gains.left[LFE] = lfe * half;
    gains.right[LFE] = lfe * half;
    gains
}

/// One ear's path from one virtual speaker.
#[derive(Clone, Copy, Default)]
struct Ear {
    /// The whole and the fractional part of its lag, in frames.
    lag: usize,
    fraction: f32,
    /// The head's shadow, as a one-pole one-zero filter.
    b0: f32,
    b1: f32,
    a1: f32,
    in1: f32,
    out1: f32,
    /// Coefficient and state of the one-pole cue on top of that.
    cue: f32,
    tilt: f32,
    cue_state: f32,
}

impl Ear {
    fn new(sample_rate: f32, ear_side: f32, azimuth: f32, elevation: f32) -> Self {
        // The angle between this ear and the source.
        let cos = dot([ear_side, 0.0, 0.0], direction(azimuth, elevation)).clamp(-1.0, 1.0);
        let angle = cos.acos();
        // Brown and Duda: the shadow is strongest a little past the far side of the head.
        let alpha = 1.05 + 0.95 * (angle * 180.0 / 150.0).cos();
        let tau = HEAD / (2.0 * SOUND);
        let k = 2.0 * sample_rate;
        let a0 = 1.0 + k * tau;
        let (b0, b1, a1) = (
            (1.0 + k * alpha * tau) / a0,
            (1.0 - k * alpha * tau) / a0,
            (1.0 - k * tau) / a0,
        );
        // Woodworth: the way around the head to this ear.
        let seconds = if angle < PI / 2.0 {
            HEAD / SOUND * (1.0 - angle.cos())
        } else {
            HEAD / SOUND * (angle - PI / 2.0 + 1.0)
        };
        let frames = (seconds * sample_rate).clamp(0.0, (DELAY_FRAMES - 2) as f32);
        let behind = azimuth.abs() > 90.0;
        let above = elevation > 10.0;
        // Above: a touch more air. Behind: a touch less.
        let (hz, tilt) = match (behind, above) {
            (true, false) => (7000.0, -0.35),
            (true, true) => (7000.0, -0.1),
            (false, true) => (7000.0, 0.3),
            (false, false) => (7000.0, 0.0),
        };
        Self {
            lag: frames as usize,
            fraction: frames.fract(),
            b0,
            b1,
            a1,
            cue: 1.0 - (-2.0 * PI * hz / sample_rate).exp(),
            tilt,
            ..Self::default()
        }
    }

    #[inline]
    fn filter(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.in1 - self.a1 * self.out1;
        self.in1 = x;
        self.out1 = y;
        self.cue_state += self.cue * (y - self.cue_state);
        // What is above the cue's corner is `y` less its low part.
        y + self.tilt * (y - self.cue_state)
    }
}

/// One virtual speaker for headphones.
struct Virtual {
    channel: usize,
    /// Where it is round the listener, for a channel that does not go through the head.
    azimuth: f32,
    ears: [Ear; 2],
    /// What it has played, for the ears' lags.
    delay: Box<[f32; DELAY_FRAMES]>,
    at: usize,
}

/// The first reflections of a room: without them a modelled head tends to put everything inside the
/// listener's own. Delays and gains for each ear, the other ear's sound crossing over.
const ROOM_TAPS: [(f32, f32, bool); 6] = [
    (0.0071, 0.55, true),
    (0.0113, 0.45, false),
    (0.0167, 0.38, true),
    (0.0229, 0.30, false),
    (0.0311, 0.24, true),
    (0.0433, 0.18, false),
];
/// The frames of a room's memory: enough for its longest tap at 192 kHz.
const ROOM_FRAMES: usize = 8192 * 2;

/// The render to headphones.
struct Binaural {
    speakers: Vec<Virtual>,
    room_l: Box<[f32; ROOM_FRAMES]>,
    room_r: Box<[f32; ROOM_FRAMES]>,
    room_at: usize,
    /// Each tap: its delay in frames, gain and whether it crosses to the other ear.
    taps: [(usize, f32, bool); ROOM_TAPS.len()],
    /// What goes into the room this span, each speaker by its distance.
    send_l: Box<[f64; BUS_FRAMES]>,
    send_r: Box<[f64; BUS_FRAMES]>,
}

impl Binaural {
    fn new(sample_rate: u32, layout: SpeakerLayout) -> Self {
        let sr = sample_rate.max(1) as f32;
        let speakers = (0..SPATIAL_CHANNELS)
            .filter_map(|channel| {
                let (az, el) = layout.place(channel)?;
                Some(Virtual {
                    channel,
                    azimuth: az,
                    ears: [Ear::new(sr, -1.0, az, el), Ear::new(sr, 1.0, az, el)],
                    delay: Box::new([0.0; DELAY_FRAMES]),
                    at: 0,
                })
            })
            .collect();
        let taps = ROOM_TAPS.map(|(seconds, gain, cross)| {
            (((seconds * sr) as usize).min(ROOM_FRAMES - 1), gain, cross)
        });
        Self {
            speakers,
            room_l: Box::new([0.0; ROOM_FRAMES]),
            room_r: Box::new([0.0; ROOM_FRAMES]),
            room_at: 0,
            taps,
            send_l: Box::new([0.0; BUS_FRAMES]),
            send_r: Box::new([0.0; BUS_FRAMES]),
        }
    }

    /// Adds the room's first reflections of what was sent to it this span.
    fn add_room(&mut self, room: f32, left: &mut [f64], right: &mut [f64]) {
        let room = f64::from(room);
        for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            self.room_l[self.room_at] = self.send_l[j] as f32;
            self.room_r[self.room_at] = self.send_r[j] as f32;
            let (mut add_l, mut add_r) = (0f32, 0f32);
            for &(lag, gain, cross) in &self.taps {
                let at = (self.room_at + ROOM_FRAMES - lag) % ROOM_FRAMES;
                let (from_l, from_r) = (self.room_l[at], self.room_r[at]);
                if cross {
                    add_l += from_r * gain;
                    add_r += from_l * gain;
                } else {
                    add_l += from_l * gain;
                    add_r += from_r * gain;
                }
            }
            *l += f64::from(add_l) * room;
            *r += f64::from(add_r) * room;
            self.room_at = (self.room_at + 1) % ROOM_FRAMES;
        }
    }

    /// Adds the ears' sound for the frames of `bus` to `left` and `right`, each speaker as far as
    /// `distance` says, and what of it goes to the room to the room's input.
    fn render(
        &mut self,
        bus: &[[f64; BUS_FRAMES]; SPATIAL_CHANNELS],
        distance: &[Distance; SPATIAL_CHANNELS],
        left: &mut [f64],
        right: &mut [f64],
    ) {
        let n = left.len();
        self.send_l[..n].fill(0.0);
        self.send_r[..n].fill(0.0);
        for v in &mut self.speakers {
            let input = &bus[v.channel];
            let how = distance[v.channel];
            if how == Distance::Off {
                // Plainly on its side, as on a pair of speakers.
                let side = (v.azimuth / 90.0).clamp(-1.0, 1.0);
                let angle = (side + 1.0) * std::f32::consts::FRAC_PI_4;
                let (to_l, to_r) = (f64::from(angle.cos()), f64::from(angle.sin()));
                for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    *l += input[j] * to_l;
                    *r += input[j] * to_r;
                }
                continue;
            }
            let (direct, room) = how.amounts();
            let (direct, room) = (f64::from(direct), f64::from(room));
            for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                v.delay[v.at] = input[j] as f32;
                let [ear_l, ear_r] = &mut v.ears;
                let a = f64::from(ear_l.filter(read_delay(&v.delay, v.at, ear_l.lag, ear_l.fraction)));
                let b = f64::from(ear_r.filter(read_delay(&v.delay, v.at, ear_r.lag, ear_r.fraction)));
                *l += a * direct;
                *r += b * direct;
                self.send_l[j] += a * room;
                self.send_r[j] += b * room;
                v.at = (v.at + 1) % DELAY_FRAMES;
            }
        }
        // The LFE to both ears, as it has no place.
        for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            *l += bus[LFE][j];
            *r += bus[LFE][j];
        }
    }
}

#[inline]
fn read_delay(delay: &[f32; DELAY_FRAMES], at: usize, lag: usize, fraction: f32) -> f32 {
    let a = delay[(at + DELAY_FRAMES - lag) % DELAY_FRAMES];
    let b = delay[(at + DELAY_FRAMES - lag - 1) % DELAY_FRAMES];
    a + (b - a) * fraction
}

/// How a layout folds to stereo, as (to left, to right) for each channel, when the device has too few
/// outputs for it or the sound is going to a stereo file.
fn fold_gains(layout: SpeakerLayout) -> [(f64, f64); SPATIAL_CHANNELS] {
    let mut fold = [(0.0, 0.0); SPATIAL_CHANNELS];
    let side = std::f64::consts::FRAC_1_SQRT_2;
    for (c, gain) in fold.iter_mut().enumerate() {
        let Some((az, _)) = layout.place(c) else {
            continue;
        };
        *gain = if az.abs() < 1.0 {
            (side, side)
        } else if az < 0.0 {
            (if c == 0 { 1.0 } else { side }, 0.0)
        } else {
            (0.0, if c == 1 { 1.0 } else { side })
        };
    }
    fold
}

/// The bus and what renders it. Made on the UI thread and handed over whole: it allocates, the audio
/// thread does not.
pub struct SpatialRender {
    pub(crate) mode: SpatialMode,
    law: SpatialLaw,
    /// The panning tables of the layout, for [`SpatialLaw::Vbap`].
    vbap: Option<Box<crate::vbap::Vbap>>,
    bus: Box<[[f64; BUS_FRAMES]; SPATIAL_CHANNELS]>,
    binaural: Option<Binaural>,
    /// For objects with a distance of their own, in the headphone render: a bus and a head for
    /// each distance, and whether anything went to it this span.
    own: Vec<(Box<[[f64; BUS_FRAMES]; SPATIAL_CHANNELS]>, Binaural, bool)>,
    fold: [(f64, f64); SPATIAL_CHANNELS],
    /// How much of the headphone render's room there is, 0..=1.
    room: f32,
    setup: ChannelSetup,
    sample_rate: f64,
    /// Where the speakers objects are aimed at are (the room's, for speakers straight out).
    places: [Option<(f32, f32)>; SPATIAL_CHANNELS],
    /// For [`SpatialMode::Downmix`]: the matrix, and a bus to mix into.
    downmix: Option<(Box<[[f32; SPATIAL_CHANNELS]; SPATIAL_CHANNELS]>, Box<[[f64; BUS_FRAMES]; SPATIAL_CHANNELS]>)>,
    /// The room's speakers: their gains and delays (and the delay lines, when any is delayed).
    trim_gain: [f64; SPATIAL_CHANNELS],
    delay_frames: [usize; SPATIAL_CHANNELS],
    delays: Vec<Box<[f64; DELAY_RING]>>,
    delay_at: usize,
    lfe_filter: Option<LowPass>,
    /// The channel playing the test signal, and the noise's state.
    test: Option<usize>,
    noise: (u32, [f64; 3]),
    /// The spatial buses, in order (an object's `bus` 1 is the first).
    buses: Vec<SpatialBus>,
    /// Delay compensation of the rest of the mix against the slowest bus (made with the buses).
    main_delay: Option<Box<DelayRing>>,
    main_delay_was: usize,
    /// The headphone render through a measured head, when one is loaded (else the modelled head).
    hrtf: Option<HrtfRender>,
    /// In the headphone render, the bed stays plain stereo and only the objects go through the
    /// head: spatial tracks in a project that is not spatial.
    bed_dry: bool,
}

impl SpatialRender {
    /// A renderer with the [`SpatialLaw::Smooth`] law.
    pub fn new(mode: SpatialMode, sample_rate: u32) -> Box<Self> {
        Self::with_law(mode, sample_rate, SpatialLaw::Smooth)
    }

    /// A renderer panning objects with `law`. Allocates, and builds the panning tables: call it off
    /// the audio thread.
    pub fn with_law(mode: SpatialMode, sample_rate: u32, law: SpatialLaw) -> Box<Self> {
        Self::with_speakers(mode, sample_rate, law, SpeakerSetup::default())
    }

    /// [`Self::with_law`], heard on the room's `speakers`: where they really are, their levels and
    /// delays (used when the bus goes straight out to speakers).
    pub fn with_speakers(
        mode: SpatialMode,
        sample_rate: u32,
        law: SpatialLaw,
        speakers: SpeakerSetup,
    ) -> Box<Self> {
        let layout = mode.layout().unwrap_or_default();
        let out = mode.output_layout().unwrap_or_default();
        let straight = matches!(mode, SpatialMode::Speakers(_));
        // The room's places matter only where the bed itself goes to the speakers.
        let places: [Option<(f32, f32)>; SPATIAL_CHANNELS] = std::array::from_fn(|c| {
            let base = layout.place(c)?;
            Some(if straight { speakers.place[c].unwrap_or(base) } else { base })
        });
        let outs = matches!(mode, SpatialMode::Speakers(_) | SpatialMode::Downmix(..));
        let trim_gain: [f64; SPATIAL_CHANNELS] = std::array::from_fn(|c| {
            let db = speakers.gain_db[c];
            if outs && db.is_finite() {
                10f64.powf(f64::from(db.clamp(-24.0, 12.0)) / 20.0)
            } else {
                1.0
            }
        });
        let sr = f64::from(sample_rate.max(1));
        let delay_frames: [usize; SPATIAL_CHANNELS] = std::array::from_fn(|c| {
            let ms = speakers.delay_ms[c];
            if outs && ms.is_finite() {
                ((f64::from(ms.clamp(0.0, MAX_SPEAKER_DELAY_MS)) * sr / 1000.0).round() as usize)
                    .min(DELAY_RING - 1)
            } else {
                0
            }
        });
        let delays = if delay_frames.iter().any(|d| *d > 0) {
            (0..SPATIAL_CHANNELS).map(|_| Box::new([0.0; DELAY_RING])).collect()
        } else {
            Vec::new()
        };
        let downmix = match mode {
            SpatialMode::Downmix(from, to) => Some((
                Box::new(downmix_matrix(from, to)),
                Box::new([[0.0; BUS_FRAMES]; SPATIAL_CHANNELS]),
            )),
            _ => None,
        };
        Box::new(Self {
            mode,
            law,
            vbap: (law == SpatialLaw::Vbap && mode != SpatialMode::Off)
                .then(|| Box::new(crate::vbap::Vbap::with_places(layout, &places))),
            sample_rate: sr,
            places,
            downmix,
            trim_gain,
            delay_frames,
            delays,
            delay_at: 0,
            lfe_filter: None,
            test: None,
            noise: (0x1234_5678, [0.0; 3]),
            buses: Vec::new(),
            main_delay: None,
            main_delay_was: 0,
            hrtf: None,
            bed_dry: false,
            bus: Box::new([[0.0; BUS_FRAMES]; SPATIAL_CHANNELS]),
            binaural: matches!(mode, SpatialMode::Binaural(_))
                .then(|| Binaural::new(sample_rate, layout)),
            own: if matches!(mode, SpatialMode::Binaural(_)) {
                Distance::ALL
                    .iter()
                    .map(|_| {
                        (
                            Box::new([[0.0; BUS_FRAMES]; SPATIAL_CHANNELS]),
                            Binaural::new(sample_rate, layout),
                            false,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            },
            fold: fold_gains(out),
            room: 0.0,
            setup: ChannelSetup::default(),
        })
    }

    pub(crate) fn room(&self) -> f32 {
        self.room
    }

    /// How much room the headphone render has, 0..=1.
    pub(crate) fn set_room(&mut self, room: f32) {
        self.room = if room.is_finite() { room.clamp(0.0, 1.0) } else { 0.0 };
    }

    /// The layout objects are aimed at.
    pub(crate) fn layout(&self) -> SpeakerLayout {
        self.mode.layout().unwrap_or_default()
    }

    pub(crate) fn setup(&self) -> ChannelSetup {
        self.setup
    }

    pub(crate) fn set_setup(&mut self, setup: ChannelSetup) {
        let gain = if setup.gain.is_finite() { setup.gain.clamp(0.0, 4.0) } else { 1.0 };
        if setup.lfe_hz != self.setup.lfe_hz || self.lfe_filter.is_none() {
            self.lfe_filter = (setup.lfe_hz.is_finite() && setup.lfe_hz > 0.0)
                .then(|| LowPass::new(f64::from(setup.lfe_hz.clamp(20.0, 500.0)), self.sample_rate));
        }
        self.setup = ChannelSetup { gain, ..setup };
    }

    /// Plays pink noise on bus channel `channel` (none: stops), to find a speaker.
    pub(crate) fn set_test(&mut self, channel: Option<usize>) {
        self.test = channel.filter(|c| *c < SPATIAL_CHANNELS);
    }

    /// The speakers the sound goes out to.
    pub(crate) fn output_layout(&self) -> SpeakerLayout {
        self.mode.output_layout().unwrap_or_default()
    }

    /// One sample of pink noise (Paul Kellet's economy filter on white noise), about -20 dB.
    #[inline]
    fn pink(&mut self) -> f64 {
        let (seed, b) = &mut self.noise;
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        let white = f64::from(*seed) / f64::from(u32::MAX) * 2.0 - 1.0;
        b[0] = 0.99765 * b[0] + white * 0.0990460;
        b[1] = 0.96300 * b[1] + white * 0.2965164;
        b[2] = 0.57000 * b[2] + white * 1.0526913;
        (b[0] + b[1] + b[2] + white * 0.1848) * 0.05
    }

    pub fn law(&self) -> SpatialLaw {
        self.law
    }

    /// With these spatial buses, each a layout and where it is placed. Allocates: call it off the
    /// audio thread, before the renderer is sent.
    pub fn with_buses(mut self: Box<Self>, buses: &[(SpeakerLayout, BusPlace)]) -> Box<Self> {
        let vbap = self.law == SpatialLaw::Vbap;
        self.buses = buses
            .iter()
            .take(MAX_SPATIAL_BUSES)
            .map(|&(layout, _)| SpatialBus {
                layout,
                places: std::array::from_fn(|c| layout.place(c)),
                vbap: vbap.then(|| Box::new(crate::vbap::Vbap::new(layout))),
                buf: Box::new([[0.0; BUS_FRAMES]; SPATIAL_CHANNELS]),
                matrix: [[0.0; SPATIAL_CHANNELS]; SPATIAL_CHANNELS],
                used: false,
                fx: Vec::new(),
                plugins: Box::default(),
                delay: Box::new(DelayRing::new()),
                pairs: layout
                    .device_order()
                    .chunks(2)
                    .map(|c| (c[0], c.get(1).copied()))
                    .collect(),
            })
            .collect();
        for (k, (_, place)) in buses.iter().enumerate().take(MAX_SPATIAL_BUSES) {
            self.set_bus_place(k, *place);
        }
        if !self.buses.is_empty() {
            self.main_delay = Some(Box::new(DelayRing::new()));
        }
        self
    }

    /// With effects on its spatial buses: for each bus, its effects in order (kind, settings,
    /// bypassed). Allocates: call it off the audio thread, after [`Self::with_buses`].
    pub fn with_bus_effects(
        mut self: Box<Self>,
        effects: &[Vec<(crate::fx::FxKind, [f32; crate::fx::MAX_FX_PARAMS], bool)>],
        sample_rate: f32,
    ) -> Box<Self> {
        for (b, list) in self.buses.iter_mut().zip(effects) {
            b.fx = list
                .iter()
                .map(|&(kind, params, bypass)| {
                    b.pairs
                        .iter()
                        .map(|_| crate::fx::Fx::new(kind, params, bypass, sample_rate))
                        .collect()
                })
                .collect();
        }
        self
    }

    /// Puts `plugins` on bus `bus` in place of the ones it had, which come back to be dropped off
    /// the audio thread. `None` if there is no such bus (the new ones come back then).
    pub(crate) fn swap_bus_plugins(&mut self, bus: usize, mut plugins: Box<Vec<BusPlugin>>) -> Box<Vec<BusPlugin>> {
        if let Some(b) = self.buses.get_mut(bus) {
            std::mem::swap(&mut b.plugins, &mut plugins);
        }
        plugins
    }

    /// A parameter of plugin `slot` on bus `bus`, for every copy but the first (whose window moved
    /// it): linked stereo copies move together. On the audio thread.
    pub(crate) fn set_bus_plugin_param(&mut self, bus: usize, slot: usize, id: u32, value: f64) {
        if let Some(p) = self.buses.get_mut(bus).and_then(|b| b.plugins.get_mut(slot)) {
            for unit in p.units.iter_mut().skip(1) {
                unit.set_param(id, value);
            }
        }
    }

    /// Bypasses plugin `slot` on bus `bus`, or brings it back.
    pub(crate) fn set_bus_plugin_bypass(&mut self, bus: usize, slot: usize, bypass: bool) {
        if let Some(p) = self.buses.get_mut(bus).and_then(|b| b.plugins.get_mut(slot)) {
            p.bypass = bypass;
        }
    }

    /// Takes the plugins of `old`'s buses that have the same layout here: a new renderer keeps
    /// them running. On the audio thread: only boxes move.
    pub(crate) fn keep_plugins_of(&mut self, old: &mut SpatialRender) {
        for (new, was) in self.buses.iter_mut().zip(old.buses.iter_mut()) {
            if new.layout == was.layout {
                std::mem::swap(&mut new.plugins, &mut was.plugins);
            }
        }
    }

    /// New settings for effect `slot` of bus `bus`, on every pair it runs on. On the audio thread.
    pub(crate) fn set_bus_fx(
        &mut self,
        bus: usize,
        slot: usize,
        params: [f32; crate::fx::MAX_FX_PARAMS],
        bypass: bool,
    ) {
        if let Some(stage) = self.buses.get_mut(bus).and_then(|b| b.fx.get_mut(slot)) {
            for unit in stage.iter_mut() {
                unit.set(params, bypass);
            }
        }
    }

    /// With a measured head for the headphone render (the modelled head stays for the room).
    /// Allocates: call it off the audio thread.
    pub fn with_hrtf(mut self: Box<Self>, set: &HrtfSet) -> Box<Self> {
        if matches!(self.mode, SpatialMode::Binaural(_)) && !set.speakers.is_empty() {
            self.hrtf = Some(HrtfRender::new(set));
        }
        self
    }

    /// Leaves the bed (the tracks that are not spatial) as plain stereo in the headphone render: only
    /// the spatial tracks go through the head.
    pub fn with_dry_bed(mut self: Box<Self>) -> Box<Self> {
        self.bed_dry = true;
        self
    }

    /// Whether the headphone render uses a measured head.
    pub fn has_hrtf(&self) -> bool {
        self.hrtf.is_some()
    }

    /// Places spatial bus `k` (from 0). Allocates nothing.
    pub(crate) fn set_bus_place(&mut self, k: usize, place: BusPlace) {
        let Some(layout) = self.buses.get(k).map(|b| b.layout) else {
            return;
        };
        let mut m = [[0f32; SPATIAL_CHANNELS]; SPATIAL_CHANNELS];
        if !place.muted {
            let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
            let gain = finite(place.gain, 1.0).clamp(0.0, 4.0);
            let spread = finite(place.spread, 1.0).clamp(0.0, 1.0);
            let size = finite(place.size, 0.0).clamp(0.0, 1.0);
            for &c in layout.device_order() {
                if c == LFE {
                    m[c][LFE] = finite(place.lfe, 1.0).clamp(0.0, 1.0) * gain;
                    continue;
                }
                let Some((az, el)) = layout.place(c) else {
                    continue;
                };
                let az = finite(place.direction, 0.0) + az * spread;
                let el = (el + finite(place.elevation, 0.0)).clamp(-90.0, 90.0);
                let g = self.point(self.layout(), self.vbap.as_deref(), &self.places, az, el, size, 1.0, false);
                let w = if c == 2 { finite(place.centre, 1.0).clamp(0.0, 1.0) } else { 1.0 };
                for (t, v) in g.iter().enumerate() {
                    m[c][t] = v * gain * w;
                }
            }
        }
        if let Some(bus) = self.buses.get_mut(k) {
            bus.matrix = m;
        }
    }

    /// Sums the spatial buses into the main bus, each as it is placed, and empties them.
    pub(crate) fn mix_buses(&mut self, n: usize, transport: &crate::plugin::PluginTransport) {
        // Delay compensation: the rest of the mix waits for the slowest bus, and each bus for the
        // difference between it and that one.
        let lat = |b: &SpatialBus| b.plugins.iter().map(BusPlugin::latency_samples).sum::<usize>().min(BUS_DELAY_MAX - 1);
        let slowest = self.buses.iter().map(lat).max().unwrap_or(0);
        if slowest > 0 || self.main_delay_was > 0 {
            if let Some(ring) = self.main_delay.as_mut() {
                ring.run(&mut self.bus, n, slowest);
            }
        }
        self.main_delay_was = slowest;
        for b in self.buses.iter_mut() {
            let wait = slowest - lat(b);
            if wait > 0 && !b.used {
                // Its delay line still runs, empty, so nothing old comes out later.
                for channel in b.buf.iter_mut() {
                    channel[..n].fill(0.0);
                }
                b.used = true;
            }
        }
        for b in self.buses.iter_mut().filter(|b| b.used) {
            let wait = slowest - lat(b);
            // Its effects first, pair by pair, then its plugins.
            let order = b.layout.device_order();
            let SpatialBus { fx, pairs, buf, plugins, delay, .. } = b;
            for plug in plugins.iter_mut() {
                plug.run(buf, order, pairs, n, transport);
            }
            if wait > 0 {
                delay.run(buf, n, wait);
            }
            for stage in fx.iter_mut() {
                for (unit, &(lc, rc)) in stage.iter_mut().zip(pairs.iter()) {
                    for j in 0..n {
                        let l = buf[lc][j] as f32;
                        let r = rc.map_or(l, |c| buf[c][j] as f32);
                        let (yl, yr) = unit.process(l, r);
                        buf[lc][j] = f64::from(yl);
                        if let Some(c) = rc {
                            buf[c][j] = f64::from(yr);
                        }
                    }
                }
            }
            for (c, row) in b.matrix.iter().enumerate() {
                for (t, g) in row.iter().enumerate() {
                    if *g == 0.0 {
                        continue;
                    }
                    let g = f64::from(*g);
                    for j in 0..n {
                        self.bus[t][j] += b.buf[c][j] * g;
                    }
                }
            }
            for channel in b.buf.iter_mut() {
                channel[..n].fill(0.0);
            }
            b.used = false;
        }
    }

    /// The gains of a point among the speakers of `layout` (at `places`), by this renderer's law.
    #[allow(clippy::too_many_arguments)]
    fn point(
        &self,
        layout: SpeakerLayout,
        vbap: Option<&crate::vbap::Vbap>,
        places: &[Option<(f32, f32)>; SPATIAL_CHANNELS],
        az: f32,
        elevation: f32,
        spread_out: f32,
        centre: f32,
        snap: bool,
    ) -> [f32; SPATIAL_CHANNELS] {
        if snap {
            return snap_gains(places, az, elevation, centre);
        }
        let wide = || point_gains_with(layout, az, elevation, spread_out, centre);
        match vbap {
            None => wide(),
            Some(vbap) => {
                let sharp = vbap.gains(az, elevation, centre);
                if spread_out <= 0.0 {
                    return sharp;
                }
                let soft = wide();
                let mut out = [0f32; SPATIAL_CHANNELS];
                for ((o, a), b) in out.iter_mut().zip(sharp).zip(soft) {
                    *o = ((1.0 - spread_out) * a * a + spread_out * b * b).sqrt();
                }
                out
            }
        }
    }

    /// How an object's two channels feed the bus, by this renderer's law, its snap and its
    /// divergence: the main bus, or the spatial bus it goes through, in that bus's layout. Allocates
    /// nothing.
    pub(crate) fn object_gains(&self, place: ObjectPlace) -> ObjectGains {
        let through = usize::from(place.bus).checked_sub(1).and_then(|k| self.buses.get(k));
        let (layout, vbap, places) = match through {
            Some(b) => (b.layout, b.vbap.as_deref(), &b.places),
            None => (self.layout(), self.vbap.as_deref(), &self.places),
        };
        let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let azimuth = finite(place.azimuth, 0.0);
        let elevation = finite(place.elevation, 0.0).clamp(-90.0, 90.0);
        let size = finite(place.size, 0.0).clamp(0.0, 1.0);
        let radius = finite(place.radius, 1.0).clamp(0.0, 1.0);
        let spread = finite(place.spread, 0.5).clamp(0.0, 1.0);
        let divergence = finite(place.divergence, 0.0).clamp(0.0, 1.0);
        let lfe = finite(place.lfe, 0.0).clamp(0.0, 1.0);
        // Inside the room it spreads: at the listener over every speaker.
        let spread_out = 1.0 - (1.0 - size) * radius;
        let point = |az: f32| -> [f32; SPATIAL_CHANNELS] {
            self.point(layout, vbap, places, az, elevation, spread_out, place.center, place.snap)
        };
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let side = |offset: f32| {
            let az = azimuth + offset * spread * 45.0;
            let g = if divergence <= 0.0 {
                point(az)
            } else {
                // Split in power between where it is and two phantom places either side.
                let (a, b, c) = (point(az), point(az - 45.0), point(az + 45.0));
                let mut out = [0f32; SPATIAL_CHANNELS];
                for k in 0..SPATIAL_CHANNELS {
                    let p = (1.0 - divergence) * a[k] * a[k]
                        + divergence * 0.5 * (b[k] * b[k] + c[k] * c[k]);
                    out[k] = p.max(0.0).sqrt();
                }
                out
            };
            g.map(|x| x * half)
        };
        let mut gains = ObjectGains {
            left: side(-1.0),
            right: side(1.0),
        };
        gains.left[LFE] = lfe * half;
        gains.right[LFE] = lfe * half;
        gains
    }

}

/// All of a point on the one speaker nearest to it (the centre left out when `centre` is 0).
fn snap_gains(
    places: &[Option<(f32, f32)>; SPATIAL_CHANNELS],
    azimuth: f32,
    elevation: f32,
    centre: f32,
) -> [f32; SPATIAL_CHANNELS] {
    {
        let there = direction(azimuth, elevation);
        let mut best = (usize::MAX, f32::MIN);
        for (c, place) in places.iter().enumerate() {
            let Some((az, el)) = *place else {
                continue;
            };
            if c == 2 && centre <= 0.0 {
                continue;
            }
            let close = dot(there, direction(az, el));
            if close > best.1 {
                best = (c, close);
            }
        }
        let mut gains = [0f32; SPATIAL_CHANNELS];
        if let Some(g) = gains.get_mut(best.0) {
            *g = 1.0;
        }
        gains
    }
}

impl SpatialRender {
    pub(crate) fn is_on(&self) -> bool {
        self.mode != SpatialMode::Off
    }

    /// Adds an object's sound to a channel: to the bus, or (in the headphone render, with a distance
    /// of its own) to the bus of that distance.
    #[inline]
    pub(crate) fn add_object(
        &mut self,
        bus: u8,
        distance: Option<Distance>,
        channel: usize,
        frame: usize,
        value: f64,
    ) {
        if let Some(b) = usize::from(bus).checked_sub(1).and_then(|k| self.buses.get_mut(k)) {
            b.buf[channel][frame] += value;
            b.used = true;
            return;
        }
        let own = distance.and_then(|d| Distance::ALL.iter().position(|x| *x == d));
        match own.and_then(|k| self.own.get_mut(k)) {
            Some((bus, _, used)) => {
                bus[channel][frame] += value;
                *used = true;
            }
            None => self.bus[channel][frame] += value,
        }
    }

    /// What a channel of the bus holds.
    #[inline]
    pub(crate) fn sample(&self, channel: usize, frame: usize) -> f64 {
        self.bus[channel][frame]
    }

    /// Readies a span for the output, with the bed in `left` and `right`. True if channels from the
    /// third on go straight out; otherwise all of it is in `left` and `right` now.
    pub(crate) fn finish_span(
        &mut self,
        left: &mut [f64],
        right: &mut [f64],
        can_go_straight: bool,
    ) -> bool {
        let n = left.len();
        // The renderer's volume, and what is muted, before it goes anywhere.
        let gain = f64::from(self.setup.gain);
        for (c, channel) in self.bus.iter_mut().enumerate() {
            let g = if self.setup.muted[c] { 0.0 } else { gain };
            if g != 1.0 {
                channel[..n].iter_mut().for_each(|v| *v *= g);
            }
        }
        let bed_l = if self.setup.muted[0] { 0.0 } else { gain };
        let bed_r = if self.setup.muted[1] { 0.0 } else { gain };
        if bed_l != 1.0 || bed_r != 1.0 {
            left.iter_mut().for_each(|v| *v *= bed_l);
            right.iter_mut().for_each(|v| *v *= bed_r);
        }
        if let Some(filter) = self.lfe_filter.as_mut() {
            for v in &mut self.bus[LFE][..n] {
                *v = filter.tick(*v);
            }
        }
        if let Some(c) = self.test {
            for j in 0..n {
                let s = self.pink();
                match c {
                    0 => left[j] += s,
                    1 => right[j] += s,
                    c => self.bus[c][j] += s,
                }
            }
        }
        match self.mode {
            SpatialMode::Off => false,
            SpatialMode::Binaural(_) => {
                // The bed through the same head, or it would sit inside the head with the rest around
                // it; unless only the spatial tracks are meant to go through it.
                if !self.bed_dry {
                    for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                        self.bus[0][j] += *l;
                        self.bus[1][j] += *r;
                        *l = 0.0;
                        *r = 0.0;
                    }
                }
                if let Some(measured) = self.hrtf.as_mut() {
                    // A measured head: the objects with a distance of their own join the rest.
                    for (bus, _, used) in self.own.iter_mut() {
                        if !std::mem::take(used) {
                            continue;
                        }
                        for (c, channel) in bus.iter_mut().enumerate() {
                            let g = if self.setup.muted[c] { 0.0 } else { gain };
                            for j in 0..n {
                                self.bus[c][j] += channel[j] * g;
                            }
                            channel[..n].fill(0.0);
                        }
                    }
                    measured.render(&self.bus, &self.setup.distance, left, right);
                    return false;
                }
                if let Some(head) = self.binaural.as_mut() {
                    head.render(&self.bus, &self.setup.distance, left, right);
                    if self.room > 0.0 {
                        head.add_room(self.room, left, right);
                    }
                }
                // The objects that chose their own distance, each group through a head of its own.
                for (k, (bus, head, used)) in self.own.iter_mut().enumerate() {
                    if !*used {
                        continue;
                    }
                    for (c, channel) in bus.iter_mut().enumerate() {
                        let g = if self.setup.muted[c] { 0.0 } else { gain };
                        channel[..n].iter_mut().for_each(|v| *v *= g);
                    }
                    let mut distance = [Distance::ALL[k]; SPATIAL_CHANNELS];
                    distance[LFE] = Distance::Off;
                    head.render(bus, &distance, left, right);
                    if self.room > 0.0 {
                        head.add_room(self.room, left, right);
                    }
                    for channel in bus.iter_mut() {
                        channel[..n].fill(0.0);
                    }
                    *used = false;
                }
                false
            }
            SpatialMode::Downmix(..) => {
                // The bed's left and right join the bus, and the whole of it goes through the matrix.
                for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    self.bus[0][j] += *l;
                    self.bus[1][j] += *r;
                    *l = 0.0;
                    *r = 0.0;
                }
                if let Some((matrix, into)) = self.downmix.as_mut() {
                    for channel in into.iter_mut() {
                        channel[..n].fill(0.0);
                    }
                    for (c, row) in matrix.iter().enumerate() {
                        for (t, g) in row.iter().enumerate() {
                            if *g == 0.0 {
                                continue;
                            }
                            let g = f64::from(*g);
                            for j in 0..n {
                                into[t][j] += self.bus[c][j] * g;
                            }
                        }
                    }
                    for (c, channel) in self.bus.iter_mut().enumerate() {
                        channel[..n].copy_from_slice(&into[c][..n]);
                    }
                }
                self.to_speakers(left, right, can_go_straight)
            }
            SpatialMode::Speakers(_) => self.to_speakers(left, right, can_go_straight),
            SpatialMode::Folded(_) => {
                self.fold_down(left, right);
                false
            }
        }
    }

    /// The bus to the speakers: straight out when the device has the outputs, with the room's levels
    /// and delays; otherwise folded down to stereo.
    fn to_speakers(&mut self, left: &mut [f64], right: &mut [f64], straight: bool) -> bool {
        if !straight {
            self.fold_down(left, right);
            return false;
        }
        let n = left.len();
        for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            *l += self.bus[0][j];
            *r += self.bus[1][j];
        }
        let order = self.output_layout().device_order();
        let delayed = !self.delays.is_empty();
        for &c in order {
            let (g, lag) = (self.trim_gain[c], self.delay_frames[c]);
            if g == 1.0 && lag == 0 {
                continue;
            }
            let samples: &mut [f64] = match c {
                0 => &mut left[..n],
                1 => &mut right[..n],
                c => &mut self.bus[c][..n],
            };
            match self.delays.get_mut(c).filter(|_| delayed && lag > 0) {
                Some(line) => {
                    for (j, v) in samples.iter_mut().enumerate() {
                        let at = (self.delay_at + j) % DELAY_RING;
                        line[at] = *v;
                        *v = line[(at + DELAY_RING - lag) % DELAY_RING] * g;
                    }
                }
                None => samples.iter_mut().for_each(|v| *v *= g),
            }
        }
        // Channels with no delay of their own still keep their lines turning with the rest.
        if delayed {
            for &c in order {
                if self.delay_frames[c] == 0 {
                    if let Some(line) = self.delays.get_mut(c) {
                        for j in 0..n {
                            line[(self.delay_at + j) % DELAY_RING] = 0.0;
                        }
                    }
                }
            }
            self.delay_at = (self.delay_at + n) % DELAY_RING;
        }
        true
    }

    fn fold_down(&mut self, left: &mut [f64], right: &mut [f64]) {
        for (c, (to_l, to_r)) in self.fold.iter().enumerate() {
            for (j, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                *l += self.bus[c][j] * to_l;
                *r += self.bus[c][j] * to_r;
            }
        }
    }

    /// Empties the bus after a span.
    pub(crate) fn clear(&mut self, n: usize) {
        for channel in self.bus.iter_mut() {
            channel[..n].fill(0.0);
        }
    }

    /// The outputs the layout needs, if it is speakers.
    pub(crate) fn channels_needed(&self) -> usize {
        match self.mode {
            SpatialMode::Speakers(layout) | SpatialMode::Downmix(_, layout) => layout.channels(),
            _ => 2,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn power(g: &[f32; SPATIAL_CHANNELS]) -> f32 {
        g.iter().map(|v| v * v).sum()
    }

    #[test]
    fn a_point_keeps_its_power_anywhere_and_never_touches_the_lfe() {
        for layout in SpeakerLayout::ALL {
            for az in (-180..=180).step_by(15) {
                for el in [0.0, 30.0, 60.0, 90.0] {
                    for size in [0.0, 0.5, 1.0] {
                        let g = point_gains(layout, az as f32, el, size);
                        assert!((power(&g) - 1.0).abs() < 1e-3, "{layout:?} {az} {el} {size}");
                        assert_eq!(g[LFE], 0.0);
                    }
                }
            }
        }
    }

    #[test]
    fn a_small_object_on_a_speaker_plays_from_that_speaker() {
        let g = point_gains(SpeakerLayout::Surround714, 30.0, 0.0, 0.0);
        assert!(g[1] > 0.9, "{g:?}");
        let g = point_gains(SpeakerLayout::Surround714, -135.0, 45.0, 0.0);
        assert!(g[10] > 0.9, "{g:?}");
        // A layout without height keeps the sound in the room it has.
        let g = point_gains(SpeakerLayout::Surround51, 0.0, 90.0, 0.0);
        assert!((power(&g) - 1.0).abs() < 1e-3);
        assert_eq!(g[8], 0.0);
    }

    #[test]
    fn nonsense_places_make_no_nonsense() {
        let g = object_gains(
            SpeakerLayout::Surround714,
            ObjectPlace {
                azimuth: f32::NAN,
                elevation: f32::INFINITY,
                size: f32::NAN,
                lfe: f32::NAN,
                spread: f32::NAN,
                center: f32::NAN,
                distance: None,
            },
        );
        assert!(g.left.iter().chain(&g.right).all(|v| v.is_finite()));
    }

    /// What each ear gets from a burst played at one speaker, and how soon.
    fn ears_for(channel: usize) -> (f64, f64, usize, usize) {
        let mut r = SpatialRender::new(SpatialMode::Binaural(SpeakerLayout::Surround714), 48_000);
        for j in 0..64 {
            r.add(channel, j, if j == 0 { 1.0 } else { 0.0 });
        }
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        r.finish_span(&mut left, &mut right, false);
        let first = |x: &[f64]| x.iter().position(|v| v.abs() > 1e-3).unwrap_or(64);
        let energy = |x: &[f64]| x.iter().map(|v| v * v).sum::<f64>();
        (energy(&left), energy(&right), first(&left), first(&right))
    }

    #[test]
    fn a_sound_on_the_right_reaches_the_right_ear_first_and_louder() {
        let (l, r, first_l, first_r) = ears_for(7);
        assert!(r > l * 1.5, "{l} {r}");
        assert!(first_r < first_l, "{first_l} {first_r}");
        let (l, r, first_l, first_r) = ears_for(6);
        assert!(l > r * 1.5, "{l} {r}");
        assert!(first_l < first_r);
    }

    #[test]
    fn leaving_the_centre_out_puts_its_part_on_left_and_right() {
        let g = point_gains_with(SpeakerLayout::Surround714, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(g[2], 0.0);
        assert!((power(&g) - 1.0).abs() < 1e-3);
        assert!((g[0] - g[1]).abs() < 1e-3 && g[0] > 0.5);
    }

    #[test]
    fn a_muted_channel_is_silent_and_the_volume_scales_the_rest() {
        let mut r = SpatialRender::new(SpatialMode::Folded(SpeakerLayout::Surround714), 48_000);
        let mut setup = ChannelSetup::default();
        setup.muted[7] = true;
        setup.gain = 0.5;
        r.set_setup(setup);
        r.add(7, 0, 1.0);
        let (mut left, mut right) = (vec![0.4; 1], vec![0.0; 1]);
        r.finish_span(&mut left, &mut right, false);
        assert!((left[0] - 0.2).abs() < 1e-9, "{}", left[0]);
        assert_eq!(right[0], 0.0);
    }

    #[test]
    fn folding_keeps_every_channel_of_the_layout_on_its_side() {
        let mut r = SpatialRender::new(SpatialMode::Speakers(SpeakerLayout::Surround714), 48_000);
        r.add(7, 0, 1.0);
        let (mut left, mut right) = (vec![0.0; 1], vec![0.0; 1]);
        assert!(!r.finish_span(&mut left, &mut right, false));
        assert_eq!(left[0], 0.0);
        assert!(right[0] > 0.5);
    }
}
