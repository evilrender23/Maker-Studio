//! Estado de entrada por fotograma (teclado y ratón) que leen los scripts.

use glam::Vec2;
pub use maker_api::Key;

#[derive(Clone, Debug)]
pub struct Input {
    down: [bool; Key::COUNT as usize],
    pressed: [bool; Key::COUNT as usize],
    pub mouse_buttons: [bool; 3],
    pub mouse_delta: Vec2,
    /// Posición del ratón en píxeles de la vista del juego (y hacia abajo).
    pub mouse_pos: Vec2,
    /// Tamaño de la vista del juego en píxeles.
    pub pantalla: Vec2,
    /// Texto escrito en este fotograma (campos de texto de la interfaz).
    pub texto: String,
}

impl Default for Input {
    fn default() -> Self {
        Self { down: [false; Key::COUNT as usize], pressed: [false; Key::COUNT as usize], mouse_buttons: [false; 3], mouse_delta: Vec2::ZERO, mouse_pos: Vec2::ZERO, pantalla: Vec2::ZERO, texto: String::new() }
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
        self.texto.clear();
    }
    /// Copia sin reservar memoria (reutiliza el texto).
    pub fn copiar_de(&mut self, o: &Input) {
        self.down = o.down;
        self.pressed = o.pressed;
        self.mouse_buttons = o.mouse_buttons;
        self.mouse_delta = o.mouse_delta;
        self.mouse_pos = o.mouse_pos;
        self.pantalla = o.pantalla;
        self.texto.clear();
        self.texto.push_str(&o.texto);
    }
    /// Suelta todas las teclas (las consume la interfaz).
    pub fn soltar_teclas(&mut self) {
        self.down = [false; Key::COUNT as usize];
        self.pressed = [false; Key::COUNT as usize];
        self.texto.clear();
    }
    pub fn release_all(&mut self) {
        let (pos, pantalla) = (self.mouse_pos, self.pantalla);
        *self = Input::default();
        self.mouse_pos = pos;
        self.pantalla = pantalla;
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
