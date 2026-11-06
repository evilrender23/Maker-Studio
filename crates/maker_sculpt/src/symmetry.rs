//! Simetría: espejos X/Y/Z combinables y copias radiales alrededor de un eje.
//!
//! Cada *dab* se repite con cada transformación de la lista (la identidad la primera), alrededor
//! de un centro: el origen de la malla o el centro de su caja («Local»).

use glam::{Mat3, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
    pub fn vec(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

/// Copias radiales permitidas.
pub const RADIAL_MIN: u32 = 2;
pub const RADIAL_MAX: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symmetry {
    /// Espejo en X, Y y Z (planos YZ, XZ y XY).
    pub mirror: [bool; 3],
    pub radial: bool,
    pub radial_count: u32,
    pub radial_axis: Axis,
    /// Centro en el de la caja de la malla en vez del origen.
    pub local: bool,
}

impl Default for Symmetry {
    fn default() -> Self {
        Self { mirror: [true, false, false], radial: false, radial_count: 8, radial_axis: Axis::Y, local: false }
    }
}

impl Symmetry {
    /// Transformaciones lineales de las copias (ortogonales), la identidad primero.
    pub fn transforms(&self, out: &mut Vec<Mat3>) {
        out.clear();
        let n = if self.radial { self.radial_count.clamp(RADIAL_MIN, RADIAL_MAX) } else { 1 };
        let axis = self.radial_axis.vec();
        for k in 0..n {
            let rot = Mat3::from_axis_angle(axis, k as f32 / n as f32 * std::f32::consts::TAU);
            for bits in 0..8u32 {
                if (0..3).any(|i| bits & (1 << i) != 0 && !self.mirror[i]) {
                    continue;
                }
                let s = Vec3::new(if bits & 1 != 0 { -1.0 } else { 1.0 }, if bits & 2 != 0 { -1.0 } else { 1.0 }, if bits & 4 != 0 { -1.0 } else { 1.0 });
                out.push(rot * Mat3::from_diagonal(s));
            }
        }
    }

    /// Número de copias de cada *dab* (incluida la original).
    pub fn count(&self) -> usize {
        let mut v = Vec::new();
        self.transforms(&mut v);
        v.len()
    }
}

/// Punto transformado alrededor del centro.
#[inline]
pub fn apply(m: &Mat3, center: Vec3, p: Vec3) -> Vec3 {
    center + *m * (p - center)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_mirror_and_radial() {
        let mut s = Symmetry { mirror: [false; 3], ..Default::default() };
        assert_eq!(s.count(), 1);
        s.mirror = [true, false, false];
        assert_eq!(s.count(), 2);
        s.mirror = [true, false, true];
        assert_eq!(s.count(), 4);
        s.mirror = [true; 3];
        assert_eq!(s.count(), 8);
        s.mirror = [false; 3];
        s.radial = true;
        s.radial_count = 8;
        assert_eq!(s.count(), 8);
        s.mirror = [true, false, false];
        assert_eq!(s.count(), 16, "8 radiales con X dan 16");
        s.radial_count = 1000;
        s.mirror = [false; 3];
        assert_eq!(s.count(), RADIAL_MAX as usize);
    }

    #[test]
    fn radial_copies_are_evenly_spaced() {
        let s = Symmetry { mirror: [false; 3], radial: true, radial_count: 6, radial_axis: Axis::Z, local: false };
        let mut v = Vec::new();
        s.transforms(&mut v);
        let pts: Vec<Vec3> = v.iter().map(|m| apply(m, Vec3::ZERO, Vec3::X)).collect();
        assert!((pts[0] - Vec3::X).length() < 1e-6);
        for i in 0..6 {
            let a = pts[i];
            let b = pts[(i + 1) % 6];
            assert!((a.distance(b) - 1.0).abs() < 1e-5, "hexágono de lado 1");
            assert!(a.z.abs() < 1e-6);
        }
    }

    #[test]
    fn mirror_about_local_center() {
        let s = Symmetry::default();
        let mut v = Vec::new();
        s.transforms(&mut v);
        let c = Vec3::new(2.0, 0.0, 0.0);
        assert_eq!(apply(&v[1], c, Vec3::new(3.0, 1.0, 1.0)), Vec3::new(1.0, 1.0, 1.0));
    }
}
