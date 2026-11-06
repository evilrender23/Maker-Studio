//! Estado de entrada por fotograma (teclado y ratón) que leen los scripts.

use glam::Vec2;
pub use maker_api::Key;

#[derive(Clone, Debug)]
pub struct Input {
    down: [bool; Key::COUNT as usize],
    pressed: [bool; Key::COUNT as usize],
    pub mouse_buttons: [bool; 3],
    pub mouse_delta: Vec2,
    pub mouse_pos: Vec2,
}

impl Default for Input {
    fn default() -> Self {
        Self { down: [false; Key::COUNT as usize], pressed: [false; Key::COUNT as usize], mouse_buttons: [false; 3], mouse_delta: Vec2::ZERO, mouse_pos: Vec2::ZERO }
    }
}

impl Input {
    pub fn key_down(&self, code: u32) -> bool {
        self.down.get(code as usize).copied().unwrap_or(false)
    }
    pub fn key_pressed(&self, code: u32) -> bool {
        self.pressed.get(code as usize).copied().unwrap_or(false)
    }
    pub fn set_key(&mut self, k: Key, down: bool) {
        let i = k as usize;
        if down && !self.down[i] {
            self.pressed[i] = true;
        }
        self.down[i] = down;
    }
    /// Al final de cada fotograma: limpia los "pulsado en este fotograma".
    pub fn end_frame(&mut self) {
        self.pressed = [false; Key::COUNT as usize];
        self.mouse_delta = Vec2::ZERO;
    }
    pub fn release_all(&mut self) {
        *self = Input::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_only_once() {
        let mut i = Input::default();
        i.set_key(Key::Space, true);
        assert!(i.key_pressed(Key::Space as u32));
        i.end_frame();
        i.set_key(Key::Space, true);
        assert!(!i.key_pressed(Key::Space as u32));
        assert!(i.key_down(Key::Space as u32));
    }
}
