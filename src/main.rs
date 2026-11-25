#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod catalog;
mod player;
mod screen_cast;
mod style;

use catalog::{
    DetailInfo, Episode, Job, Kind, Network, Provider, Query, Reply, Title, Trailer, anime_catalog,
    home_catalogs, server_urls,
};
use eframe::egui::{self, *};
use player::{EmbeddedPlayer, PlayerMode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use style::*;

const SEARCH: &str = "catalog_search";
const PANEL_SEARCH: &str = "panel_search";
const SEARCH_HISTORY: &str = "recent-searches";
const APP_ID: &str = "me.watchvortex.desktop";
const LIBRARY: &str = "watchlist";
const HISTORY: &str = "watch-history";
const PREFERENCES: &str = "preferences";
const HERO_INTERVAL: Duration = Duration::from_secs(8);
const ICON: &[u8] = include_bytes!("../assets/vortex-icon.png");
const WORDMARK: &[u8] = include_bytes!("../assets/vortex-wordmark.png");

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_inner_size([1380.0, 890.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(eframe::icon_data::from_png_bytes(ICON).expect("bundled Vortex icon")),
        #[cfg(target_os = "linux")]
        event_loop_builder: if std::env::var_os("DISPLAY")
            .is_some_and(|display| !display.is_empty())
        {
            Some(Box::new(|builder| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }))
        } else {
            None
        },
        ..Default::default()
    };
    eframe::run_native(
        "Vortex",
        options,
        Box::new(|cc| Ok(Box::new(Vortex::new(cc)))),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Page {
    Home,
    Discover,
    Anime,
    Browse(Kind, u32),
    Library,
    Settings,
    Search,
    Providers,
    Provider(u32, Kind),
    Collection(usize),
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Discover => "Browse by Genre",
            Self::Anime => "Anime",
            Self::Browse(kind, genre) => genre_list(kind)
                .iter()
                .find(|(id, _)| *id == genre)
                .map(|(_, name)| *name)
                .unwrap_or(if kind == Kind::Tv {
                    "TV Shows"
                } else {
                    "Movies"
                }),
            Self::Library => "Library",
            Self::Settings => "Settings",
            Self::Search => "Search",
            Self::Providers | Self::Provider(..) => "Providers",
            Self::Collection(_) => "Collection",
        }
    }
}

enum Load<T> {
    Idle,
    Loading(u64),
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    fn accepts(&self, token: u64) -> bool {
        matches!(self, Self::Loading(active) if *active == token)
    }

    fn resolve(&mut self, result: Result<T, String>) {
        *self = match result {
            Ok(value) => Self::Ready(value),
            Err(error) => Self::Failed(error),
        };
    }
}

struct Rail {
    label: &'static str,
    query: Query,
    items: Load<(Vec<Title>, u32)>,
    offset: f32,
}

struct Listing {
    query: Query,
    page: u32,
    total_pages: u32,
    entries: Vec<Title>,
    load: Load<()>,
}

impl Listing {
    fn new(query: Query) -> Self {
        Self {
            query,
            page: 0,
            total_pages: 1,
            entries: Vec::new(),
            load: Load::Idle,
        }
    }
}

struct Detail {
    summary: Title,
    metadata: Load<Box<DetailInfo>>,
    season: Option<u32>,
    episodes: Load<Vec<Episode>>,
    recommendations: Load<Vec<Title>>,
    recommendation_offset: f32,
}

impl Detail {
    fn title(&self) -> &Title {
        match &self.metadata {
            Load::Ready(details) => &details.title,
            _ => &self.summary,
        }
    }
}

enum Art {
    Loading,
    Ready(TextureHandle),
    Failed,
}

#[derive(Default)]
struct Artwork {
    textures: HashMap<String, (Art, Instant)>,
}

impl Artwork {
    fn get(
        &mut self,
        network: &Network,
        visible: bool,
        url: Option<String>,
    ) -> Option<&TextureHandle> {
        let url = url.filter(|_| visible)?;
        if let Some((art, since)) = self.textures.get_mut(&url)
            && matches!(art, Art::Failed)
            && since.elapsed() >= Duration::from_secs(30)
        {
            network.send_image(url.clone());
            *art = Art::Loading;
        }
        if !self.textures.contains_key(&url) {
            if self.textures.len() >= 260
                && let Some(key) = self
                    .textures
                    .iter()
                    .filter(|(_, (art, _))| !matches!(art, Art::Loading))
                    .min_by_key(|(_, (_, last_used))| *last_used)
                    .map(|(key, _)| key.clone())
            {
                self.textures.remove(&key);
            }
            if self.textures.len() < 260 {
                network.send_image(url.clone());
                self.textures
                    .insert(url.clone(), (Art::Loading, Instant::now()));
            }
        }
        match self.textures.get_mut(&url) {
            Some((Art::Ready(texture), last_used)) => {
                *last_used = Instant::now();
                Some(texture)
            }
            _ => None,
        }
    }
}

struct Vortex {
    account: account::Account,
    session_open: bool,
    network: Network,
    art: Artwork,
    wordmark: TextureHandle,
    home: Vec<Rail>,
    hero: usize,
    hero_changed_at: Instant,
    page: Page,
    query: String,
    search_at: Option<Instant>,
    search_history: Vec<String>,
    listing: Option<Listing>,
    discover_sort: &'static str,
    library: Vec<Title>,
    history: Vec<HistoryEntry>,
    history_offset: f32,
    providers: Load<Vec<Provider>>,
    provider_offset: f32,
    preferences: Preferences,
    detail: Option<Detail>,
    related_backstack: Vec<Title>,
    playback: Option<Playback>,
    trailer: Option<TrailerPlayback>,
    player_fullscreen: bool,
    coming_soon: Option<(String, Option<String>)>,
    web_player: Option<EmbeddedPlayer>,
    player_surface: Option<(Rect, Rect)>,
    preferred_server: usize,
    next_token: u64,
}

struct Playback {
    episode: (u32, u32),
    servers: [String; 7],
    error: Option<String>,
}

