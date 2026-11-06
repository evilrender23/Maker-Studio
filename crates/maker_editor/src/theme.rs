//! Tema claro de Maker Studio, tomado de Hermes Agent Desktop (solo claro).
//!
//! Hermes define el tema con unas pocas semillas (`--theme-*`) y deriva cada
//! superficie con `color-mix()` de CSS. Estos son esos valores resueltos a sRGB
//! (extraídos de `/usr/lib/hermes-agent-desktop/app.asar`, hoja
//! `dist/assets/index-*.css`, bloque `:root`):
//!
//! | Variable de Hermes        | Semilla / mezcla                              | Aquí           |
//! |---------------------------|-----------------------------------------------|----------------|
//! | `--ui-bg-chrome`          | `#f8faff` 92 % + `#f3f3f3`                     | `CHROME`       |
//! | `--ui-bg-sidebar`         | `#f3f7ff`                                      | `SIDEBAR`      |
//! | `--ui-bg-editor`          | `#fff` 22 % + `#fcfcfc`                        | `CARD`         |
//! | `--ui-bg-input`           | `#fcfcfc`                                      | `INPUT`        |
//! | `--ui-base` (texto)       | `#17171a` al 94/74/54/36 %                     | `TEXT1..4`     |
//! | `--theme-primary`         | `#0053fd`                                      | `ACCENT`       |
//! | `--theme-accent-soft`     | `#e6eeff`                                      | `ACCENT_SOFT`  |
//! | `--ui-stroke-*`           | acento 24/16/10/6 % sobre base 10/7/5/3 %      | `STROKE1..4`   |
//! | `--ui-row-hover/active`   | acento 4/8 % sobre base 3/5 %                  | `ROW_*`        |
//! | `--ui-control-hover`      | acento 6 % sobre base 4 %                      | `CONTROL_HOVER`|
//! | `--ui-red/green/yellow…`  | `#cf2d56` `#1f8a65` `#c08532` `#4c7f8c`        | `RED`…         |
//! | `--radius-*`              | `0.75rem × 0.2` ≈ 2–3 px                       | `RADIUS`       |
//! | código                    | Shiki `github-light-default`                   | `code_palette` |

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals};

pub const CHROME: Color32 = Color32::from_rgb(0xf8, 0xf9, 0xfe);
pub const SIDEBAR: Color32 = Color32::from_rgb(0xf3, 0xf7, 0xff);
pub const CARD: Color32 = Color32::from_rgb(0xfd, 0xfd, 0xfd);
pub const INPUT: Color32 = Color32::from_rgb(0xfc, 0xfc, 0xfc);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x53, 0xfd);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xe6, 0xee, 0xff);
pub const BUBBLE: Color32 = Color32::from_rgb(0xf0, 0xf5, 0xff);
pub const TEXT1: Color32 = Color32::from_rgb(0x25, 0x25, 0x28);
pub const TEXT2: Color32 = Color32::from_rgb(0x53, 0x53, 0x55);
pub const TEXT3: Color32 = Color32::from_rgb(0x81, 0x81, 0x82);
pub const TEXT4: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xab);
pub const STROKE1: Color32 = Color32::from_rgb(0xab, 0xc0, 0xec);
pub const STROKE2: Color32 = Color32::from_rgb(0xc3, 0xd1, 0xf0);
pub const STROKE3: Color32 = Color32::from_rgb(0xd5, 0xdf, 0xf4);
pub const STROKE4: Color32 = Color32::from_rgb(0xe2, 0xe9, 0xf8);
pub const ROW_HOVER: Color32 = Color32::from_rgb(0xe7, 0xec, 0xf7);
pub const ROW_ACTIVE: Color32 = Color32::from_rgb(0xd9, 0xe2, 0xf3);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0xe0, 0xe7, 0xf5);
#[allow(dead_code)]
pub const FILL_TERTIARY: Color32 = Color32::from_rgb(0xde, 0xe5, 0xf2);
pub const RED: Color32 = Color32::from_rgb(0xcf, 0x2d, 0x56);
pub const ORANGE: Color32 = Color32::from_rgb(0xdb, 0x70, 0x4b);
pub const YELLOW: Color32 = Color32::from_rgb(0xc0, 0x85, 0x32);
pub const GREEN: Color32 = Color32::from_rgb(0x1f, 0x8a, 0x65);
pub const CYAN: Color32 = Color32::from_rgb(0x4c, 0x7f, 0x8c);
#[allow(dead_code)]
pub const PURPLE: Color32 = Color32::from_rgb(0x9e, 0x94, 0xd5);
#[allow(dead_code)]
pub const WARM: Color32 = Color32::from_rgb(0xcf, 0x80, 0x6d);
/// `--ui-selection-background` (#ffd24a al 55 %).
#[allow(dead_code)]
pub const SELECTION: Color32 = Color32::from_rgba_premultiplied(0x8c, 0x73, 0x29, 0x8c);

/// Ejes del gizmo (rojo X, verde Y, azul Z) en la paleta del tema.
pub const AXIS_X: Color32 = RED;
pub const AXIS_Y: Color32 = GREEN;
pub const AXIS_Z: Color32 = ACCENT;

