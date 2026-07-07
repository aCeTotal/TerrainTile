/// Height samples for one tile plus an apron of `apron` samples on every
/// side, so normals along tile edges use the same source data as the
/// neighboring tile — identical edge vertices and normals, no cracks.
pub struct HeightPatch {
    /// Row-major, `(n + 2*apron + 1)^2` samples.
    pub data: Vec<f32>,
    /// Samples per tile edge (vertex grid is `n+1`).
    pub n: usize,
    pub apron: usize,
}

impl HeightPatch {
    /// Sample in tile-local units; valid range `-apron ..= n + apron`.
    #[inline]
    pub fn h(&self, i: i64, j: i64) -> f32 {
        let side = (self.n + 2 * self.apron + 1) as i64;
        let a = self.apron as i64;
        let ii = (i + a).clamp(0, side - 1);
        let jj = (j + a).clamp(0, side - 1);
        self.data[(ii * side + jj) as usize]
    }
}

/// Geometry of one LOD without materializing any vertex buffers — the
/// exporter streams attributes straight to disk, keeping RAM flat no matter
/// the tile size.
pub struct LodGeometry<'a> {
    pub patch: &'a HeightPatch,
    pub stride: usize,
    pub res: f64,
    pub overlap: bool,
}

impl LodGeometry<'_> {
    /// Vertices per edge.
    pub fn vc(&self) -> usize {
        self.patch.n / self.stride + if self.overlap { 1 } else { 0 }
    }

    pub fn vertex_count(&self) -> usize {
        self.vc() * self.vc()
    }

    pub fn index_count(&self) -> usize {
        let q = self.vc() - 1;
        q * q * 6
    }

    /// Bevy convention: x = east, y = up, z = south; tile-local origin at
    /// the north-west corner.
    #[inline]
    pub fn position(&self, i: usize, j: usize) -> [f32; 3] {
        let step = self.stride as f32 * self.res as f32;
        let (si, sj) = ((i * self.stride) as i64, (j * self.stride) as i64);
        [j as f32 * step, self.patch.h(si, sj), i as f32 * step]
    }

    #[inline]
    pub fn normal(&self, i: usize, j: usize) -> [f32; 3] {
        normal_at(
            self.patch,
            (i * self.stride) as i64,
            (j * self.stride) as i64,
            self.stride as i64,
            self.res,
        )
    }

    #[inline]
    pub fn uv(&self, i: usize, j: usize) -> [f32; 2] {
        let n = self.patch.n as f32;
        [(j * self.stride) as f32 / n, (i * self.stride) as f32 / n]
    }

    #[inline]
    pub fn tangent(&self, i: usize, j: usize) -> [f32; 4] {
        tangent_for(self.normal(i, j))
    }

    /// Indices of one quad (two CCW triangles seen from above).
    #[inline]
    pub fn quad(&self, i: usize, j: usize) -> [u32; 6] {
        let vc = self.vc() as u32;
        let a = i as u32 * vc + j as u32;
        let b = (i as u32 + 1) * vc + j as u32;
        let c = a + 1;
        let d = b + 1;
        [a, b, c, c, b, d]
    }
}

pub struct MeshStats {
    pub min_h: f32,
    pub max_h: f32,
    pub avg_slope_deg: f32,
    pub avg_normal: [f32; 3],
}

/// Normal from central differences at the given sample and stride.
#[inline]
pub fn normal_at(patch: &HeightPatch, i: i64, j: i64, stride: i64, res: f64) -> [f32; 3] {
    let d = (2 * stride) as f32 * res as f32;
    let dhdx = (patch.h(i, j + stride) - patch.h(i, j - stride)) / d;
    let dhdz = (patch.h(i + stride, j) - patch.h(i - stride, j)) / d;
    normalize([-dhdx, 1.0, -dhdz])
}

/// Stats over the full-resolution tile grid (LOD0 samples).
pub fn stats(patch: &HeightPatch, res: f64) -> MeshStats {
    let n = patch.n as i64;
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    let mut slope_sum = 0.0f64;
    let mut nsum = [0.0f64; 3];
    let mut count = 0usize;
    for i in 0..=n {
        for j in 0..=n {
            let h = patch.h(i, j);
            min_h = min_h.min(h);
            max_h = max_h.max(h);
            let nrm = normal_at(patch, i, j, 1, res);
            slope_sum += (nrm[1].clamp(-1.0, 1.0) as f64).acos().to_degrees();
            for k in 0..3 {
                nsum[k] += nrm[k] as f64;
            }
            count += 1;
        }
    }
    let avg = normalize([
        (nsum[0] / count as f64) as f32,
        (nsum[1] / count as f64) as f32,
        (nsum[2] / count as f64) as f32,
    ]);
    MeshStats {
        min_h,
        max_h,
        avg_slope_deg: (slope_sum / count as f64) as f32,
        avg_normal: avg,
    }
}

#[inline]
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Tangent along +x (the UV u-direction), orthogonalized against the normal.
#[inline]
fn tangent_for(n: [f32; 3]) -> [f32; 4] {
    let t = normalize([1.0 - n[0] * n[0], -n[0] * n[1], -n[0] * n[2]]);
    [t[0], t[1], t[2], 1.0]
}