struct TrailerPlayback {
    selected: Trailer,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct HistoryEntry {
    title: Title,
    season: u32,
    episode: u32,
    #[serde(default)]
    timestamp: u64,
}

#[derive(Deserialize, Serialize)]
struct Preferences {
    autoplay_featured: bool,
    poster_size: PosterSize,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            autoplay_featured: true,
            poster_size: PosterSize::Standard,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
enum PosterSize {
    Compact,
    Standard,
    Large,
}

impl PosterSize {
    fn width(self) -> f32 {
        match self {
            Self::Compact => 140.0,
            Self::Standard => 180.0,
            Self::Large => 220.0,
        }
    }
}

impl Vortex {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let logo = eframe::icon_data::from_png_bytes(WORDMARK).expect("bundled Vortex wordmark");
        let wordmark = cc.egui_ctx.load_texture(
            "Vortex wordmark",
            ColorImage::from_rgba_unmultiplied(
                [logo.width as usize, logo.height as usize],
                &logo.rgba,
            ),
            TextureOptions::LINEAR,
        );
        Self {
            account: account::Account::default(),
            session_open: false,
            network: Network::new(&cc.egui_ctx),
            art: Artwork::default(),
            wordmark,
            home: home_catalogs()
                .into_iter()
                .map(|(label, query)| Rail {
                    label,
                    query,
                    items: Load::Idle,
                    offset: 0.0,
                })
                .collect(),
            hero: 0,
            hero_changed_at: Instant::now(),
            page: Page::Home,
            query: String::new(),
            search_at: None,
            search_history: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, SEARCH_HISTORY))
                .unwrap_or_default(),
            listing: None,
            discover_sort: "popularity.desc",
            library: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, LIBRARY))
                .unwrap_or_default(),
            history: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, HISTORY))
                .unwrap_or_default(),
            history_offset: 0.0,
            providers: Load::Idle,
            provider_offset: 0.0,
            preferences: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, PREFERENCES))
                .unwrap_or_default(),
            detail: None,
            related_backstack: Vec::new(),
            playback: None,
            trailer: None,
            player_fullscreen: false,
            coming_soon: None,
            web_player: None,
            player_surface: None,
            preferred_server: 0,
            next_token: 0,
        }
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn load_home(&mut self) {
        for index in 0..self.home.len() {
            if matches!(self.home[index].items, Load::Idle | Load::Failed(_)) {
                let token = self.token();
                let rail = &mut self.home[index];
                rail.items = Load::Loading(token);
                self.network
                    .send_background(Job::Catalog(token, rail.query.clone()));
            }
        }
    }

    fn load_providers(&mut self) {
        let token = self.token();
        self.providers = Load::Loading(token);
        self.network.send(Job::Providers(token));
    }

    fn navigate(&mut self, page: Page) {
        self.stop_playback();
        self.coming_soon = None;
        self.detail = None;
        self.related_backstack.clear();
        if self.page == page {
            return;
        }
        self.page = page;
        self.hero_changed_at = Instant::now();
        self.search_at = None;
        self.listing = None;
        if page != Page::Search {
            self.query.clear();
        }
        match page {
            Page::Browse(kind, genre) => self.load_browse(kind, genre),
            Page::Anime => self.load_anime(),
            Page::Provider(id, kind) => self.start_listing(
                Query::new(format!("discover/{}", kind.path()))
                    .with("with_watch_providers", id)
                    .with("watch_region", "US")
                    .with("include_adult", false),
            ),
            Page::Collection(index) => self.start_listing(self.home[index].query.clone()),
            Page::Search => self.load_search(),
            _ => {}
        }
    }

    fn start_listing(&mut self, query: Query) {
        self.listing = Some(Listing::new(query));
        self.next_page();
    }

    fn next_page(&mut self) {
        let token = self.token();
        if let Some(list) = &mut self.listing
            && list.page < list.total_pages
            && !matches!(list.load, Load::Loading(_))
        {
            list.load = Load::Loading(token);
            self.network.send(Job::Catalog(
                token,
                list.query.clone().with("page", list.page + 1),
            ));
        }
    }

    fn load_browse(&mut self, kind: Kind, genre: u32) {
        let mut query =
            Query::new(format!("discover/{}", kind.path())).with("sort_by", self.discover_sort);
        if genre != 0 {
            query = query.with("with_genres", genre);
        }
        if self.discover_sort == "vote_average.desc" {
            query = query.with("vote_count.gte", 300);
        }
        self.start_listing(query);
    }

    fn load_anime(&mut self) {
        let mut query = anime_catalog().with("sort_by", self.discover_sort);
        if self.discover_sort == "vote_average.desc" {
            query = query.with("vote_count.gte", 100);
        }
        self.start_listing(query);
    }

    fn load_search(&mut self) {
        let query = self.query.trim();
        if query.is_empty() {
            self.listing = None;
        } else {
            self.start_listing(
                Query::new("search/multi")
                    .with("query", query)
                    .with("include_adult", false),
            );
        }
    }

    fn begin_search(&mut self) {
        if self.page != Page::Search {
            self.navigate(Page::Search);
        } else if self.detail.is_some() {
            self.stop_playback();
            self.detail = None;
            self.related_backstack.clear();
        }
    }

    fn remember_search(&mut self) {
        let query = self.query.trim();
        if query.is_empty() {
            return;
        }
        let key = query.to_lowercase();
        self.search_history
            .retain(|entry| entry.to_lowercase() != key);
        self.search_history.insert(0, query.to_owned());
        self.search_history.truncate(15);
    }

    fn search_input(
        &mut self,
        ui: &mut Ui,
        field: Id,
        width: f32,
        height: f32,
        hint: &str,
    ) -> Response {
        let focused = ui.memory(|memory| memory.has_focus(field));
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter();
        painter.rect(
            rect,
            24,
            CARD,
            Stroke::new(1.0_f32, if focused { VIOLET } else { BORDER }),
            StrokeKind::Inside,
        );
        let center = pos2(rect.left() + 17.0, rect.center().y - 1.0);
        let stroke = Stroke::new(1.5_f32, MUTED);
        painter.circle_stroke(center, 5.0, stroke);
        painter.line_segment([center + vec2(3.5, 3.5), center + vec2(7.0, 7.0)], stroke);
        let mut field_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(field)
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let response = TextEdit::singleline(&mut self.query)
            .id(field)
            .font(FontId::proportional(15.0))
            .frame(false)
            .desired_width((width - 50.0).max(25.0))
            .margin(Margin {
                left: 35,
                right: 10,
                top: 8,
                bottom: 8,
            })
            .hint_text(RichText::new(hint).color(MUTED))
            .show(&mut field_ui)
            .response;
        if response.changed() {
            self.stop_playback();
            self.detail = None;
            self.related_backstack.clear();
            self.page = Page::Search;
            self.listing = None;
            if self.query.trim().is_empty() {
                self.search_at = None;
            } else {
                self.search_at = Some(Instant::now() + Duration::from_millis(250));
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        } else if field == Id::new(SEARCH) && response.clicked() {
            self.begin_search();
        }
        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            self.begin_search();
            self.remember_search();
            if self.search_at.take().is_some() || self.listing.is_none() {
                self.load_search();
            }
        }
        response
    }

    fn open_title(&mut self, title: Title) {
        self.related_backstack.clear();
        self.show_title(title);
    }

    fn show_title(&mut self, title: Title) {
        self.stop_playback();
        self.coming_soon = premiere_notice(&title);
        let token = self.token();
        self.network
            .send(Job::Details(token, title.kind(), title.id));
        self.detail = Some(Detail {
            summary: title,
            metadata: Load::Loading(token),
            season: None,
            episodes: Load::Idle,
            recommendations: Load::Idle,
            recommendation_offset: 0.0,
        });
        self.load_recommendations();
    }

    fn load_recommendations(&mut self) {
        let token = self.token();
        if let Some(detail) = &mut self.detail {
            detail.recommendations = Load::Loading(token);
            self.network.send(Job::Related(
                token,
                detail.summary.kind(),
                detail.summary.id,
            ));
        }
    }

    fn back_from_title(&mut self) {
        self.stop_playback();
        if let Some(previous) = self.related_backstack.pop() {
            self.show_title(previous);
        } else {
            self.detail = None;
        }
    }

    fn select_season(&mut self, season: u32) {
        let token = self.token();
        if let Some(detail) = &mut self.detail {
            detail.season = Some(season);
            detail.episodes = Load::Loading(token);
            self.network
                .send(Job::Episodes(token, detail.summary.id, season));
        }
    }

    fn receive(&mut self, ctx: &Context) {
        while let Ok(reply) = self.network.replies.try_recv() {
            match reply {
                Reply::Providers(token, result) => {
                    if self.providers.accepts(token) {
                        self.providers.resolve(result);
                    }
                }
                Reply::Related(token, result) => {
                    if let Some(detail) = &mut self.detail
                        && detail.recommendations.accepts(token)
                    {
                        detail.recommendations.resolve(result);
                    }
                }
                Reply::Catalog(token, result) => {
                    if let Some(rail) = self.home.iter_mut().find(|rail| rail.items.accepts(token))
                    {
                        rail.items.resolve(result);
                    } else if let Some(list) = &mut self.listing
                        && list.load.accepts(token)
                    {
                        match result {
                            Ok((titles, total)) => {
                                list.page += 1;
                                list.total_pages = total;
                                for title in titles {
                                    if !list.entries.iter().any(|item| item.key() == title.key()) {
                                        list.entries.push(title);
                                    }
                                }
                                list.load = Load::Ready(());
                            }
                            Err(error) => list.load = Load::Failed(error),
                        }
                    }
                }
                Reply::Details(token, result) => {
                    if let Some(detail) = &mut self.detail
                        && detail.metadata.accepts(token)
                    {
                        // A details response can omit the type used in a mixed catalog.
                        detail.metadata.resolve(result.map(|mut details| {
                            details.title.media_type = Some(detail.summary.kind());
                            details
                        }));
                        let premiere = if detail.summary.upcoming() {
                            None
                        } else {
                            premiere_notice(detail.title())
                        };
                        let seasons = &detail.title().seasons;
                        let resumed = self.playback.as_ref().map(|playback| playback.episode.0);
                        let season = seasons
                            .iter()
                            .find(|s| Some(s.season_number) == resumed && s.episode_count > 0)
                            .or_else(|| {
                                seasons
                                    .iter()
                                    .find(|s| s.season_number > 0 && s.episode_count > 0)
                            })
                            .or_else(|| seasons.iter().find(|s| s.episode_count > 0))
                            .map(|s| s.season_number);
                        if let Some(season) = season {
                            self.select_season(season);
                        }
                        if let Some(premiere) = premiere {
                            self.stop_playback();
                            self.coming_soon = Some(premiere);
                        }
                    }
                }
                Reply::Episodes(token, result) => {
                    if let Some(detail) = &mut self.detail
                        && detail.episodes.accepts(token)
                    {
                        detail.episodes.resolve(result);
                    }
                }
            }
        }
        let upload_limit = if self.web_player.is_some() { 1 } else { 3 };
        for _ in 0..upload_limit {
            let Ok((url, image)) = self.network.images.try_recv() else {
                return;
            };
            self.art.textures.insert(
                url.clone(),
                (
                    image.map_or(Art::Failed, |image| {
                        Art::Ready(ctx.load_texture(url, image, TextureOptions::LINEAR))
                    }),
                    Instant::now(),
                ),
            );
        }
        ctx.request_repaint_after(Duration::from_millis(12));
    }

    fn is_saved(&self, title: &Title) -> bool {
        self.library.iter().any(|entry| entry.key() == title.key())
    }

    fn toggle_save(&mut self, title: Title) {
        if let Some(index) = self
            .library
            .iter()
            .position(|entry| entry.key() == title.key())
        {
            self.library.remove(index);
        } else {
            self.library.insert(0, title);
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let inset = gutter(width);
        let compact = width < 920.0;
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(Margin::symmetric(inset, 0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(62.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = if compact { 8.0 } else { 20.0 };
                    let search = Id::new(SEARCH);
                    if ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::K)) {
                        self.begin_search();
                        ui.memory_mut(|memory| memory.request_focus(Id::new(PANEL_SEARCH)));
                    }
                    // Show the wordmark without the transparent padding in its source image.
                    let wordmark_uv = Rect::from_min_max(pos2(0.018, 0.27), pos2(0.982, 0.725));
                    let width = if compact { 108.0 } else { 132.0 };
                    let size = self.wordmark.size_vec2();
                    let aspect = (size.y * wordmark_uv.height()) / (size.x * wordmark_uv.width());
                    let logo = Image::new(&self.wordmark)
                        .uv(wordmark_uv)
                        .maintain_aspect_ratio(false)
                        .fit_to_exact_size(vec2(width, width * aspect))
                        .alt_text("Vortex");
                    let home = ui.add(Button::image(logo).frame(false));
                    home.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Button, true, "Vortex Home")
                    });
                    if home
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text("Home")
                        .clicked()
                    {
                        self.navigate(Page::Home);
                    }
                    ui.add_space(if compact { 4.0 } else { 9.0 });
                    let links = [
                        (Page::Anime, "Anime"),
                        (Page::Browse(Kind::Movie, 0), "Movies"),
                        (Page::Browse(Kind::Tv, 0), "TV Shows"),
                        (Page::Discover, "Genres"),
                    ];
                    for (page, name) in links {
                        let active = page == self.page
                            || page == Page::Discover && matches!(self.page, Page::Browse(_, 1..));
                        if nav_link(ui, name, active) {
                            self.navigate(page);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for page in [Page::Settings, Page::Library] {
                            if nav_link(ui, page.label(), page == self.page) {
                                self.navigate(page);
                            }
                        }
                        let field_width = (ui.available_width() - 4.0).clamp(75.0, 590.0);
                        self.search_input(
                            ui,
                            search,
                            field_width,
                            41.0,
                            if compact {
                                "Search…"
                            } else {
                                "Search movies, tv shows, anime..."
                            },
                        );
                    });
                });
            });
    }
}

fn nav_link(ui: &mut Ui, name: &str, active: bool) -> bool {
    ui.add(
        Button::new(label(name, 14.0).color(if active { LAVENDER } else { SOFT_TEXT }))
            .frame(false),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .clicked()
}

fn row_arrow(ui: &mut Ui, arrow: &str, hint: &str) -> bool {
    ui.add_sized(
        [32.0, 32.0],
        Button::new(RichText::new(arrow).size(20.0)).corner_radius(16),
    )
    .on_hover_text(hint)
    .on_hover_cursor(CursorIcon::PointingHand)
    .clicked()
}

fn rail_arrows(ui: &mut Ui) -> f32 {
    let right = row_arrow(ui, "›", "Scroll right");
    let left = row_arrow(ui, "‹", "Scroll left");
    f32::from(u8::from(right)) - f32::from(u8::from(left))
}

fn horizontal_rail(ui: &Ui, offset: f32, movement: f32, stride: f32) -> ScrollArea {
    let mut area = ScrollArea::horizontal()
        .auto_shrink([false, true])
        .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden);
    if movement != 0.0 {
        let step = (ui.available_width() / stride).floor().max(1.0) * stride;
        area = area.horizontal_scroll_offset((offset + step * movement).max(0.0));
    }
    area
}

