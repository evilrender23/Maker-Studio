//! Deshacer por trazo: se guardan solo las hojas del BVH que el trazo tocó (posiciones y/o
//! máscara de sus vértices, antes del cambio), con un presupuesto de memoria.
//!
//! Deshacer intercambia lo guardado con lo actual, así el mismo registro sirve para rehacer.

use glam::Vec3;
use std::collections::VecDeque;

/// Qué guarda un registro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Positions,
    Mask,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub label: String,
    pub target: Target,
    /// Hojas guardadas y dónde empieza cada una en los datos.
    pub leaves: Vec<u32>,
    pub offsets: Vec<u32>,
    pub pos: Vec<Vec3>,
    pub mask: Vec<f32>,
}

impl Record {
    pub fn new(label: &str, target: Target) -> Self {
        Self { label: label.to_string(), target, leaves: Vec::new(), offsets: Vec::new(), pos: Vec::new(), mask: Vec::new() }
    }
    pub fn bytes(&self) -> usize {
        self.leaves.len() * 8 + self.pos.len() * 12 + self.mask.len() * 4 + self.label.len()
    }
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }
}

/// Historial con presupuesto: al pasarse, se olvidan los pasos más antiguos.
#[derive(Debug)]
pub struct UndoStack {
    undo: VecDeque<Record>,
    redo: Vec<Record>,
    bytes: usize,
    pub budget: usize,
}

/// Presupuesto por defecto del historial de una escultura.
pub const DEFAULT_BUDGET: usize = 1 << 30;

impl Default for UndoStack {
    fn default() -> Self {
        Self { undo: VecDeque::new(), redo: Vec::new(), bytes: 0, budget: DEFAULT_BUDGET }
    }
}

impl UndoStack {
    pub fn with_budget(budget: usize) -> Self {
        Self { budget, ..Default::default() }
    }
    pub fn push(&mut self, r: Record) {
        for old in self.redo.drain(..) {
            self.bytes -= old.bytes();
        }
        self.bytes += r.bytes();
        self.undo.push_back(r);
        while self.bytes > self.budget && self.undo.len() > 1 {
            let old = self.undo.pop_front().unwrap();
            self.bytes -= old.bytes();
        }
    }
    pub fn pop_undo(&mut self) -> Option<Record> {
        self.undo.pop_back()
    }
    pub fn push_redo(&mut self, r: Record) {
        self.redo.push(r);
    }
    pub fn pop_redo(&mut self) -> Option<Record> {
        self.redo.pop()
    }
    /// Vuelve a la pila de deshacer sin borrar la de rehacer.
    pub fn push_undo_keep_redo(&mut self, r: Record) {
        self.undo.push_back(r);
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|r| r.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|r| r.label.as_str())
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(n: usize) -> Record {
        let mut r = Record::new("x", Target::Positions);
        r.leaves.push(0);
        r.offsets.push(0);
        r.pos = vec![Vec3::ZERO; n];
        r
    }

    #[test]
    fn budget_drops_oldest_but_keeps_last() {
        let mut u = UndoStack::with_budget(12 * 250);
        for _ in 0..10 {
            u.push(rec(100));
        }
        assert!(u.bytes() <= 12 * 250);
        assert_eq!(u.len(), 2);
        let mut big = UndoStack::with_budget(10);
        big.push(rec(100));
        assert_eq!(big.len(), 1, "el último paso se queda aunque no quepa");
    }

    #[test]
    fn new_step_clears_redo() {
        let mut u = UndoStack::default();
        u.push(rec(1));
        let r = u.pop_undo().unwrap();
        u.push_redo(r);
        assert!(u.can_redo());
        u.push(rec(1));
        assert!(!u.can_redo());
    }
}