pub const RADIUS: u8 = 3;

/// Colores del editor de código (Shiki `github-light-default`).
pub struct CodePalette {
    pub text: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub function: Color32,
    pub type_: Color32,
    pub macro_: Color32,
    pub attribute: Color32,
}

pub fn code_palette() -> CodePalette {
    CodePalette {
        text: Color32::from_rgb(0x1f, 0x23, 0x28),
        keyword: Color32::from_rgb(0xcf, 0x22, 0x2e),
        string: Color32::from_rgb(0x0a, 0x30, 0x69),
        number: Color32::from_rgb(0x05, 0x50, 0xae),
        comment: Color32::from_rgb(0x59, 0x63, 0x6e),
        function: Color32::from_rgb(0x82, 0x50, 0xdf),
        type_: Color32::from_rgb(0x95, 0x38, 0x00),
        macro_: Color32::from_rgb(0x05, 0x50, 0xae),
        attribute: Color32::from_rgb(0x11, 0x63, 0x29),
    }
}

fn visuals() -> Visuals {
    let mut v = Visuals::light();
    let r = CornerRadius::same(RADIUS);
    v.override_text_color = None;
    v.weak_text_color = Some(TEXT3);
    v.hyperlink_color = ACCENT;
    v.panel_fill = CHROME;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, STROKE2);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(5);
    // --shadow-md: 0 .25rem 1rem #00000014, 0 1rem 2rem -1.5rem #0000002e
    v.window_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(0x14) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(0x14) };
    v.faint_bg_color = BUBBLE;
    v.extreme_bg_color = INPUT;
    v.code_bg_color = Color32::from_rgb(0xf0, 0xf2, 0xf6);
    v.text_edit_bg_color = Some(INPUT);
    v.warn_fg_color = YELLOW;
    v.error_fg_color = RED;
    v.selection.bg_fill = Color32::from_rgb(0xcc, 0xdc, 0xff);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    v.striped = false;
    v.indent_has_left_vline = true;
    v.collapsing_header_frame = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CHROME;
    w.noninteractive.weak_bg_fill = CHROME;
    w.noninteractive.bg_stroke = Stroke::new(1.0, STROKE3);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT1);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = Color32::from_rgb(0xec, 0xf0, 0xfa);
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = Stroke::new(1.0, STROKE3);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT1);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = CONTROL_HOVER;
    w.hovered.weak_bg_fill = ROW_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, STROKE1);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT1);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = ROW_ACTIVE;
    w.active.weak_bg_fill = ROW_ACTIVE;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT1);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.bg_fill = ACCENT_SOFT;
    w.open.weak_bg_fill = ACCENT_SOFT;
    w.open.bg_stroke = Stroke::new(1.0, STROKE1);
    w.open.fg_stroke = Stroke::new(1.0, TEXT1);
    w.open.corner_radius = r;
    v
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals());
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.all_styles_mut(|s| {
        // Hermes: texto 13 px, leyendas 11–12 px, radios de 2–3 px, filas compactas.
        s.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(10);
        s.spacing.interact_size.y = 22.0;
        s.spacing.indent = 14.0;
    });
}

/// Hermes usa Segoe/SF y JetBrains Mono; en Linux la equivalente instalada
/// suele ser Noto Sans (si no está, se queda la fuente de egui).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut changed = false;
    let sans = [
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
        "C:/Windows/Fonts/segoeui.ttf",
    ];
    if let Some(bytes) = sans.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("ui-sans".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui-sans".into());
        changed = true;
    }
    // Respaldo para símbolos (■ ☀ ▶ ✕ ◉ ◆) que Noto Sans no trae.
    let symbols = [
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "C:/Windows/Fonts/seguisym.ttf",
    ];
    if let Some(bytes) = symbols.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("ui-symbols".into(), egui::FontData::from_owned(bytes).into());
        let fam = fonts.families.entry(FontFamily::Proportional).or_default();
        let at = fam.iter().position(|f| f == "ui-sans").map(|i| i + 1).unwrap_or(0);
        fam.insert(at, "ui-symbols".into());
        fonts.families.entry(FontFamily::Monospace).or_default().push("ui-symbols".into());
        changed = true;
    }
    let mono = [
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        "C:/Windows/Fonts/CascadiaMono.ttf",
    ];
    if let Some(bytes) = mono.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("ui-mono".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "ui-mono".into());
        changed = true;
    }
    if changed {
        ctx.set_fonts(fonts);
    }
}

/// Marco de panel lateral (fondo `--ui-bg-sidebar`).
pub fn sidebar_frame() -> egui::Frame {
    egui::Frame::new().fill(SIDEBAR).stroke(Stroke::new(1.0, STROKE3)).inner_margin(Margin::same(8))
}

/// Marco de tarjeta (`--ui-bg-editor`).
pub fn card_frame() -> egui::Frame {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, STROKE3)).corner_radius(CornerRadius::same(5)).inner_margin(Margin::same(10))
}

/// Encabezado de sección pequeño en mayúsculas (estilo de Hermes).
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(10.5).color(TEXT3).strong());
}