fn provider_tile(ui: &mut Ui, art: &mut Artwork, network: &Network, provider: &Provider) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(130.0, 130.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &provider.provider_name));
    if ui.is_rect_visible(rect) {
        let base = match provider.provider_id {
            8 => [85, 8, 11],
            9 => [16, 48, 96],
            15 => [6, 77, 41],
            337 => [19, 28, 82],
            350 => [34, 35, 42],
            258 => [8, 70, 66],
            283 => [93, 43, 4],
            _ => [[43, 23, 82], [71, 14, 46], [11, 61, 47], [78, 40, 14]]
                [provider.provider_id as usize % 4],
        };
        let color = Color32::from_rgb(base[0], base[1], base[2]);
        let painter = ui.painter();
        painter.rect(
            rect,
            24,
            color,
            Stroke::new(
                1.0_f32,
                if response.hovered() || response.has_focus() {
                    LAVENDER
                } else {
                    BORDER
                },
            ),
            StrokeKind::Inside,
        );
        let logo = Rect::from_center_size(rect.center(), vec2(86.0, 86.0));
        if let Some(texture) = art.get(network, true, provider.logo()) {
            cover(painter, logo, texture, 43);
        } else {
            let name = fitted_text(ui, &provider.provider_name, strong(14.0), TEXT, 104.0, 2);
            painter.galley(rect.center() - name.size() / 2.0, name, TEXT);
        }
    }
    response
        .on_hover_text(&provider.provider_name)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

fn filter_chip(ui: &mut Ui, name: &str, active: bool) -> bool {
    let mut chip =
        Button::new(
            RichText::new(name)
                .size(13.0)
                .color(if active { TEXT } else { SOFT_TEXT }),
        )
        .corner_radius(17);
    if active {
        chip = chip
            .fill(Color32::from_rgb(36, 24, 66))
            .stroke(Stroke::new(1.0_f32, VIOLET));
    }
    ui.add(chip)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

fn genre_list(kind: Kind) -> &'static [(u32, &'static str)] {
    match kind {
        Kind::Movie => &[
            (28, "Action"),
            (12, "Adventure"),
            (16, "Animation"),
            (35, "Comedy"),
            (80, "Crime"),
            (99, "Documentary"),
            (18, "Drama"),
            (10751, "Family"),
            (14, "Fantasy"),
            (36, "History"),
            (27, "Horror"),
            (10402, "Music"),
            (9648, "Mystery"),
            (10749, "Romance"),
            (878, "Science Fiction"),
            (10770, "TV Movie"),
            (53, "Thriller"),
            (10752, "War"),
            (37, "Western"),
        ],
        Kind::Tv => &[
            (10759, "Action & Adventure"),
            (16, "Animation"),
            (35, "Comedy"),
            (80, "Crime"),
            (99, "Documentary"),
            (18, "Drama"),
            (10751, "Family"),
            (10762, "Kids"),
            (9648, "Mystery"),
            (10763, "News"),
            (10764, "Reality"),
            (10765, "Sci-Fi & Fantasy"),
            (10766, "Soap"),
            (10767, "Talk"),
            (10768, "War & Politics"),
            (37, "Western"),
        ],
        Kind::Other => &[],
    }
}

impl eframe::App for Vortex {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LIBRARY, &self.library);
        eframe::set_value(storage, HISTORY, &self.history);
        eframe::set_value(storage, SEARCH_HISTORY, &self.search_history);
        eframe::set_value(storage, PREFERENCES, &self.preferences);
    }

    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        if !self.account.signed_in(ctx) {
            if self.session_open {
                self.session_open = false;
                self.navigate(Page::Home);
                self.search_at = None;
                self.player_surface = None;
                if self.player_fullscreen {
                    self.set_player_fullscreen(ctx, false);
                }
            }
            CentralPanel::default()
                .frame(Frame::NONE.fill(BACKGROUND))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.2).min(150.0));
                        ui.label(label("Vortex", 36.0));
                        ui.add_space(16.0);
                        Frame::new()
                            .fill(PANEL)
                            .corner_radius(12)
                            .inner_margin(22)
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                self.account.show(ui, || serde_json::Value::Null);
                            });
                    });
                });
            return;
        }
        if !self.session_open {
            self.session_open = true;
            self.load_home();
            self.load_providers();
        }
        self.receive(ctx);
        if let Some(deadline) = self.search_at {
            let now = Instant::now();
            if now >= deadline {
                self.search_at = None;
                self.load_search();
            } else {
                ctx.request_repaint_after(deadline.duration_since(now));
            }
        }
        self.player_surface = None;
        if self.player_fullscreen {
            self.fullscreen_player(ctx, frame);
            return;
        }
        if ctx.input(|input| input.key_pressed(Key::Escape))
            && !Popup::is_any_open(ctx)
            && self.coming_soon.is_none()
        {
            if self.playback.is_some() || self.trailer.is_some() {
                self.stop_playback();
            } else if self.detail.is_some() {
                self.back_from_title();
            }
        }
        TopBottomPanel::top("searchbar")
            .frame(Frame::NONE)
            .show(ctx, |ui| self.header(ui));
        CentralPanel::default()
            .frame(Frame::NONE.fill(BACKGROUND))
            .show(ctx, |ui| {
                let detail_key = self.detail.as_ref().map(|detail| detail.summary.key());
                ScrollArea::vertical()
                    .id_salt((self.page, detail_key))
                    .scroll_source(
                        scroll_area::ScrollSource::SCROLL_BAR
                            | scroll_area::ScrollSource::MOUSE_WHEEL,
                    )
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.detail.is_some() {
                            self.details(ui);
                        } else {
                            match self.page {
                                Page::Home => self.home_screen(ui),
                                _ => {
                                    let inset = gutter(ui.available_width());
                                    Frame::new()
                                        .inner_margin(Margin::symmetric(inset, 37))
                                        .show(ui, |ui| {
                                            if self.page == Page::Settings {
                                                self.settings(ui);
                                            } else {
                                                self.catalog_screen(ui);
                                            }
                                        })
                                        .inner
                                }
                            }
                        }
                        ui.add_space(22.0);
                    });
            });
        self.coming_soon_dialog(ctx);
        self.sync_player(ctx, frame);
    }
}

