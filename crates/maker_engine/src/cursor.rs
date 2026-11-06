//! Captura del ratón mientras se juega (Play del editor y `maker-player`).
//!
//! Lógica pura, sin ventana: el editor y el reproductor la alimentan con los
//! eventos (Play, Esc, foco, clics, peticiones de los scripts) y aplican a la
//! ventana el modo que devuelve [`CursorCapture::cambio`]. Funciona como en
//! Unity y Unreal: Play captura, Esc suelta sin parar el juego, un clic en el
//! juego recaptura y perder el foco suelta.

pub use maker_api::CursorMode;

#[derive(Clone, Debug)]
pub struct CursorCapture {
    /// Hay un juego en marcha.
    activo: bool,
    /// El jugador no ha soltado el ratón (Esc, foco, clic fuera).
    sujeto: bool,
    /// La ventana tiene el foco del sistema.
    foco: bool,
    /// Modo del proyecto cuando ningún script pide otro.
    por_defecto: CursorMode,
    /// Último modo pedido por un script.
    script: Option<CursorMode>,
    /// El clic que recapturó no llega al juego hasta soltar los botones.
    tragar_clic: bool,
    /// Modo aplicado a la ventana.
    aplicado: CursorMode,
}

impl Default for CursorCapture {
    fn default() -> Self {
        CursorCapture { activo: false, sujeto: false, foco: false, por_defecto: CursorMode::Bloqueado, script: None, tragar_clic: false, aplicado: CursorMode::Libre }
    }
}

impl CursorCapture {
    /// Empieza a jugar. `capturar` es el ajuste del proyecto; `con_foco` dice
    /// si el juego tiene la entrada (en el editor, la vista de juego).
    pub fn iniciar(&mut self, capturar: bool, con_foco: bool) {
        self.activo = true;
        self.por_defecto = if capturar { CursorMode::Bloqueado } else { CursorMode::Libre };
        self.script = None;
        self.sujeto = con_foco;
        self.tragar_clic = false;
    }

    /// Termina el juego: el ratón queda libre.
    pub fn parar(&mut self) {
        self.activo = false;
        self.sujeto = false;
        self.script = None;
        self.tragar_clic = false;
    }

    /// Modo que quiere el juego ahora mismo (script o proyecto).
    pub fn deseado(&self) -> CursorMode {
        self.script.unwrap_or(self.por_defecto)
    }

    /// Modo efectivo de la ventana.
    pub fn modo(&self) -> CursorMode {
        if self.activo && self.sujeto && self.foco { self.deseado() } else { CursorMode::Libre }
    }

    /// El ratón está capturado (confinado o bloqueado).
    pub fn capturado(&self) -> bool {
        self.modo() != CursorMode::Libre
    }

    /// Suelta el ratón (clic fuera de la vista de juego). El juego sigue.
    pub fn soltar(&mut self) {
        self.sujeto = false;
        self.tragar_clic = false;
    }

    /// Esc o Mayús+F1: suelta el ratón si estaba capturado. Devuelve `true`
    /// si la tecla se usó para eso y no debe llegar al juego.
    pub fn tecla_soltar(&mut self) -> bool {
        let c = self.capturado();
        if c {
            self.soltar();
        }
        c
    }

    /// Estado del foco de la ventana, cada fotograma. Perderlo suelta el ratón.
    pub fn foco_ventana(&mut self, foco: bool) {
        if self.foco && !foco {
            self.soltar();
        }
        self.foco = foco;
    }

    /// Clic dentro del juego. Devuelve `true` si recapturó el ratón; ese
    /// clic no se pasa al juego (no dispara al volver a la partida).
    pub fn clic_en_juego(&mut self) -> bool {
        if !self.activo || self.sujeto {
            return false;
        }
        self.sujeto = true;
        let recaptura = self.capturado();
        self.tragar_clic = recaptura;
        recaptura
    }

    /// Modo pedido por los scripts tras cada fotograma. Solo un cambio de
    /// petición captura al momento (si `con_foco`); repetir la misma cada
    /// fotograma no anula un Esc del jugador.
    pub fn modo_script(&mut self, pedido: Option<CursorMode>, con_foco: bool) {
        if pedido == self.script || !self.activo {
            return;
        }
        self.script = pedido;
        if con_foco && pedido.is_some_and(|m| m != CursorMode::Libre) {
            self.sujeto = true;
        }
    }

