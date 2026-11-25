#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
mod shield;

use eframe::egui;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use wry::{NewWindowResponse, PermissionResponse, WebView, WebViewBuilder};

type ShieldCallback = Box<dyn FnOnce(Result<(), String>)>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlayerMode {
    Stream,
    Trailer,
}

pub struct EmbeddedPlayer {
    webview: WebView,
    #[cfg(target_os = "macos")]
    _clip_container: platform::ClipContainer,
    shield_status: Rc<RefCell<Option<Result<(), String>>>>,
    started_at: Instant,
    current_url: String,
    geometry: Option<(egui::Rect, egui::Rect, f32)>,
    visible: bool,
    mode: PlayerMode,
}

impl EmbeddedPlayer {
    pub fn new(
        frame: &eframe::Frame,
        rect: egui::Rect,
        ctx: &egui::Context,
        mode: PlayerMode,
    ) -> Result<Option<Self>, String> {
        let builder = WebViewBuilder::new();
        #[cfg(target_os = "windows")]
        let builder = {
            use wry::WebViewBuilderExtWindows;

            let Some(environment) = platform::prepare(ctx)? else {
                return Ok(None);
            };
            builder.with_environment(environment)
        };
        let mut builder = builder
            .with_bounds(native_bounds(rect))
            .with_visible(false)
            .with_focused(false)
            .with_background_color((0, 0, 0, 255))
            .with_incognito(true)
            .with_autoplay(mode == PlayerMode::Trailer)
            .with_navigation_handler(move |url| match mode {
                PlayerMode::Stream => shield::allows_navigation(&url),
                PlayerMode::Trailer => url == "about:blank" || allowed_trailer(&url),
            })
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_permission_handler(|_| PermissionResponse::Deny);
        if mode == PlayerMode::Stream {
            builder = builder.with_initialization_script_for_main_only(shield::PAGE_SCRIPT, false);
        }

        #[cfg(target_os = "linux")]
        platform::initialize(frame)?;
        let webview = builder
            .build_as_child(frame)
            .map_err(|error| format!("Could not open the embedded player. {error}"))?;
        #[cfg(target_os = "macos")]
        let clip_container = platform::ClipContainer::new(&webview)?;

        let shield_status = Rc::new(RefCell::new(
            (mode == PlayerMode::Trailer).then_some(Ok(())),
        ));
        if mode == PlayerMode::Stream {
            let report = Rc::clone(&shield_status);
            let repaint = ctx.clone();
            platform::install_shield(
                &webview,
                Box::new(move |result| {
                    *report.borrow_mut() = Some(result);
                    repaint.request_repaint();
                }),
            );
        }
        Ok(Some(Self {
            webview,
            #[cfg(target_os = "macos")]
            _clip_container: clip_container,
            shield_status,
            started_at: Instant::now(),
            current_url: String::new(),
            geometry: None,
            visible: false,
            mode,
        }))
    }

    pub fn update(
        &mut self,
        rect: egui::Rect,
        clip_rect: egui::Rect,
        source: &str,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            platform::pump_events();
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        let native_scale = ctx
            .input(|input| input.viewport().native_pixels_per_point)
            .unwrap_or(1.0);
        let snap = |point: egui::Pos2| (point.to_vec2() * native_scale).round() / native_scale;
        let rect = egui::Rect::from_min_size(
            snap(rect.min).to_pos2(),
            (rect.size() * native_scale).round() / native_scale,
        );
        let clip = rect.intersect(egui::Rect::from_min_max(
            snap(clip_rect.min).to_pos2(),
            snap(clip_rect.max).to_pos2(),
        ));
        let visible = clip.is_positive();
        let geometry = (rect, clip, native_scale);
        if visible && self.geometry != Some(geometry) {
            #[cfg(not(target_os = "macos"))]
            {
                let previous = self.geometry;
                if previous.is_none_or(|(old, _, scale)| {
                    (old.size() * scale).round() != (rect.size() * native_scale).round()
                        || scale != native_scale
                }) {
                    self.webview
                        .set_bounds(native_bounds(rect))
                        .map_err(|error| error.to_string())?;
                } else if previous.is_some_and(|(old, _, _)| old.min != rect.min) {
                    platform::move_player(&self.webview, rect.min, native_scale)?;
                }
                let local_clip = clip.translate(-rect.min.to_vec2());
                if previous.is_none_or(|(old, old_clip, scale)| {
                    snap(old_clip.min - old.min.to_vec2()) != snap(local_clip.min)
                        || snap(old_clip.max - old.min.to_vec2()) != snap(local_clip.max)
                        || scale != native_scale
                }) {
                    platform::clip(&self.webview, local_clip, native_scale)?;
                }
            }
            self.geometry = Some(geometry);
        }
        #[cfg(target_os = "macos")]
        if visible {
            self._clip_container.layout(&self.webview, rect, clip);
        }
        if self.visible != visible {
            self.webview
                .set_visible(visible)
                .map_err(|error| error.to_string())?;
            self.visible = visible;
        }
        match self.shield_status.borrow().as_ref() {
            Some(Ok(())) => {}
            Some(Err(error)) => return Err(format!("Vortex Shield could not start. {error}")),
            None if self.started_at.elapsed() > Duration::from_secs(10) => {
                return Err("Vortex Shield took too long to start.".to_owned());
            }
            None => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return Ok(());
            }
        }
        if self.current_url != source {
            let result = if self.mode == PlayerMode::Trailer {
                use wry::http::{HeaderMap, HeaderValue, header::REFERER};
                let mut headers = HeaderMap::new();
                headers.insert(
                    REFERER,
                    HeaderValue::from_static("https://me.watchvortex.desktop/"),
                );
                self.webview.load_url_with_headers(source, headers)
            } else {
                self.webview.load_url(source)
            };
            result.map_err(|error| format!("Could not load the video. {error}"))?;
            self.current_url = source.to_owned();
        }
        Ok(())
    }

    pub fn ready(&self) -> bool {
        !self.current_url.is_empty()
    }
}

fn allowed_trailer(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("www.youtube.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.path().strip_prefix("/embed/").is_some_and(|key| {
                key.len() == 11
                    && key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
    })
}

fn native_bounds(rect: egui::Rect) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(rect.left() as f64, rect.top() as f64).into(),
        size: wry::dpi::LogicalSize::new(rect.width() as f64, rect.height() as f64).into(),
    }
}