impl Vortex {
    fn continue_watching(&mut self, ui: &mut Ui) {
        if self.history.is_empty() {
            return;
        }
        let inset = gutter(ui.available_width());
        let width = self.preferences.poster_size.width();
        let mut resume = None;
        let mut remove = None;
        let mut clear = false;
        Frame::new()
            .inner_margin(Margin::symmetric(inset, 24))
            .show(ui, |ui| {
                let mut movement = 0.0;
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    section_heading(ui, "Continue Watching", 24.0, CYAN);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        movement = rail_arrows(ui);
                        clear = ui
                            .add(Button::new(label("CLEAR", 13.0).color(MUTED)).frame(false))
                            .clicked();
                    });
                });
                ui.add_space(12.0);
                let row = horizontal_rail(ui, self.history_offset, movement, width + 24.0)
                    .id_salt("history")
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 24.0;
                        ui.horizontal_top(|ui| {
                            for entry in &self.history {
                                let title = &entry.title;
                                ui.push_id(title.key(), |ui| {
                                    let origin = ui.cursor().min;
                                    let clicked =
                                        poster(ui, &mut self.art, &self.network, title, width);
                                    if ui.is_rect_visible(Rect::from_min_size(
                                        origin,
                                        vec2(width, width * 1.5),
                                    )) {
                                        let badge = if title.kind() == Kind::Tv {
                                            format!("S{} E{}", entry.season, entry.episode)
                                        } else {
                                            "MOVIE".to_owned()
                                        };
                                        let rect = Rect::from_min_size(
                                            origin + vec2(8.0, 8.0),
                                            vec2(48.0, 21.0),
                                        );
                                        ui.painter().rect_filled(rect, 4, CYAN);
                                        ui.painter().text(
                                            rect.center(),
                                            Align2::CENTER_CENTER,
                                            badge,
                                            strong(10.0),
                                            TEXT,
                                        );
                                        let target = Rect::from_min_size(
                                            origin + vec2(width - 34.0, 6.0),
                                            vec2(27.0, 27.0),
                                        );
                                        let response = ui.interact(
                                            target,
                                            ui.make_persistent_id("remove"),
                                            Sense::click(),
                                        );
                                        if response.hovered()
                                            || response.has_focus()
                                            || ui.rect_contains_pointer(Rect::from_min_size(
                                                origin,
                                                vec2(width, width * 1.5),
                                            ))
                                        {
                                            ui.painter().circle_filled(
                                                target.center(),
                                                13.0,
                                                Color32::from_rgb(221, 53, 70),
                                            );
                                            ui.painter().text(
                                                target.center(),
                                                Align2::CENTER_CENTER,
                                                "×",
                                                strong(18.0),
                                                TEXT,
                                            );
                                        }
                                        response.widget_info(|| {
                                            WidgetInfo::labeled(
                                                WidgetType::Button,
                                                true,
                                                "Remove from Continue Watching",
                                            )
                                        });
                                        if response.clicked() {
                                            remove = Some(title.key());
                                            return;
                                        }
                                    }
                                    if clicked {
                                        resume = Some(entry.clone());
                                    }
                                });
                            }
                        });
                    });
                self.history_offset = row.state.offset.x;
            });
        if clear {
            self.history.clear();
            self.history_offset = 0.0;
        } else if let Some(key) = remove {
            self.history.retain(|entry| entry.title.key() != key);
        } else if let Some(entry) = resume {
            self.open_title(entry.title.clone());
            self.start_playback(&entry.title, entry.season, entry.episode, ui.ctx());
        }
    }

    fn provider_section(&mut self, ui: &mut Ui, all: bool) {
        let mut page = None;
        let mut retry = false;
        let inset = if all { 0 } else { gutter(ui.available_width()) };
        Frame::new()
            .inner_margin(Margin::symmetric(inset, if all { 0 } else { 24 }))
            .show(ui, |ui| {
                let mut movement = 0.0;
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    section_heading(ui, "Providers", if all { 32.0 } else { 24.0 }, CYAN);
                    if !all {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            movement = rail_arrows(ui);
                            if ui
                                .add(
                                    Button::new(RichText::new("View All").size(15.0).color(CYAN))
                                        .frame(false),
                                )
                                .clicked()
                            {
                                page = Some(Page::Providers);
                            }
                        });
                    }
                });
                ui.add_space(12.0);
                match &self.providers {
                    Load::Ready(providers) if !providers.is_empty() => {
                        let mut tile = |ui: &mut Ui, provider: &Provider| {
                            if provider_tile(ui, &mut self.art, &self.network, provider) {
                                page = Some(Page::Provider(provider.provider_id, Kind::Movie));
                            }
                        };
                        if all {
                            let columns =
                                ((ui.available_width() + 20.0) / 150.0).floor().max(1.0) as usize;
                            ui.spacing_mut().item_spacing = vec2(20.0, 20.0);
                            visible_grid(ui, providers, columns, 130.0, 20.0, tile);
                        } else {
                            let row = horizontal_rail(ui, self.provider_offset, movement, 150.0)
                                .id_salt("providers rail")
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 20.0;
                                    ui.horizontal_top(|ui| {
                                        for provider in providers.iter().take(20) {
                                            tile(ui, provider);
                                        }
                                    });
                                });
                            self.provider_offset = row.state.offset.x;
                        }
                    }
                    Load::Ready(_) => {
                        status(ui, "No providers are available.", false);
                    }
                    Load::Failed(error) => retry = status(ui, error, true),
                    _ => loading(ui, "Loading providers…"),
                }
            });
        if retry {
            self.load_providers();
        }
        if let Some(page) = page {
            self.navigate(page);
        }
    }

    fn home_screen(&mut self, ui: &mut Ui) {
        let featured = self.home.iter().take(2).find_map(|rail| match &rail.items {
            Load::Ready((titles, _)) => {
                let slides: Vec<_> = titles
                    .iter()
                    .filter(|title| title.backdrop_path.is_some())
                    .take(5)
                    .cloned()
                    .collect();
                (!slides.is_empty()).then_some(slides)
            }
            _ => None,
        });
        if let Some(slides) = featured {
            self.featured(ui, &slides);
        }
        if self.detail.is_some() {
            return;
        }
        self.continue_watching(ui);
        if self.detail.is_some() {
            return;
        }
        self.provider_section(ui, false);
        if self.page != Page::Home {
            return;
        }

        let mut open = None;
        let mut collection = None;
        let mut retry = false;
        let inset = gutter(ui.available_width());
        let poster_width = self.preferences.poster_size.width();
        Frame::new()
            .inner_margin(Margin::symmetric(inset, 24))
            .show(ui, |ui| {
                for (index, rail) in self.home.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        let mut move_by = 0.0;
                        ui.horizontal(|ui| {
                            ui.set_width(ui.available_width());
                            section_heading(ui, rail.label, 24.0, CYAN);
                            if matches!(&rail.items, Load::Ready((titles, _)) if !titles.is_empty())
                            {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if row_arrow(ui, "›", "Scroll right") {
                                        move_by = 1.0;
                                    }
                                    if row_arrow(ui, "‹", "Scroll left") {
                                        move_by = -1.0;
                                    }
                                    ui.add_space(8.0);
                                    if ui
                                        .add(
                                            Button::new(
                                                RichText::new("View All").size(15.0).color(VIOLET),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_cursor(CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        collection = Some(index);
                                    }
                                });
                            }
                        });
                        ui.add_space(12.0);
                        match &rail.items {
                            Load::Ready((titles, _)) if !titles.is_empty() => {
                                let mut scroll = ScrollArea::horizontal()
                                    .auto_shrink([false, true])
                                    .scroll_bar_visibility(
                                        scroll_area::ScrollBarVisibility::AlwaysHidden,
                                    );
                                if move_by != 0.0 {
                                    let stride = poster_width + 24.0;
                                    let step =
                                        (ui.available_width() / stride).floor().max(1.0) * stride;
                                    scroll = scroll.horizontal_scroll_offset(
                                        (rail.offset + step * move_by).max(0.0),
                                    );
                                }
                                let row = scroll.show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 24.0;
                                    ui.horizontal_top(|ui| {
                                        for title in
                                            titles.iter().skip(usize::from(index == 0)).take(18)
                                        {
                                            if poster(
                                                ui,
                                                &mut self.art,
                                                &self.network,
                                                title,
                                                poster_width,
                                            ) {
                                                open = Some(title.clone());
                                            }
                                        }
                                    });
                                });
                                rail.offset = row.state.offset.x;
                            }
                            Load::Failed(error) => retry |= status(ui, error, true),
                            Load::Ready(_) => {
                                status(ui, "No titles are available in this collection.", false);
                            }
                            _ => loading(ui, "Loading titles…"),
                        }
                        ui.add_space(37.0);
                    });
                }
            });
        if retry {
            self.load_home();
        }
        if let Some(index) = collection {
            self.navigate(Page::Collection(index));
        }
        if let Some(title) = open {
            self.open_title(title);
        }
    }

    fn featured(&mut self, ui: &mut Ui, slides: &[Title]) {
        let count = slides.len();
        let current = self.hero % count;
        let title = &slides[current];
        let width = ui.available_width();
        let height = (ui.ctx().content_rect().height() * 0.85 - 64.0).clamp(310.0, 840.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let visible = ui.is_rect_visible(rect);
        let image = self.art.get(&self.network, visible, title.backdrop());
        backdrop(ui.painter(), rect, image);
        if self.preferences.autoplay_featured && count > 1 {
            self.art.get(
                &self.network,
                visible,
                slides[(current + 1) % count].backdrop(),
            );
        }
        let inset = f32::from(gutter(width));
        let content_width = (width - 2.0 * inset).min(672.0);
        let compact = width < 920.0;
        let title_font = display(if compact { 43.0 } else { 60.0 });
        let synopsis_font = FontId::proportional(if compact { 15.0 } else { 18.0 });
        let mut title_text = fitted_text(ui, title.name(), title_font, TEXT, content_width, 3);
        if title_text.rows.len() > 2 && !compact {
            let fitted = fitted_text(ui, title.name(), display(53.0), TEXT, content_width, 2);
            if !fitted.elided {
                title_text = fitted;
            }
        }
        let synopsis = fitted_text(
            ui,
            title.overview.as_deref().unwrap_or_default(),
            synopsis_font,
            SOFT_TEXT,
            content_width,
            3,
        );
        let content_height = title_text.size().y + synopsis.size().y + 83.0;
        let origin = pos2(rect.left() + inset, rect.center().y - content_height / 2.0);
        let bounds = Rect::from_min_size(origin, vec2(content_width, content_height));
        let mut hero_ui = ui.new_child(
            UiBuilder::new()
                .id_salt("featured title")
                .max_rect(bounds)
                .layout(Layout::top_down(Align::Min)),
        );
        hero_ui
            .add(Label::new(title_text))
            .on_hover_text(title.name());
        hero_ui.add_space(5.0);
        hero_ui.add(Label::new(synopsis));
        hero_ui.add_space(13.0);
        let mut watch = false;
        let mut info = false;
        hero_ui.horizontal(|ui| {
            watch = button(ui, "▶   Watch Now", true).clicked();
            info = button(ui, "More Info", false).clicked();
        });
        if watch || info {
            self.open_title(title.clone());
            if watch && (title.kind() == Kind::Movie || title.upcoming()) {
                self.start_playback(title, 1, 1, ui.ctx());
            }
        }

        let pager = Rect::from_min_max(
            pos2(rect.left() + inset, rect.bottom() - 57.0),
            pos2(rect.right() - inset, rect.bottom() - 33.0),
        );
        let mut controls = ui.new_child(
            UiBuilder::new()
                .id_salt("featured slides")
                .max_rect(pager)
                .layout(Layout::right_to_left(Align::Center)),
        );
        controls.spacing_mut().item_spacing.x = 5.0;
        for index in (0..count).rev() {
            let selected = index == current;
            let (dot, response) = controls.allocate_exact_size(
                vec2(if selected { 32.0 } else { 10.0 }, 23.0),
                Sense::click(),
            );
            let caption = format!("Featured title {}", index + 1);
            response
                .widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &caption));
            let mark =
                Rect::from_center_size(dot.center(), vec2(if selected { 32.0 } else { 8.0 }, 6.0));
            let color = if selected {
                VIOLET
            } else if response.hovered() || response.has_focus() {
                SOFT_TEXT
            } else {
                Color32::from_white_alpha(85)
            };
            controls.painter().rect_filled(mark, 3, color);
            if response
                .on_hover_text(caption)
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                self.hero = index;
            }
        }
        if self.hero % count != current || !visible || !ui.input(|input| input.focused) {
            self.hero_changed_at = Instant::now();
        } else if self.preferences.autoplay_featured && count > 1 {
            let elapsed = self.hero_changed_at.elapsed();
            if elapsed >= HERO_INTERVAL {
                self.hero = (current + 1) % count;
                self.hero_changed_at = Instant::now();
                ui.ctx().request_repaint();
            } else {
                ui.ctx().request_repaint_after(HERO_INTERVAL - elapsed);
            }
        }
    }

    fn settings(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(PANEL)
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_max_width(480.0);
                ui.label(label("Appearance", 21.0));
                ui.add_space(5.0);
                ui.checkbox(
                    &mut self.preferences.autoplay_featured,
                    "Automatically rotate featured titles",
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Poster size");
                    for (size, name) in [
                        (PosterSize::Compact, "Compact"),
                        (PosterSize::Standard, "Standard"),
                        (PosterSize::Large, "Large"),
                    ] {
                        ui.selectable_value(&mut self.preferences.poster_size, size, name);
                    }
                });
            });
        ui.add_space(16.0);
        Frame::new()
            .fill(PANEL)
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                let restored = self.account.show(ui, || {
                    serde_json::json!({
                        "library": self.library,
                        "progress": self.history,
                        "searchHistory": self.search_history,
                        "preferences": self.preferences,
                    })
                });
                if let Some(mut data) = restored {
                    if let Ok(library) = serde_json::from_value(data["library"].take()) {
                        self.library = library;
                    }
                    if let Ok(history) = serde_json::from_value(data["progress"].take()) {
                        self.history = history;
                        self.history_offset = 0.0;
                    }
                    if let Ok(search) = serde_json::from_value(data["searchHistory"].take()) {
                        self.search_history = search;
                    }
                    if let Ok(preferences) = serde_json::from_value(data["preferences"].take()) {
                        self.preferences = preferences;
                    }
                }
            });
    }

    fn search_panel(&mut self, ui: &mut Ui) {
        ui.label(label("Search", 37.0));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 183.0).clamp(260.0, 625.0);
            self.search_input(
                ui,
                Id::new(PANEL_SEARCH),
                width,
                46.0,
                "Search movies, TV shows, and anime",
            );
            if button(ui, "Search", true).clicked() {
                self.remember_search();
                self.search_at = None;
                self.load_search();
            }
            if !self.query.is_empty() && ui.button("×").on_hover_text("Clear search").clicked() {
                self.query.clear();
                self.listing = None;
                self.search_at = None;
                ui.memory_mut(|memory| memory.request_focus(Id::new(PANEL_SEARCH)));
            }
        });
        ui.add_space(25.0);
        if !self.query.trim().is_empty() {
            section_heading(ui, "Search Results", 24.0, CYAN);
            ui.add_space(12.0);
            return;
        }
        if self.search_history.is_empty() {
            ui.label(muted(
                "Recent searches appear here when you submit a search or open a result.",
            ));
            return;
        }
        let mut clear = false;
        let mut remove = None;
        let mut selected = None;
        ui.horizontal(|ui| {
            section_heading(ui, "Recent Searches", 24.0, VIOLET);
            clear = ui
                .add(Button::new(RichText::new("Clear history").color(LAVENDER)).frame(false))
                .clicked();
        });
        ui.add_space(9.0);
        ui.horizontal_wrapped(|ui| {
            for (index, query) in self.search_history.iter().enumerate() {
                ui.push_id(index, |ui| {
                    Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(18)
                        .inner_margin(Margin::symmetric(7, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let name = Label::new(RichText::new(query).color(SOFT_TEXT))
                                    .truncate()
                                    .sense(Sense::click());
                                if ui
                                    .add_sized(
                                        [
                                            ((query.chars().count() as f32) * 8.0)
                                                .clamp(50.0, 260.0),
                                            32.0,
                                        ],
                                        name,
                                    )
                                    .on_hover_text(query)
                                    .on_hover_cursor(CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    selected = Some(query.clone());
                                }
                                if ui
                                    .add(Button::new("×").frame(false))
                                    .on_hover_text("Remove recent search")
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                            });
                        });
                });
            }
        });
        if clear {
            self.search_history.clear();
        } else if let Some(index) = remove {
            self.search_history.remove(index);
        } else if let Some(query) = selected {
            self.query = query;
            self.remember_search();
            self.search_at = None;
            self.load_search();
        }
    }

    fn catalog_screen(&mut self, ui: &mut Ui) {
        if self.page == Page::Discover {
            self.genres(ui);
            return;
        }
        if self.page == Page::Providers {
            self.provider_section(ui, true);
            return;
        }
        if self.page == Page::Search {
            self.search_panel(ui);
            if self.query.trim().is_empty() {
                return;
            }
        } else {
            let heading = match self.page {
                Page::Collection(index) => self.home[index].label,
                Page::Provider(id, _) => match &self.providers {
                    Load::Ready(providers) => providers
                        .iter()
                        .find(|provider| provider.provider_id == id)
                        .map_or("Provider", |provider| provider.provider_name.as_str()),
                    _ => "Provider",
                },
                _ => self.page.label(),
            };
            section_heading(ui, heading, 32.0, CYAN);
            ui.add_space(15.0);
        }
        if let Page::Provider(id, kind) = self.page {
            let mut selected = kind;
            ui.horizontal(|ui| {
                if filter_chip(ui, "Movies", kind == Kind::Movie) {
                    selected = Kind::Movie;
                }
                if filter_chip(ui, "TV Shows", kind == Kind::Tv) {
                    selected = Kind::Tv;
                }
            });
            ui.add_space(10.0);
            if selected != kind {
                self.navigate(Page::Provider(id, selected));
            }
        }
        if matches!(self.page, Page::Anime | Page::Browse(..)) {
            self.browse_filters(ui);
            ui.add_space(16.0);
        }

        let mut next = false;
        let open = if self.page == Page::Library {
            if self.library.is_empty() {
                status(ui, "Save a movie or show to find it here.", false);
            }
            poster_grid(
                ui,
                &mut self.art,
                &self.network,
                &self.library,
                self.preferences.poster_size.width(),
            )
        } else if let Some(listing) = &self.listing {
            let open = poster_grid(
                ui,
                &mut self.art,
                &self.network,
                &listing.entries,
                self.preferences.poster_size.width(),
            );
            match &listing.load {
                Load::Failed(error) => next = status(ui, error, true),
                Load::Loading(_) => loading(ui, "Loading titles…"),
                Load::Ready(()) if listing.entries.is_empty() => {
                    status(ui, "No matching titles found.", false);
                }
                _ if listing.page < listing.total_pages => {
                    ui.add_space(8.0);
                    next = button(ui, "Load more", false).clicked();
                }
                _ => {}
            }
            open
        } else {
            if self.search_at.is_some() && !self.query.trim().is_empty() {
                loading(ui, "Searching…");
            } else {
                status(ui, "Search by movie or TV show title.", false);
            }
            None
        };
        if next {
            self.next_page();
        }
        if let Some(title) = open {
            if self.page == Page::Search {
                self.remember_search();
            }
            self.open_title(title);
        }
    }

    fn browse_filters(&mut self, ui: &mut Ui) {
        let sorts = [
            ("popularity.desc", "Popular"),
            ("vote_average.desc", "Top Rated"),
        ];
        let previous = self.discover_sort;
        let mut next = None;
        ui.horizontal_wrapped(|ui| {
            for (value, label) in sorts {
                if filter_chip(ui, label, self.discover_sort == value) {
                    self.discover_sort = value;
                }
            }
        });
        if let Page::Browse(kind, selected) = self.page {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for &(id, label) in std::iter::once(&(0, "All genres")).chain(genre_list(kind)) {
                    if filter_chip(ui, label, selected == id) {
                        next = Some(Page::Browse(kind, id));
                    }
                }
            });
        }
        if let Some(page) = next.filter(|page| *page != self.page) {
            self.navigate(page);
        } else if previous != self.discover_sort {
            match self.page {
                Page::Browse(kind, genre) => self.load_browse(kind, genre),
                Page::Anime => self.load_anime(),
                _ => {}
            }
        }
    }

    fn genres(&mut self, ui: &mut Ui) {
        ui.label(label("Browse by Genre", 37.0));
        ui.add_space(20.0);
        let mut next = None;
        section_heading(ui, "Featured", 24.0, VIOLET);
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_sized(
                    [245.0, 75.0],
                    Button::new(label("International Drama", 18.0))
                        .fill(Color32::from_rgb(28, 19, 47))
                        .corner_radius(14),
                )
                .clicked()
                && let Some(index) = self
                    .home
                    .iter()
                    .position(|rail| rail.label == "Heart-Racing K-Dramas")
            {
                next = Some(Page::Collection(index));
            }
            if ui
                .add_sized(
                    [245.0, 75.0],
                    Button::new(label("Anime Collection", 18.0))
                        .fill(Color32::from_rgb(12, 30, 43))
                        .corner_radius(14),
                )
                .clicked()
            {
                next = Some(Page::Anime);
            }
        });
        for (kind, name, accent) in [
            (Kind::Movie, "Movie Genres", CYAN),
            (Kind::Tv, "TV Show Genres", VIOLET),
        ] {
            ui.add_space(30.0);
            section_heading(ui, name, 24.0, accent);
            ui.add_space(8.0);
            let gap = 14.0;
            let width = ui.available_width();
            let columns = ((width + gap) / (150.0 + gap)).floor().clamp(2.0, 6.0) as usize;
            let tile = (width - gap * (columns - 1) as f32) / columns as f32 - 0.5;
            ui.spacing_mut().item_spacing.x = gap;
            for row in genre_list(kind).chunks(columns) {
                ui.horizontal(|ui| {
                    for &(genre, name) in row {
                        if ui
                            .add_sized(
                                [tile, 52.0],
                                Button::new(RichText::new(name).size(14.0).color(SOFT_TEXT))
                                    .corner_radius(12),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .clicked()
                        {
                            next = Some(Page::Browse(kind, genre));
                        }
                    }
                });
            }
        }
        if let Some(page) = next {
            self.navigate(page);
        }
    }

    fn details(&mut self, ui: &mut Ui) {
        let detail = self.detail.as_ref().expect("open detail page");
        let title = detail.title();
        let saved = self.is_saved(title);
        let mut save = false;
        let mut watch = None;
        let ready = self.web_player.as_ref().is_some_and(EmbeddedPlayer::ready);
        let mut extras = None;
        let width = ui.available_width();
        let inset = gutter(width);
        let art_bounds = Rect::from_min_size(
            ui.cursor().min,
            vec2(width, (width * 0.49).clamp(385.0, 660.0)),
        );
        if let Some(image) = self.art.get(
            &self.network,
            ui.is_rect_visible(art_bounds),
            title.backdrop(),
        ) {
            cover(ui.painter(), art_bounds, image, 0);
            let dark = Color32::from_black_alpha(206);
            fade(
                ui.painter(),
                art_bounds,
                [dark, dark, BACKGROUND, BACKGROUND],
            );
            let bottom = Rect::from_min_max(
                pos2(art_bounds.left(), art_bounds.bottom() - 28.0),
                art_bounds.max,
            );
            fade(
                ui.painter(),
                bottom,
                [
                    Color32::TRANSPARENT,
                    Color32::TRANSPARENT,
                    BACKGROUND,
                    BACKGROUND,
                ],
            );
        }

        let back = Frame::new()
            .inner_margin(Margin::symmetric(inset, 26))
            .show(ui, |ui| {
                let back = ui.button("‹  Back").clicked();
                ui.add_space(19.0);

                let compact = width < 1024.0;
                let poster_width = if compact {
                    142.0
                } else if width < 1280.0 {
                    238.0
                } else {
                    282.0
                };
                let mut synopsis = |ui: &mut Ui| {
                    ui.add_space(9.0);
                    ui.add(
                        Label::new(label(title.name(), if compact { 29.0 } else { 36.0 })).wrap(),
                    );
                    ui.label(
                        RichText::new(metadata_line(title))
                            .size(14.0)
                            .color(LAVENDER),
                    );
                    if !title.genres.is_empty() {
                        ui.add_space(3.0);
                        ui.horizontal_wrapped(|ui| {
                            for genre in &title.genres {
                                Frame::new()
                                    .fill(Color32::from_white_alpha(24))
                                    .corner_radius(12)
                                    .inner_margin(Margin::symmetric(10, 4))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(&genre.name).size(12.0).color(SOFT_TEXT),
                                        );
                                    });
                            }
                        });
                    }
                    ui.add_space(8.0);
                    let save_label = if saved {
                        "✓  In Library"
                    } else {
                        "+  My Library"
                    };
                    save = button(ui, save_label, false).clicked();
                    if let Some(overview) =
                        title.overview.as_deref().filter(|text| !text.is_empty())
                    {
                        ui.add_space(10.0);
                        ui.add(
                            Label::new(RichText::new(overview).size(15.0).color(SOFT_TEXT)).wrap(),
                        );
                    }
                };

                if compact {
                    ui.horizontal_top(|ui| {
                        detail_poster(ui, &mut self.art, &self.network, title, poster_width);
                        ui.add_space(12.0);
                        ui.vertical(synopsis);
                    });
                    ui.add_space(22.0);
                    watch = Some(watch_section(
                        ui,
                        detail,
                        &mut self.art,
                        &self.network,
                        &mut self.preferred_server,
                        self.playback.as_ref(),
                        ready && self.playback.is_some(),
                    ));
                    extras = Some(detail_extras(
                        ui,
                        detail,
                        &mut self.art,
                        &self.network,
                        self.trailer.as_ref(),
                        ready && self.trailer.is_some(),
                    ));
                } else {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(poster_width);
                            detail_poster(ui, &mut self.art, &self.network, title, poster_width);
                            synopsis(ui);
                        });
                        ui.add_space(22.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            watch = Some(watch_section(
                                ui,
                                detail,
                                &mut self.art,
                                &self.network,
                                &mut self.preferred_server,
                                self.playback.as_ref(),
                                ready && self.playback.is_some(),
                            ));
                            extras = Some(detail_extras(
                                ui,
                                detail,
                                &mut self.art,
                                &self.network,
                                self.trailer.as_ref(),
                                ready && self.trailer.is_some(),
                            ));
                        });
                    });
                }
                back
            })
            .inner;

        let title = title.clone();
        let recommended = self.recommendations(ui);
        if save {
            self.toggle_save(title.clone());
        }
        let watch = watch.expect("watch panel rendered");
        let extras = extras.expect("detail extras rendered");
        self.player_surface = if self.trailer.is_some() {
            extras.surface
        } else {
            Some((watch.rect, watch.clip))
        };
        if let Some((season, episode)) = watch.play {
            self.start_playback(&title, season, episode, ui.ctx());
        }
        if watch.stop {
            self.stop_playback();
        }
        if watch.fullscreen || extras.fullscreen {
            self.set_player_fullscreen(ui.ctx(), true);
        }
        if extras.close {
            self.stop_playback();
        }
        if let Some(trailer) = extras.selected {
            self.stop_playback();
            self.trailer = Some(TrailerPlayback {
                selected: trailer,
                error: None,
            });
            self.player_surface = None;
            ui.ctx().request_repaint();
        }
        if back {
            self.back_from_title();
        } else if let Some(movie) = recommended {
            self.related_backstack.push(title);
            self.show_title(movie);
        } else if watch.retry_details {
            self.show_title(title);
        } else if let Some(season) = watch.season {
            self.select_season(season);
        }
    }

    fn recommendations(&mut self, ui: &mut Ui) -> Option<Title> {
        let detail = self.detail.as_mut().expect("title page");
        let heading = if detail.summary.kind() == Kind::Tv {
            "Recommended TV Shows"
        } else {
            "Recommended Movies"
        };
        let inset = gutter(ui.available_width());
        let width = self.preferences.poster_size.width();
        let mut selected = None;
        let mut retry = false;
        Frame::new()
            .inner_margin(Margin::symmetric(inset, 19))
            .show(ui, |ui| {
                let mut movement = 0.0;
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    section_heading(ui, heading, 24.0, CYAN);
                    if matches!(&detail.recommendations, Load::Ready(items) if !items.is_empty()) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            movement = rail_arrows(ui)
                        });
                    }
                });
                ui.add_space(12.0);
                match &detail.recommendations {
                    Load::Ready(items) if !items.is_empty() => {
                        let row = horizontal_rail(
                            ui,
                            detail.recommendation_offset,
                            movement,
                            width + 24.0,
                        )
                        .id_salt("movie recommendations")
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 24.0;
                            ui.horizontal_top(|ui| {
                                for movie in items {
                                    if poster(ui, &mut self.art, &self.network, movie, width) {
                                        selected = Some(movie.clone());
                                    }
                                }
                            });
                        });
                        detail.recommendation_offset = row.state.offset.x;
                    }
                    Load::Ready(_) => {
                        status(
                            ui,
                            "No recommendations are available for this title.",
                            false,
                        );
                    }
                    Load::Failed(error) => retry = status(ui, error, true),
                    _ => loading(ui, "Loading recommendations…"),
                }
            });
        if retry {
            self.load_recommendations();
        }
        selected
    }

    fn start_playback(&mut self, title: &Title, season: u32, episode: u32, ctx: &Context) {
        if let Some(premiere) = premiere_notice(title) {
            self.coming_soon = Some(premiere);
            ctx.request_repaint();
            return;
        }
        if self.trailer.is_some() {
            self.stop_playback();
            self.player_surface = None;
        }
        self.history
            .retain(|entry| entry.title.key() != title.key());
        self.history.insert(
            0,
            HistoryEntry {
                title: title.clone(),
                season,
                episode,
                timestamp: u64::try_from(
                    time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000,
                )
                .unwrap_or_default(),
            },
        );
        self.history.truncate(20);
        self.history_offset = 0.0;
        self.playback = Some(Playback {
            episode: (season, episode),
            servers: server_urls(title, season, episode),
            error: None,
        });
        ctx.request_repaint();
    }

    fn stop_playback(&mut self) {
        self.web_player = None;
        self.playback = None;
        self.trailer = None;
    }

    fn coming_soon_dialog(&mut self, ctx: &Context) {
        let Some((name, date)) = &self.coming_soon else {
            return;
        };
        let id = Id::new("upcoming release");
        let response = Modal::new(id)
            .area(Modal::default_area(id).fade_in(false))
            .backdrop_color(Color32::from_black_alpha(170))
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(16)
                    .inner_margin(Margin::symmetric(22, 16)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(370.0);
                ui.set_min_width(340.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(label("Coming Soon", 29.0).color(LAVENDER));
                    ui.label(label(name, 20.0));
                    let message = date
                        .as_ref()
                        .map(|date| format!("Scheduled for {date}"))
                        .unwrap_or_else(|| "Release date to be announced".to_owned());
                    ui.label(muted(message));
                    ui.add_space(8.0);
                    button(ui, "OK", true).clicked()
                })
                .inner
            });
        if response.inner || response.should_close() {
            self.coming_soon = None;
        }
    }

    fn set_player_fullscreen(&mut self, ctx: &Context, fullscreen: bool) {
        self.player_fullscreen = fullscreen;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(fullscreen));
        ctx.request_repaint();
    }

    fn fullscreen_player(&mut self, ctx: &Context, frame: &eframe::Frame) {
        if ctx.input(|input| input.key_pressed(Key::Escape) || input.key_pressed(Key::F11))
            || (self.playback.is_none() && self.trailer.is_none())
        {
            self.set_player_fullscreen(ctx, false);
            return;
        }
        let mut close = false;
        TopBottomPanel::top("fullscreen player controls")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(14, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    close = ui.button("Exit fullscreen").clicked();
                    screen_cast::button(ui);
                });
            });
        if close {
            self.set_player_fullscreen(ctx, false);
            return;
        }
        CentralPanel::default()
            .frame(Frame::NONE.fill(BACKGROUND))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                self.player_surface = Some((rect, ui.clip_rect()));
                if let Some(error) = self
                    .playback
                    .as_ref()
                    .and_then(|video| video.error.as_deref())
                    .or_else(|| {
                        self.trailer
                            .as_ref()
                            .and_then(|video| video.error.as_deref())
                    })
                {
                    status(ui, error, false);
                }
            });
        self.sync_player(ctx, frame);
    }

    fn sync_player(&mut self, ctx: &Context, frame: &eframe::Frame) {
        let video = if let Some(playback) = &mut self.playback {
            Some((
                playback.servers[self.preferred_server].clone(),
                &mut playback.error,
                PlayerMode::Stream,
            ))
        } else {
            self.trailer.as_mut().map(|trailer| {
                (
                    trailer.selected.embed_url(),
                    &mut trailer.error,
                    PlayerMode::Trailer,
                )
            })
        };
        let (Some((source, playback_error, mode)), Some((rect, clip))) =
            (video, self.player_surface)
        else {
            self.web_player = None;
            return;
        };
        if playback_error.is_some() {
            return;
        }
        // The native child sits above egui. Hide it temporarily while a menu overlaps it.
        let visible = if Popup::is_any_open(ctx) {
            Rect::NOTHING
        } else {
            clip
        };
        let zoom = ctx.zoom_factor();
        let scaled = |r: Rect| {
            Rect::from_min_max(
                (r.min.to_vec2() * zoom).to_pos2(),
                (r.max.to_vec2() * zoom).to_pos2(),
            )
        };
        if self.web_player.is_none() {
            if !rect.intersect(visible).is_positive() {
                return;
            }
            match EmbeddedPlayer::new(frame, scaled(rect), ctx, mode) {
                Ok(Some(player)) => self.web_player = Some(player),
                Ok(None) => return,
                Err(error) => {
                    *playback_error = Some(error);
                    ctx.request_repaint();
                    return;
                }
            }
        }
        let player = self.web_player.as_mut().expect("player created");
        let ready = player.ready();
        if let Err(error) = player.update(scaled(rect), scaled(visible), &source, ctx) {
            self.web_player = None;
            *playback_error = Some(error);
            ctx.request_repaint();
        } else if ready != player.ready() {
            ctx.request_repaint();
        }
    }
}

