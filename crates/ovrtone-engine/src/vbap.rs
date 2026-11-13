//! Vector Base Amplitude Panning (Pulkki, "Virtual Sound Source Positioning Using Vector Base
//! Amplitude Panning", JAES 1997): a sound in a direction plays on the two or three speakers around
//! it, at the gains that sum their directions to its own, scaled to constant power.
//!
//! The speakers are triangulated once, as the faces of their convex hull, and each triangle's matrix
//! is inverted then (on the UI thread, where [`Vbap::new`] runs). On the audio thread, [`Vbap::gains`]
//! only multiplies 3x3 matrices: it allocates nothing.
//!
//! A layout with no speakers overhead gets a virtual one at the top, whose share is spread over the
//! ear-level ring (a sound going up turns diffuse rather than vanishing). Every layout gets a virtual
//! one below, whose share is dropped (nothing is placed below the ear, and the rest is scaled back up).
//! Stereo has only two speakers in front, so it pans between them by direction, a sound behind
//! mirrored to the front.

use crate::spatial::{SpeakerLayout, SPATIAL_CHANNELS};

/// Channel numbers of the virtual speakers.
const TOP: usize = SPATIAL_CHANNELS;
const BOTTOM: usize = SPATIAL_CHANNELS + 1;

#[derive(Clone, Copy, Debug)]
struct Triangle {
    channels: [usize; 3],
    /// The inverse of the matrix of the three speakers' directions (one per row).
    inverse: [[f32; 3]; 3],
}

/// The triangles of one speaker set.
#[derive(Clone, Debug, Default)]
struct Mesh {
    triangles: Vec<Triangle>,
}

/// A layout's panning tables.
#[derive(Clone, Debug)]
pub(crate) struct Vbap {
    layout: SpeakerLayout,
    full: Mesh,
    /// The same without the centre speaker, for objects that leave it out.
    no_centre: Option<Mesh>,
    /// The ear-level speakers, which the virtual top speaker is spread over.
    ring: Vec<usize>,
}

pub(crate) fn direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [a.sin() * e.cos(), a.cos() * e.cos(), e.sin()]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn invert(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let det = dot(m[0], cross(m[1], m[2]));
    if det.abs() < 1e-6 {
        return None;
    }
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    // The inverse's columns are these cross products over the determinant.
    let d = det.recip();
    Some([
        [c0[0] * d, c1[0] * d, c2[0] * d],
        [c0[1] * d, c1[1] * d, c2[1] * d],
        [c0[2] * d, c1[2] * d, c2[2] * d],
    ])
}

impl Mesh {
    /// The convex hull of `speakers` (channel, azimuth, elevation), as triangles. The directions
    /// are nudged apart a hair first, so that speakers on one plane (a ring, four tops) give one
    /// clean triangulation rather than overlapping ones. The matrices use the true directions.
    fn new(speakers: &[(usize, f32, f32)]) -> Self {
        let real: Vec<[f32; 3]> = speakers.iter().map(|&(_, a, e)| direction(a, e)).collect();
        let nudged: Vec<[f32; 3]> = speakers
            .iter()
            .enumerate()
            .map(|(i, &(_, a, e))| direction(a + i as f32 * 0.0137, e + i as f32 * 0.0091))
            .collect();
        let n = speakers.len();
        let mut triangles = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let normal = cross(sub(nudged[j], nudged[i]), sub(nudged[k], nudged[i]));
                    if dot(normal, normal) < 1e-12 {
                        continue;
                    }
                    let (mut above, mut below) = (false, false);
                    for (m, p) in nudged.iter().enumerate() {
                        if m == i || m == j || m == k {
                            continue;
                        }
                        let side = dot(normal, sub(*p, nudged[i]));
                        above |= side > 1e-7;
                        below |= side < -1e-7;
                    }
                    if above && below {
                        continue;
                    }
                    if let Some(inverse) = invert([real[i], real[j], real[k]]) {
                        triangles.push(Triangle {
                            channels: [speakers[i].0, speakers[j].0, speakers[k].0],
                            inverse,
                        });
                    }
                }
            }
        }
        Self { triangles }
    }

    /// Gains of the triangle around `p` as (channel, gain) for its three speakers. The triangle
    /// whose smallest gain is largest: inside it, that is 0 or more.
    fn find(&self, p: [f32; 3]) -> Option<[(usize, f32); 3]> {
        let mut best: Option<([(usize, f32); 3], f32)> = None;
        for t in &self.triangles {
            let g = [
                p[0] * t.inverse[0][0] + p[1] * t.inverse[1][0] + p[2] * t.inverse[2][0],
                p[0] * t.inverse[0][1] + p[1] * t.inverse[1][1] + p[2] * t.inverse[2][1],
                p[0] * t.inverse[0][2] + p[1] * t.inverse[1][2] + p[2] * t.inverse[2][2],
            ];
            let least = g[0].min(g[1]).min(g[2]);
            if best.is_none_or(|(_, b)| least > b) {
                best = Some((
                    [
                        (t.channels[0], g[0]),
                        (t.channels[1], g[1]),
                        (t.channels[2], g[2]),
                    ],
                    least,
                ));
            }
            if least >= 0.0 {
                break;
            }
        }
        best.map(|(g, _)| g)
    }
}

