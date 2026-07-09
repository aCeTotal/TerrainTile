//! User-defined material classes: what replaces the old fixed 8-layer
//! masks. A class is gated by optional terrain rules (min/max height and
//! slope, soft bands) and — unless it is a `base` class — by coverage the
//! user paints on the aerial view. Each class carries its own uploaded
//! PBR materials.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClassDef {
    pub id: u32,
    pub name: String,
    /// Display color for the aerial overlay, "#rrggbb".
    pub color: String,
    /// Mean albedo of the first material — what the far/cheap layers show.
    #[serde(default)]
    pub avg_color: String,
    /// Coverage 1 everywhere; the gates alone shape where it appears.
    #[serde(default)]
    pub base: bool,
    /// Skip the soft transition blur (e.g. grass against rock).
    #[serde(default)]
    pub sharp: bool,
    /// Rendered with the water shading path.
    #[serde(default)]
    pub water: bool,
    /// Road splines paint this class under the roadway.
    #[serde(default)]
    pub road: bool,
    /// Relative priority against other classes ("how much it shows").
    #[serde(default = "one")]
    pub weight: f32,
    pub h_min: Option<f32>,
    pub h_max: Option<f32>,
    pub slope_min: Option<f32>,
    pub slope_max: Option<f32>,
    #[serde(default)]
    pub materials: Vec<MaterialRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialRef {
    /// Relative to the output dir, e.g. "materials/2/Ground003".
    pub dir: String,
    /// 0..1 — how much this material shows against the previous one.
    #[serde(default = "one")]
    pub amount: f32,
    /// "mix" (noise blend) or "top" (patches on top).
    #[serde(default = "mix")]
    pub mode: String,
}

fn one() -> f32 {
    1.0
}

fn mix() -> String {
    "mix".into()
}

impl ClassDef {
    /// Terrain gate 0..1 with soft bands: ~0.5 m / 4° each side.
    pub fn gate(&self, h: f32, slope: f32) -> f32 {
        let mut g = 1.0f32;
        if let Some(min) = self.h_min {
            g *= smooth(min, min + band_h(min), h);
        }
        if let Some(max) = self.h_max {
            g *= 1.0 - smooth(max, max + band_h(max), h);
        }
        if let Some(min) = self.slope_min {
            g *= smooth(min, min + 4.0, slope);
        }
        if let Some(max) = self.slope_max {
            g *= 1.0 - smooth(max, max + 4.0, slope);
        }
        g
    }
}

fn band_h(limit: f32) -> f32 {
    (0.2 * limit.abs()).max(0.5)
}

#[inline]
pub fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The classes every new project starts with; the embedded ambientCG sets
/// are installed under materials/<id>/<name>/ at project creation.
pub fn default_classes() -> Vec<ClassDef> {
    let class = |id: u32, name: &str, color: &str| ClassDef {
        id,
        name: name.into(),
        color: color.into(),
        avg_color: color.into(),
        base: true,
        sharp: false,
        water: false,
        road: false,
        weight: 1.0,
        h_min: None,
        h_max: None,
        slope_min: None,
        slope_max: None,
        materials: Vec::new(),
    };
    let mat = |dir: &str| MaterialRef { dir: dir.into(), amount: 1.0, mode: "mix".into() };
    vec![
        ClassDef {
            water: true,
            sharp: true,
            weight: 10.0,
            h_max: Some(0.05),
            ..class(0, "vann", "#1a5c73")
        },
        ClassDef {
            weight: 3.0,
            h_min: Some(0.0),
            h_max: Some(3.5),
            slope_max: Some(8.0),
            materials: vec![mat("materials/1/Ground037")],
            ..class(1, "sand", "#e0d3a0")
        },
        ClassDef { materials: vec![mat("materials/2/Ground003")], ..class(2, "gress", "#57713a") },
        ClassDef {
            slope_min: Some(15.0),
            slope_max: Some(35.0),
            materials: vec![mat("materials/3/Ground037")],
            ..class(3, "jord", "#6b5636")
        },
        ClassDef {
            sharp: true,
            weight: 2.0,
            slope_min: Some(30.0),
            materials: vec![mat("materials/4/Rock051")],
            ..class(4, "fjell", "#77736c")
        },
        ClassDef { base: false, sharp: true, road: true, weight: 6.0, ..class(5, "vei", "#3a3a3d") },
    ]
}