fn premiere_notice(title: &Title) -> Option<(String, Option<String>)> {
    title.upcoming().then(|| {
        (
            title.name().to_owned(),
            title.premiere_date().map(str::to_owned),
        )
    })
}

fn metadata_line(title: &Title) -> String {
    let mut pieces = vec![title.kind().label().to_owned()];
    if !title.year().is_empty() {
        pieces.push(title.year().to_owned());
    }
    if let Some(rating) = title.rating() {
        pieces.push(format!("★ {rating}"));
    }
    if title.kind() == Kind::Movie {
        if let Some(runtime) = title.runtime.filter(|minutes| *minutes > 0) {
            pieces.push(format!("{runtime} min"));
        }
    } else if let Some(seasons) = title.number_of_seasons.filter(|count| *count > 0) {
        let unit = if seasons == 1 { "season" } else { "seasons" };
        pieces.push(format!("{seasons} {unit}"));
    }
    pieces.join("  ·  ")
}

fn detail_poster(ui: &mut Ui, art: &mut Artwork, network: &Network, title: &Title, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 1.5), Sense::hover());
    let visible = ui.is_rect_visible(rect);
    if !visible {
        return;
    }
    ui.painter().add(
        egui::epaint::RectShape::filled(rect.expand(5.0), 16, VIOLET.gamma_multiply(0.38))
            .with_blur_width(25.0),
    );
    ui.painter().rect_filled(rect, 12, CARD);
    if let Some(image) = art.get(network, visible, title.poster()) {
        cover(ui.painter(), rect, image, 12);
    }
    ui.painter()
        .rect_stroke(rect, 12, Stroke::new(1.0_f32, BORDER), StrokeKind::Inside);
}