impl Vbap {
    /// The tables for `layout`. Allocates: call it off the audio thread.
    pub(crate) fn new(layout: SpeakerLayout) -> Self {
        let places: [Option<(f32, f32)>; SPATIAL_CHANNELS] = std::array::from_fn(|c| layout.place(c));
        Self::with_places(layout, &places)
    }

    /// The tables for `layout` with its speakers where `places` says (a measured room). Allocates.
    pub(crate) fn with_places(layout: SpeakerLayout, places: &[Option<(f32, f32)>; SPATIAL_CHANNELS]) -> Self {
        let speakers: Vec<(usize, f32, f32)> = (0..SPATIAL_CHANNELS)
            .filter_map(|c| places[c].map(|(a, e)| (c, a, e)))
            .collect();
        let ring: Vec<usize> = speakers
            .iter()
            .filter(|(_, _, e)| e.abs() < 1.0)
            .map(|(c, _, _)| *c)
            .collect();
        let overhead = speakers.iter().any(|(_, _, e)| *e > 1.0);
        let with_virtual = |set: &[(usize, f32, f32)]| {
            let mut all = set.to_vec();
            if !overhead {
                all.push((TOP, 0.0, 90.0));
            }
            all.push((BOTTOM, 0.0, -90.0));
            Mesh::new(&all)
        };
        let stereo = layout == SpeakerLayout::Stereo;
        let full = if stereo { Mesh::default() } else { with_virtual(&speakers) };
        let no_centre = (!stereo && speakers.iter().any(|(c, _, _)| *c == 2)).then(|| {
            let rest: Vec<_> = speakers.iter().copied().filter(|(c, _, _)| *c != 2).collect();
            with_virtual(&rest)
        });
        Self {
            layout,
            full,
            no_centre,
            ring,
        }
    }

    /// The power of each channel for a point in `direction`, from one mesh; sums to 1.
    fn power(&self, mesh: &Mesh, p: [f32; 3], out: &mut [f32; SPATIAL_CHANNELS]) {
        *out = [0.0; SPATIAL_CHANNELS];
        let Some(found) = mesh.find(p) else {
            out[0] = 0.5;
            out[1] = 0.5;
            return;
        };
        let mut total = 0.0;
        for (c, g) in found {
            let g = g.max(0.0);
            let power = g * g;
            match c {
                TOP => {
                    let share = power / self.ring.len().max(1) as f32;
                    for &r in &self.ring {
                        out[r] += share;
                    }
                }
                BOTTOM => continue,
                c => out[c] += power,
            }
            total += power;
        }
        // The share of a virtual speaker below was dropped: the rest is scaled back up.
        let kept: f32 = out.iter().sum();
        if kept > 1e-9 {
            out.iter_mut().for_each(|v| *v /= kept);
        } else if total <= 1e-9 || kept <= 1e-9 {
            *out = [0.0; SPATIAL_CHANNELS];
            out[0] = 0.5;
            out[1] = 0.5;
        }
    }

    /// Constant-power gains for a point at `azimuth`/`elevation` (degrees; 0 ahead, right positive,
    /// up positive). `centre` 0..=1 scales how much the centre speaker plays, the others taking up
    /// what it leaves.
    pub(crate) fn gains(&self, azimuth: f32, elevation: f32, centre: f32) -> [f32; SPATIAL_CHANNELS] {
        let mut gains = [0f32; SPATIAL_CHANNELS];
        if self.layout == SpeakerLayout::Stereo {
            // Behind mirrors to the front; past a speaker stays on it.
            let a = if azimuth.abs() > 90.0 {
                azimuth.signum() * (180.0 - azimuth.abs())
            } else {
                azimuth
            };
            let t = (a.clamp(-30.0, 30.0) + 30.0) / 60.0 * std::f32::consts::FRAC_PI_2;
            gains[0] = t.cos();
            gains[1] = t.sin();
            return gains;
        }
        let p = direction(azimuth, elevation);
        let mut power = [0f32; SPATIAL_CHANNELS];
        self.power(&self.full, p, &mut power);
        let centre = if centre.is_finite() { centre.clamp(0.0, 1.0) } else { 1.0 };
        if let (Some(mesh), true) = (self.no_centre.as_ref(), centre < 1.0) {
            let mut without = [0f32; SPATIAL_CHANNELS];
            self.power(mesh, p, &mut without);
            for (a, b) in power.iter_mut().zip(without) {
                *a = *a * centre + b * (1.0 - centre);
            }
        }
        for (g, w) in gains.iter_mut().zip(power) {
            *g = w.max(0.0).sqrt();
        }
        gains
    }
}