    /// Botones que ve el juego: el clic de recaptura se descarta hasta soltarlo.
    pub fn botones(&mut self, crudos: [bool; 3]) -> [bool; 3] {
        if self.tragar_clic {
            if crudos.iter().any(|b| *b) {
                return [false; 3];
            }
            self.tragar_clic = false;
        }
        crudos
    }

    /// Nuevo modo que aplicar a la ventana, si cambió desde la última vez.
    pub fn cambio(&mut self) -> Option<CursorMode> {
        let m = self.modo();
        (m != self.aplicado).then(|| {
            self.aplicado = m;
            m
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CursorMode::*;

    fn jugando() -> CursorCapture {
        let mut c = CursorCapture::default();
        c.foco_ventana(true);
        c.iniciar(true, true);
        c
    }

    #[test]
    fn play_locks_and_stop_frees() {
        let mut c = jugando();
        assert_eq!(c.cambio(), Some(Bloqueado));
        assert_eq!(c.cambio(), None, "solo se aplica una vez");
        c.parar();
        assert_eq!(c.cambio(), Some(Libre));
        assert!(!c.capturado());
    }

    #[test]
    fn play_without_game_focus_waits_for_click() {
        let mut c = CursorCapture::default();
        c.foco_ventana(true);
        c.iniciar(true, false);
        assert_eq!(c.modo(), Libre);
        assert!(c.clic_en_juego());
        assert_eq!(c.modo(), Bloqueado);
    }

    #[test]
    fn escape_releases_once_without_stopping() {
        let mut c = jugando();
        assert!(c.tecla_soltar(), "el primer Esc suelta el ratón");
        assert_eq!(c.modo(), Libre);
        assert!(!c.tecla_soltar(), "el segundo Esc es para el juego");
        assert!(c.clic_en_juego(), "un clic recaptura");
        assert_eq!(c.modo(), Bloqueado);
        assert!(!c.clic_en_juego(), "ya capturado: el clic es del juego");
    }

    #[test]
    fn recapture_click_is_swallowed_until_released() {
        let mut c = jugando();
        c.tecla_soltar();
        assert!(c.clic_en_juego());
        assert_eq!(c.botones([true, false, false]), [false; 3]);
        assert_eq!(c.botones([true, false, false]), [false; 3]);
        assert_eq!(c.botones([false; 3]), [false; 3]);
        assert_eq!(c.botones([true, false, false]), [true, false, false], "el siguiente clic sí llega");
    }

    #[test]
    fn focus_loss_releases_and_regain_does_not_recapture() {
        let mut c = jugando();
        c.foco_ventana(false);
        assert_eq!(c.modo(), Libre);
        c.foco_ventana(true);
        assert_eq!(c.modo(), Libre, "volver a la ventana no captura sin clic");
        assert!(c.clic_en_juego());
        assert_eq!(c.modo(), Bloqueado);
    }

    #[test]
    fn capture_waits_for_initial_window_focus() {
        // El reproductor arranca jugando antes de que la ventana tenga foco.
        let mut c = CursorCapture::default();
        c.iniciar(true, true);
        assert_eq!(c.cambio(), None);
        c.foco_ventana(true);
        assert_eq!(c.cambio(), Some(Bloqueado));
    }

    #[test]
    fn project_setting_off_keeps_cursor_free() {
        let mut c = CursorCapture::default();
        c.foco_ventana(true);
        c.iniciar(false, true);
        assert_eq!(c.modo(), Libre);
        assert!(!c.clic_en_juego(), "nada que recapturar: el clic es del juego");
        assert!(!c.tecla_soltar(), "Esc llega al juego");
    }

    #[test]
    fn scripts_choose_mode_and_escape_still_wins() {
        let mut c = jugando();
        c.modo_script(Some(Libre), true);
        assert_eq!(c.modo(), Libre, "menú del juego");
        c.modo_script(Some(Confinado), true);
        assert_eq!(c.modo(), Confinado);
        assert!(c.tecla_soltar());
        c.modo_script(Some(Confinado), true);
        assert_eq!(c.modo(), Libre, "repetir la petición no anula el Esc");
        c.modo_script(Some(Bloqueado), false);
        assert_eq!(c.modo(), Libre, "sin foco del juego no captura");
        assert!(c.clic_en_juego());
        assert_eq!(c.modo(), Bloqueado);
        c.parar();
        c.iniciar(true, true);
        assert_eq!(c.deseado(), Bloqueado, "cada Play empieza con el modo del proyecto");
    }
}