#[derive(Default)]
struct ExtraActions {
    selected: Option<Trailer>,
    surface: Option<(Rect, Rect)>,
    close: bool,
    fullscreen: bool,
}

fn detail_extras(
    ui: &mut Ui,
    detail: &Detail,
    art: &mut Artwork,
    network: &Network,
    active: Option<&TrailerPlayback>,
    ready: bool,
) -> ExtraActions {
    let mut actions = ExtraActions::default();
    let Load::Ready(details) = &detail.metadata else {
        return actions;
    };
    let width = ui.available_width().min(1152.0);
    if let Some(videos) = details
        .videos
        .as_ref()
        .filter(|videos| !videos.results.is_empty())
    {
        ui.add_space(22.0);
        let heading = ui.cursor().min;
        detail_rail(ui, "trailers", "Trailers & Clips", width, 256.0, |ui| {
            for trailer in &videos.results {
                if trailer_card(ui, art, network, trailer) {
                    actions.selected = Some(trailer.clone());
                }
            }
        });
        if actions.selected.is_some() && heading.y < ui.clip_rect().top() + 15.0 {
            ui.scroll_to_rect(
                Rect::from_min_size(heading - vec2(0.0, 15.0), vec2(1.0, 1.0)),
                Some(Align::Min),
            );
        }
        if let Some(active) = active {
            ui.add_space(9.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(label(&active.selected.name, 16.0));
                actions.close = ui.button("Close trailer").clicked();
                actions.fullscreen = ui.button("Fullscreen").clicked();
                if ui.button("Open on YouTube ↗").clicked() {
                    ui.ctx()
                        .open_url(OpenUrl::new_tab(active.selected.watch_url()));
                }
            });
            let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
            let clip = ui.clip_rect();
            ui.painter().rect(
                rect,
                14,
                BACKGROUND,
                Stroke::new(1.0_f32, BORDER),
                StrokeKind::Inside,
            );
            actions.surface = Some((rect.shrink(1.0), clip));
            if !ready {
                let mut overlay = ui.new_child(
                    UiBuilder::new()
                        .id_salt("trailer status")
                        .max_rect(Rect::from_center_size(
                            rect.center(),
                            vec2(width.min(420.0), 40.0),
                        ))
                        .layout(Layout::top_down(Align::Center)),
                );
                if let Some(error) = &active.error {
                    if status(&mut overlay, error, true) {
                        actions.selected = Some(active.selected.clone());
                    }
                } else {
                    loading(&mut overlay, "Loading trailer…");
                }
            }
        }
    }
    if let Some(credits) = details
        .credits
        .as_ref()
        .filter(|credits| !credits.cast.is_empty())
    {
        ui.add_space(25.0);
        detail_rail(ui, "cast", "Cast", width, 146.0, |ui| {
            for member in &credits.cast {
                cast_card(ui, art, network, member);
            }
        });
    }
    actions
}

