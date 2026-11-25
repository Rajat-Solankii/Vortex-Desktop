use eframe::egui::{self, *};

pub const BACKGROUND: Color32 = Color32::BLACK;
pub const PANEL: Color32 = Color32::from_rgb(10, 10, 12);
pub const CARD: Color32 = Color32::from_rgb(16, 16, 19);
pub const BORDER: Color32 = Color32::from_rgb(36, 36, 42);
pub const TEXT: Color32 = Color32::WHITE;
pub const MUTED: Color32 = Color32::from_rgb(156, 163, 175);
pub const SOFT_TEXT: Color32 = Color32::from_rgb(209, 213, 219);
pub const VIOLET: Color32 = Color32::from_rgb(124, 77, 255);
pub const LAVENDER: Color32 = Color32::from_rgb(193, 169, 255);
pub const CYAN: Color32 = Color32::from_rgb(0, 176, 255);

pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Geist Bold".into()))
}

pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Geist Black".into()))
}

pub fn label(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(strong(size)).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).size(13.0).color(MUTED)
}

pub fn setup(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Geist Regular".into(),
        FontData::from_static(include_bytes!("../assets/geist_regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "Geist Bold".into(),
        FontData::from_static(include_bytes!("../assets/geist_bold.ttf")).into(),
    );
    fonts.font_data.insert(
        "Geist Black".into(),
        FontData::from_static(include_bytes!("../assets/geist_black.ttf")).into(),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Geist Regular".into());
    let mut bold = proportional.clone();
    let mut black = proportional.clone();
    bold[0] = "Geist Bold".into();
    black[0] = "Geist Black".into();
    fonts
        .families
        .insert(FontFamily::Name("Geist Bold".into()), bold);
    fonts
        .families
        .insert(FontFamily::Name("Geist Black".into()), black);
    ctx.set_fonts(fonts);

    let mut style = Style {
        visuals: Visuals::dark(),
        ..Default::default()
    };
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = CARD;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(59, 39, 106);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, LAVENDER);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = CornerRadius::same(8);
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
    }
    style.visuals.widgets.inactive.weak_bg_fill = CARD;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 39, 70);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(65, 47, 105);
    style.spacing.item_spacing = vec2(10.0, 9.0);
    style.spacing.button_padding = vec2(12.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.scroll.bar_width = 7.0;
    style.spacing.scroll.floating_width = 7.0;
    style.spacing.scroll.dormant_handle_opacity = 0.72;
    ctx.set_style(style);
}

pub fn button(ui: &mut Ui, text: &str, primary: bool) -> Response {
    let glow = primary.then(|| ui.painter().add(Shape::Noop));
    let response = ui.add(
        Button::new(label(text, 15.0))
            .min_size(vec2(134.0, 43.0))
            .corner_radius(24)
            .fill(if primary {
                VIOLET
            } else {
                Color32::from_white_alpha(45)
            })
            .stroke(Stroke::NONE),
    );
    if let Some(glow) = glow {
        ui.painter().set(
            glow,
            egui::epaint::RectShape::filled(
                response.rect.expand(3.0),
                27,
                VIOLET.gamma_multiply(if response.hovered() { 0.59 } else { 0.39 }),
            )
            .with_blur_width(18.0),
        );
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn gutter(width: f32) -> i8 {
    if width >= 1280.0 {
        64
    } else if width >= 1000.0 {
        40
    } else {
        24
    }
}

pub fn section_heading(ui: &mut Ui, text: &str, size: f32, accent: Color32) {
    ui.horizontal(|ui| {
        let height = size.clamp(22.0, 32.0);
        let (bar, _) = ui.allocate_exact_size(vec2(6.0, height), Sense::hover());
        ui.painter().add(
            egui::epaint::RectShape::filled(bar.expand(2.0), 4, accent.gamma_multiply(0.45))
                .with_blur_width(8.0),
        );
        ui.painter().rect_filled(bar, 3, accent);
        ui.label(label(text, size));
    });
}

pub fn cover(painter: &Painter, rect: Rect, image: &TextureHandle, radius: u8) {
    let source = image.size_vec2();
    let image_ratio = source.x / source.y;
    let target_ratio = rect.width() / rect.height();
    let uv_size = if image_ratio > target_ratio {
        vec2(target_ratio / image_ratio, 1.0)
    } else {
        vec2(1.0, image_ratio / target_ratio)
    };
    painter.add(
        egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(image.id(), Rect::from_center_size(pos2(0.5, 0.5), uv_size)),
    );
}

pub fn fade(painter: &Painter, rect: Rect, colors: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
    .into_iter()
    .zip(colors)
    {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

pub fn backdrop(painter: &Painter, rect: Rect, art: Option<&TextureHandle>) {
    painter.rect_filled(rect, 0, BACKGROUND);
    if let Some(art) = art {
        cover(painter, rect, art, 0);
    }
    let center = rect.center().x;
    let shade = Color32::from_black_alpha(200);
    fade(
        painter,
        Rect::from_min_max(rect.min, pos2(center, rect.bottom())),
        [BACKGROUND, shade, shade, BACKGROUND],
    );
    fade(
        painter,
        Rect::from_min_max(pos2(center, rect.top()), rect.max),
        [shade, Color32::TRANSPARENT, Color32::TRANSPARENT, shade],
    );
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.center().y), rect.max);
    fade(
        painter,
        bottom,
        [
            Color32::TRANSPARENT,
            Color32::TRANSPARENT,
            BACKGROUND,
            BACKGROUND,
        ],
    );
}