fn detail_rail(
    ui: &mut Ui,
    key: &str,
    name: &str,
    width: f32,
    tile_width: f32,
    draw: impl FnOnce(&mut Ui),
) {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        ui.set_max_width(width);
        let id = ui.make_persistent_id(key);
        let offset = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(id))
            .unwrap_or(0.0);
        let mut movement = 0.0;
        ui.horizontal(|ui| {
            ui.set_width(width);
            section_heading(ui, name, 23.0, VIOLET);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                movement = rail_arrows(ui)
            });
        });
        ui.add_space(8.0);
        let row = horizontal_rail(ui, offset, movement, tile_width + 16.0)
            .id_salt(id)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.horizontal_top(draw);
            });
        if row.state.offset.x != offset {
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, row.state.offset.x));
        }
    });
}

fn trailer_card(ui: &mut Ui, art: &mut Artwork, network: &Network, trailer: &Trailer) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(256.0, 144.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &trailer.name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let active = response.hovered() || response.has_focus();
        painter.rect_filled(rect, 12, CARD);
        if let Some(image) = art.get(network, true, Some(trailer.thumbnail())) {
            cover(painter, rect, image, 12);
        }
        painter.rect_filled(
            rect,
            12,
            Color32::from_black_alpha(if active { 30 } else { 90 }),
        );
        painter.circle_filled(rect.center() - vec2(0.0, 5.0), 22.0, VIOLET);
        painter.text(
            rect.center() - vec2(0.0, 5.0),
            Align2::CENTER_CENTER,
            "▶",
            strong(17.0),
            TEXT,
        );
        painter.rect_filled(
            Rect::from_min_size(
                rect.left_bottom() - vec2(0.0, 33.0),
                vec2(rect.width(), 33.0),
            ),
            10,
            Color32::from_black_alpha(205),
        );
        let text = fitted_text(
            ui,
            &trailer.name,
            strong(12.0),
            TEXT,
            rect.width() - 18.0,
            1,
        );
        painter.galley(pos2(rect.left() + 9.0, rect.bottom() - 24.0), text, TEXT);
        painter.rect_stroke(
            rect,
            12,
            Stroke::new(1.0_f32, if active { VIOLET } else { BORDER }),
            StrokeKind::Inside,
        );
    }
    response
        .on_hover_text(format!("{} · {}", trailer.kind, trailer.name))
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

fn cast_card(ui: &mut Ui, art: &mut Artwork, network: &Network, member: &catalog::CastMember) {
    let (rect, response) = ui.allocate_exact_size(vec2(146.0, 245.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            12,
            PANEL,
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        let portrait = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(144.0, 194.0));
        painter.rect_filled(portrait, 11, CARD);
        if let Some(image) = art.get(network, true, member.portrait()) {
            cover(painter, portrait, image, 11);
        }
        let name = fitted_text(ui, &member.name, strong(13.0), TEXT, 126.0, 1);
        painter.galley(
            pos2(rect.left() + 10.0, portrait.bottom() + 7.0),
            name,
            TEXT,
        );
        if let Some(role) = member.character.as_deref() {
            let text = fitted_text(ui, role, FontId::proportional(12.0), MUTED, 126.0, 1);
            painter.galley(
                pos2(rect.left() + 10.0, portrait.bottom() + 25.0),
                text,
                MUTED,
            );
        }
    }
    response.on_hover_text(
        member
            .character
            .as_deref()
            .filter(|role| !role.is_empty())
            .map(|role| format!("{} as {role}", member.name))
            .unwrap_or_else(|| member.name.clone()),
    );
}

struct WatchActions {
    rect: Rect,
    clip: Rect,
    play: Option<(u32, u32)>,
    season: Option<u32>,
    stop: bool,
    retry_details: bool,
    fullscreen: bool,
}

const SERVER_NAMES: [&str; 7] = [
    "Server 1 (Multi Language)",
    "Server 2 (Anime)",
    "Server 3",
    "Server 4",
    "Server 5",
    "Server 6",
    "Server 7",
];

fn watch_section(
    ui: &mut Ui,
    detail: &Detail,
    art: &mut Artwork,
    network: &Network,
    server: &mut usize,
    playback: Option<&Playback>,
    ready: bool,
) -> WatchActions {
    let title = detail.title();
    if title.upcoming() {
        ui.label(label("Coming Soon", 24.0));
        ui.add_space(8.0);
        let width = ui.available_width().min(1152.0);
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, (width * 0.30).clamp(170.0, 300.0)),
            Sense::hover(),
        );
        ui.painter().rect(
            rect,
            16,
            Color32::from_rgb(20, 14, 33),
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center() - vec2(0.0, 16.0),
            Align2::CENTER_CENTER,
            "Not released yet",
            strong(29.0),
            TEXT,
        );
        let date = title
            .premiere_date()
            .map(|date| format!("Scheduled for {date}"))
            .unwrap_or_else(|| "Release date to be announced".to_owned());
        ui.painter().text(
            rect.center() + vec2(0.0, 25.0),
            Align2::CENTER_CENTER,
            date,
            FontId::proportional(15.0),
            LAVENDER,
        );
        return WatchActions {
            rect,
            clip: ui.clip_rect(),
            play: None,
            season: None,
            stop: false,
            retry_details: false,
            fullscreen: false,
        };
    }
    let first = match (&detail.episodes, detail.season) {
        (Load::Ready(episodes), Some(season)) => episodes
            .first()
            .map(|episode| (season, episode.episode_number)),
        _ if title.kind() == Kind::Movie => Some((1, 1)),
        _ => None,
    };
    let active = playback.map(|playback| playback.episode);
    let mut play = None;
    let mut stop = false;
    let mut choose_season = None;
    let mut retry_details = false;
    let mut fullscreen = false;

    let watch_heading = ui.label(label("Watch Now", 24.0)).rect;
    ui.add_space(5.0);
    let previous_server = *server;
    ui.horizontal_wrapped(|ui| {
        ui.label(muted("Server:"));
        ComboBox::from_id_salt("watch server")
            .width(208.0)
            .selected_text(SERVER_NAMES[*server])
            .show_ui(ui, |ui| {
                for (index, name) in SERVER_NAMES.into_iter().enumerate() {
                    ui.selectable_value(server, index, name);
                }
            });
        if ready {
            ui.label(RichText::new("Vortex Shield on").size(12.0).color(LAVENDER));
        }
        if active.is_some() {
            stop = ui.button("Stop").clicked();
            fullscreen = ui
                .button("Fullscreen")
                .on_hover_text("Fill the display with the player")
                .clicked();
        }
        screen_cast::button(ui);
    });
    if previous_server != *server && playback.is_some_and(|state| state.error.is_some()) {
        play = active;
    }
    ui.label(
        RichText::new("Use Server 2 for anime. Try Server 3 if other servers fail.")
            .size(12.0)
            .color(MUTED)
            .italics(),
    );
    ui.add_space(6.0);

    let width = ui.available_width().min(1152.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 16, CARD);
    if !ready {
        if let Some(image) = art.get(network, ui.is_rect_visible(rect), title.backdrop()) {
            cover(&painter, rect, image, 16);
        }
        painter.rect_filled(rect, 16, Color32::from_black_alpha(169));
        let accent = Color32::from_rgb(255, 204, 17);
        let caption = fitted_text(ui, title.name(), strong(28.0), TEXT, rect.width() - 48.0, 1);
        painter.galley(rect.left_top() + vec2(28.0, 26.0), caption, TEXT);
        let genres = title
            .genres
            .iter()
            .map(|genre| genre.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subtitle = if genres.is_empty() {
            title.kind().label().to_owned()
        } else {
            format!("{} · {genres}", title.kind().label())
        };
        let caption = fitted_text(ui, &subtitle, strong(14.0), accent, rect.width() - 48.0, 1);
        painter.galley(rect.left_top() + vec2(28.0, 64.0), caption, accent);
        if let Some(error) = playback.and_then(|state| state.error.as_deref()) {
            let controls =
                Rect::from_center_size(rect.center(), vec2((width - 32.0).min(440.0), 88.0));
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("player status")
                    .max_rect(controls)
                    .layout(Layout::top_down(Align::Center)),
            );
            if status(&mut child, error, true) {
                play = active;
            }
        } else if active.is_some() {
            let controls = Rect::from_center_size(rect.center(), vec2(235.0, 36.0));
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("player startup")
                    .max_rect(controls),
            );
            loading(&mut child, "Starting player…");
        } else if let Some(target) = first {
            let click = Rect::from_center_size(rect.center(), vec2(80.0, 80.0));
            let response = ui.interact(
                click,
                ui.make_persistent_id("start inline player"),
                Sense::click(),
            );
            let label = if title.kind() == Kind::Tv {
                format!("Play S{} E{}", target.0, target.1)
            } else {
                "Play movie".to_owned()
            };
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
            painter.add(
                egui::epaint::RectShape::filled(click.expand(4.0), 45, accent.gamma_multiply(0.48))
                    .with_blur_width(28.0),
            );
            painter.circle_filled(
                click.center(),
                40.0,
                if response.hovered() {
                    Color32::from_rgb(255, 224, 85)
                } else {
                    accent
                },
            );
            let center = click.center();
            painter.add(Shape::convex_polygon(
                [
                    center + vec2(-11.0, -19.0),
                    center + vec2(20.0, 0.0),
                    center + vec2(-11.0, 19.0),
                ]
                .into(),
                Color32::BLACK,
                Stroke::NONE,
            ));
            if response
                .on_hover_text(label)
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                play = Some(target);
            }
        }
    }
    painter.rect_stroke(rect, 16, Stroke::new(1.0_f32, BORDER), StrokeKind::Inside);
    let clip = ui.clip_rect();
    ui.add_space(18.0);
    match &detail.metadata {
        Load::Failed(error) => retry_details = status(ui, error, true),
        Load::Loading(_) => loading(ui, "Loading details…"),
        _ => {}
    }

    if title.kind() == Kind::Tv && matches!(detail.metadata, Load::Ready(_)) {
        ui.add_space(15.0);
        ui.horizontal_wrapped(|ui| {
            section_heading(ui, "Episodes", 24.0, VIOLET);
            ui.add_space(12.0);
            for season in title
                .seasons
                .iter()
                .filter(|season| season.episode_count > 0)
            {
                if filter_chip(
                    ui,
                    &season.name,
                    detail.season == Some(season.season_number),
                ) && detail.season != Some(season.season_number)
                {
                    choose_season = Some(season.season_number);
                }
            }
        });
        ui.add_space(10.0);
        match (&detail.episodes, detail.season) {
            (Load::Ready(episodes), Some(season)) if !episodes.is_empty() => {
                let columns = if width >= 760.0 { 2 } else { 1 };
                let gap = 16.0;
                let row_count = episodes.len().div_ceil(columns);
                let stride = 110.0;
                let content_height = row_count as f32 * stride - 14.0;
                let height = content_height.min(600.0);
                let colors = ui.visuals().widgets.clone();
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = vec2(gap, 14.0);
                    let scroll = &mut ui.spacing_mut().scroll;
                    scroll.floating = false;
                    scroll.bar_width = 7.0;
                    scroll.bar_inner_margin = 6.0;
                    scroll.bar_outer_margin = 0.0;
                    scroll.foreground_color = false;
                    ui.visuals_mut().widgets.inactive.bg_fill = VIOLET.gamma_multiply(0.52);
                    ui.visuals_mut().widgets.hovered.bg_fill = VIOLET;
                    ui.visuals_mut().widgets.active.bg_fill = LAVENDER;
                    ScrollArea::vertical()
                        .id_salt(("episode grid", season))
                        .scroll_source(
                            scroll_area::ScrollSource::SCROLL_BAR
                                | scroll_area::ScrollSource::MOUSE_WHEEL,
                        )
                        .max_width(width)
                        .max_height(height)
                        .min_scrolled_height(height)
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(if content_height > height {
                            scroll_area::ScrollBarVisibility::AlwaysVisible
                        } else {
                            scroll_area::ScrollBarVisibility::AlwaysHidden
                        })
                        .show_rows(ui, 96.0, row_count, |ui, visible_rows| {
                            ui.visuals_mut().widgets = colors;
                            let card_width = ((ui.available_width() - gap * (columns - 1) as f32)
                                / columns as f32)
                                .floor();
                            for row in visible_rows {
                                let start = row * columns;
                                let end = (start + columns).min(episodes.len());
                                ui.push_id(row, |ui| {
                                    ui.horizontal_top(|ui| {
                                        for episode in &episodes[start..end] {
                                            let episode_key = (season, episode.episode_number);
                                            if episode_card(
                                                ui,
                                                art,
                                                network,
                                                episode,
                                                card_width,
                                                active == Some(episode_key),
                                            ) {
                                                play = Some(episode_key);
                                            }
                                        }
                                    });
                                });
                            }
                        });
                });
            }
            (Load::Loading(_), _) => loading(ui, "Loading episodes…"),
            (Load::Failed(error), current) => {
                if status(ui, error, true) {
                    choose_season = current;
                }
            }
            _ => {
                status(ui, "No episodes are available.", false);
            }
        }
        if play.is_some() && watch_heading.top() - 14.0 < clip.top() {
            ui.scroll_to_rect(watch_heading.expand2(vec2(0.0, 14.0)), Some(Align::Min));
        }
    }
    WatchActions {
        rect: rect.shrink(1.0),
        clip,
        play,
        season: choose_season,
        stop,
        retry_details,
        fullscreen,
    }
}

fn fitted_text(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
    lines: usize,
) -> std::sync::Arc<Galley> {
    let mut job = text::LayoutJob::simple(text.to_owned(), font, color, width);
    job.wrap.max_rows = lines;
    ui.painter().layout_job(job)
}

fn loading(ui: &mut Ui, message: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(muted(message));
    });
}

fn status(ui: &mut Ui, message: &str, retry: bool) -> bool {
    Frame::new()
        .fill(PANEL)
        .corner_radius(9)
        .inner_margin(13)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(muted(message));
                retry && ui.button("Try again").clicked()
            })
            .inner
        })
        .inner
}

fn poster_grid(
    ui: &mut Ui,
    art: &mut Artwork,
    network: &Network,
    titles: &[Title],
    preferred_width: f32,
) -> Option<Title> {
    let available = ui.available_width();
    let gap = 24.0;
    ui.spacing_mut().item_spacing = vec2(gap, 17.0);
    let columns = ((available + gap) / (preferred_width - 3.0 + gap))
        .floor()
        .max(1.0) as usize;
    let width = ((available - (columns - 1) as f32 * gap) / columns as f32)
        .min(preferred_width + 36.0)
        .floor();
    let mut selected = None;
    visible_grid(
        ui,
        titles,
        columns,
        width * 1.5 + 47.0,
        17.0,
        |ui, title| {
            if poster(ui, art, network, title, width) {
                selected = Some(title.clone());
            }
        },
    );
    selected
}

fn visible_grid<T>(
    ui: &mut Ui,
    items: &[T],
    columns: usize,
    row_height: f32,
    row_gap: f32,
    mut draw: impl FnMut(&mut Ui, &T),
) {
    if items.is_empty() {
        return;
    }
    let stride = row_height + row_gap;
    let rows = items.len().div_ceil(columns);
    let width = ui.available_width();
    let (grid, _) =
        ui.allocate_exact_size(vec2(width, rows as f32 * stride - row_gap), Sense::hover());
    let visible = grid.intersect(ui.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let first = ((visible.top() - grid.top()) / stride).floor() as usize;
    let end = (((visible.bottom() - grid.top()) / stride).ceil() as usize).min(rows);
    for row in first..end {
        let start = row * columns;
        let stop = (start + columns).min(items.len());
        let rect = Rect::from_min_size(
            grid.min + vec2(0.0, row as f32 * stride),
            vec2(width, row_height),
        );
        let mut cells = ui.new_child(
            UiBuilder::new()
                .id_salt(("grid row", row))
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Min)),
        );
        for item in &items[start..stop] {
            draw(&mut cells, item);
        }
    }
}

fn poster(ui: &mut Ui, art: &mut Artwork, network: &Network, title: &Title, width: f32) -> bool {
    let height = width * 1.5;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height + 47.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, title.name()));
    if ui.is_rect_visible(rect) {
        let picture = Rect::from_min_size(rect.min, vec2(width, height));
        let painter = ui.painter();
        let active = response.hovered() || response.has_focus();
        let upcoming = title.upcoming();
        if active {
            painter.add(
                egui::epaint::RectShape::filled(
                    picture.expand(2.0),
                    14,
                    VIOLET.gamma_multiply(0.45),
                )
                .with_blur_width(17.0),
            );
        }
        painter.rect_filled(picture, 12, CARD);
        if let Some(image) = art.get(network, true, title.poster()) {
            cover(painter, picture, image, 12);
        } else {
            painter.text(
                picture.center(),
                Align2::CENTER_CENTER,
                "VORTEX",
                strong(11.0),
                MUTED,
            );
        }
        if upcoming {
            let badge = Rect::from_min_size(picture.min + vec2(8.0, 8.0), vec2(83.0, 22.0));
            painter.rect_filled(badge, 5, Color32::from_black_alpha(215));
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "Coming Soon",
                strong(11.0),
                LAVENDER,
            );
        }
        if active && !upcoming {
            let overlay = Rect::from_min_max(
                pos2(picture.left(), picture.bottom() - 54.0),
                picture.max - vec2(1.0, 3.0),
            );
            fade(
                painter,
                overlay,
                [
                    Color32::TRANSPARENT,
                    Color32::TRANSPARENT,
                    Color32::from_black_alpha(220),
                    Color32::from_black_alpha(220),
                ],
            );
            painter.text(
                picture.left_bottom() + vec2(12.0, -11.0),
                Align2::LEFT_BOTTOM,
                "Watch Now",
                strong(12.0),
                TEXT,
            );
        }
        let edge = if active { VIOLET } else { BORDER };
        painter.rect_stroke(picture, 12, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
        let name_color = if active { CYAN } else { SOFT_TEXT };
        painter.galley(
            pos2(rect.left(), picture.bottom() + 10.0),
            fitted_text(
                ui,
                title.name(),
                FontId::proportional(14.0),
                name_color,
                width - 2.0,
                1,
            ),
            name_color,
        );
        if let Some(rating) = title.rating() {
            let point = pos2(rect.left(), picture.bottom() + 30.0);
            painter.text(
                point,
                Align2::LEFT_TOP,
                "★",
                FontId::proportional(12.0),
                VIOLET,
            );
            painter.text(
                point + vec2(15.0, 0.0),
                Align2::LEFT_TOP,
                rating,
                FontId::proportional(12.0),
                MUTED,
            );
        }
    }
    response
        .on_hover_text(title.name())
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

fn episode_card(
    ui: &mut Ui,
    art: &mut Artwork,
    network: &Network,
    episode: &Episode,
    width: f32,
    active: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 96.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, active, &episode.name));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() || response.has_focus();
        let painter = ui.painter();
        let edge = if active || hovered { VIOLET } else { BORDER };
        painter.rect(
            rect,
            12,
            if active {
                Color32::from_rgb(25, 16, 38)
            } else {
                PANEL
            },
            Stroke::new(1.0_f32, edge),
            StrokeKind::Inside,
        );
        let image_width = (width * 0.32).clamp(106.0, 160.0);
        let picture = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(image_width, 92.0));
        painter.rect_filled(picture, 10, CARD);
        if let Some(image) = art.get(network, ui.is_rect_visible(picture), episode.still()) {
            cover(painter, picture, image, 10);
        }
        if hovered || active {
            painter.rect_filled(picture, 10, Color32::from_black_alpha(84));
            painter.text(
                picture.center(),
                Align2::CENTER_CENTER,
                "▶",
                strong(27.0),
                TEXT,
            );
        }
        let number = format!("E{}", episode.episode_number);
        let badge =
            Rect::from_min_size(picture.right_bottom() - vec2(35.0, 23.0), vec2(29.0, 17.0));
        painter.rect_filled(badge, 4, Color32::from_black_alpha(203));
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            number,
            strong(10.0),
            TEXT,
        );
        let x = picture.right() + 12.0;
        let available = rect.right() - x - 10.0;
        let name_color = if active || hovered { LAVENDER } else { TEXT };
        painter.galley(
            pos2(x, rect.top() + 11.0),
            fitted_text(ui, &episode.name, strong(14.0), name_color, available, 1),
            name_color,
        );
        if let Some(text) = episode.overview.as_deref() {
            painter.galley(
                pos2(x, rect.top() + 34.0),
                fitted_text(ui, text, FontId::proportional(12.0), MUTED, available, 2),
                MUTED,
            );
        }
        if let Some(date) = episode.air_date.as_deref() {
            painter.text(
                pos2(x, rect.bottom() - 13.0),
                Align2::LEFT_BOTTOM,
                date,
                FontId::proportional(10.0),
                MUTED,
            );
        }
    }
    response
        .on_hover_text(&episode.name)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}
