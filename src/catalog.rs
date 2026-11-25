use eframe::egui;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Movie,
    Tv,
    #[serde(other)]
    Other,
}

impl Kind {
    pub fn path(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
            Self::Other => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Movie => "Movie",
            Self::Tv => "TV Show",
            Self::Other => "",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Title {
    pub id: u32,
    pub media_type: Option<Kind>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f32>,
    pub runtime: Option<u32>,
    pub number_of_seasons: Option<u32>,
    pub status: Option<String>,
    #[serde(default)]
    pub seasons: Vec<Season>,
    #[serde(default)]
    pub genres: Vec<Genre>,
}

impl Title {
    pub fn premiere_date(&self) -> Option<&str> {
        self.release_date
            .as_deref()
            .or(self.first_air_date.as_deref())
            .filter(|date| !date.is_empty())
    }

    pub fn upcoming(&self) -> bool {
        self.premiere_date()
            .and_then(parse_date)
            .is_some_and(|date| date > time::OffsetDateTime::now_utc().date())
            || self.kind() == Kind::Movie
                && matches!(
                    self.status.as_deref(),
                    Some("Rumored" | "Planned" | "In Production" | "Post Production")
                )
    }

    pub fn kind(&self) -> Kind {
        self.media_type.unwrap_or(if self.title.is_some() {
            Kind::Movie
        } else {
            Kind::Tv
        })
    }

    pub fn key(&self) -> (Kind, u32) {
        (self.kind(), self.id)
    }

    pub fn name(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Untitled")
    }

    pub fn year(&self) -> &str {
        self.release_date
            .as_deref()
            .or(self.first_air_date.as_deref())
            .and_then(|date| date.get(..4))
            .unwrap_or("")
    }

    pub fn rating(&self) -> Option<String> {
        self.vote_average
            .filter(|rating| *rating > 0.0)
            .map(|rating| format!("{rating:.1}"))
    }

    pub fn poster(&self) -> Option<String> {
        image_url(self.poster_path.as_deref(), "w342")
    }

    pub fn backdrop(&self) -> Option<String> {
        image_url(self.backdrop_path.as_deref(), "w1280")
    }
}

fn parse_date(date: &str) -> Option<time::Date> {
    let (year, rest) = date.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let month = time::Month::try_from(month.parse::<u8>().ok()?).ok()?;
    time::Date::from_calendar_date(year.parse().ok()?, month, day.parse().ok()?).ok()
}

fn image_url(path: Option<&str>, size: &str) -> Option<String> {
    path.filter(|path| path.starts_with('/'))
        .map(|path| format!("https://image.tmdb.org/t/p/{size}{path}"))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Season {
    pub season_number: u32,
    pub name: String,
    pub episode_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Genre {
    pub id: Option<u32>,
    pub name: String,
}

#[derive(Deserialize)]
pub struct DetailInfo {
    #[serde(flatten)]
    pub title: Title,
    pub credits: Option<Credits>,
    pub videos: Option<Videos>,
}

#[derive(Default, Deserialize)]
pub struct Credits {
    pub cast: Vec<CastMember>,
}

#[derive(Deserialize)]
pub struct CastMember {
    pub name: String,
    pub character: Option<String>,
    profile_path: Option<String>,
}

impl CastMember {
    pub fn portrait(&self) -> Option<String> {
        image_url(self.profile_path.as_deref(), "w185")
    }
}

#[derive(Default, Deserialize)]
pub struct Videos {
    pub results: Vec<Trailer>,
}

#[derive(Clone, Deserialize)]
pub struct Trailer {
    key: String,
    pub name: String,
    site: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    official: bool,
}

impl Trailer {
    fn supported(&self) -> bool {
        self.site == "YouTube"
            && matches!(self.kind.as_str(), "Trailer" | "Teaser" | "Clip")
            && self.key.len() == 11
            && self
                .key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    pub fn thumbnail(&self) -> String {
        format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", self.key)
    }

    pub fn embed_url(&self) -> String {
        format!("https://www.youtube.com/embed/{}?autoplay=1", self.key)
    }

    pub fn watch_url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.key)
    }
}

#[derive(Debug, Deserialize)]
pub struct Episode {
    pub episode_number: u32,
    pub name: String,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
}

impl Episode {
    pub fn still(&self) -> Option<String> {
        image_url(self.still_path.as_deref(), "w300")
    }
}

#[derive(Deserialize)]
struct SeasonDetails {
    episodes: Vec<Episode>,
}

#[derive(Deserialize)]
pub struct Provider {
    pub provider_id: u32,
    pub provider_name: String,
    logo_path: Option<String>,
}

impl Provider {
    pub fn logo(&self) -> Option<String> {
        image_url(self.logo_path.as_deref(), "w185")
    }
}

#[derive(Deserialize)]
struct ProviderList {
    results: Vec<Provider>,
}

#[derive(Deserialize)]
struct Catalog {
    results: Vec<Title>,
    total_pages: u32,
}

#[derive(Clone)]
pub struct Query {
    pub path: String,
    pub params: Vec<(String, String)>,
}

impl Query {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            params: Vec::new(),
        }
    }

    pub fn with(mut self, key: &str, value: impl ToString) -> Self {
        self.params.push((key.to_owned(), value.to_string()));
        self
    }
}

pub fn home_catalogs() -> Vec<(&'static str, Query)> {
    let drama = |language| {
        Query::new("discover/tv")
            .with("with_original_language", language)
            .with("without_genres", "10763,10764,10767")
    };
    vec![
        ("Trending Today", Query::new("trending/all/week")),
        ("Popular Movies", Query::new("movie/popular")),
        (
            "Bollywood Blockbusters",
            Query::new("discover/movie").with("with_original_language", "hi|ta|te"),
        ),
        ("Coming Soon", Query::new("movie/upcoming")),
        ("Heart-Racing K-Dramas", drama("ko")),
        ("Top Rated TV Shows", Query::new("tv/top_rated")),
        ("Turkish Delights", drama("tr")),
        ("Trending Anime", anime_catalog()),
        ("Chinese Epics", drama("zh")),
        ("Filipino Favorites", drama("tl")),
        ("Popular TV Shows", Query::new("tv/popular")),
        (
            "Hollywood",
            Query::new("discover/movie").with("with_original_language", "en"),
        ),
        (
            "Cartoons",
            Query::new("discover/tv").with("with_genres", "16,10751"),
        ),
        (
            "Drama",
            Query::new("discover/movie").with("with_genres", 18),
        ),
        ("Now Playing", Query::new("movie/now_playing")),
        ("Top Rated Movies", Query::new("movie/top_rated")),
    ]
}

pub fn anime_catalog() -> Query {
    Query::new("discover/tv").with("with_keywords", 210024)
}

pub enum Job {
    Catalog(u64, Query),
    Details(u64, Kind, u32),
    Episodes(u64, u32, u32),
    Providers(u64),
    Related(u64, Kind, u32),
}

pub enum Reply {
    Catalog(u64, Result<(Vec<Title>, u32), String>),
    Details(u64, Result<Box<DetailInfo>, String>),
    Episodes(u64, Result<Vec<Episode>, String>),
    Providers(u64, Result<Vec<Provider>, String>),
    Related(u64, Result<Vec<Title>, String>),
}

pub struct Network {
    metadata: mpsc::Sender<Job>,
    background: mpsc::Sender<Job>,
    artwork: mpsc::Sender<String>,
    pub replies: mpsc::Receiver<Reply>,
    pub images: mpsc::Receiver<(String, Option<egui::ColorImage>)>,
}

fn worker_pool<Job: Send + 'static, Output: Send + 'static>(
    count: usize,
    ctx: &egui::Context,
    client: &Result<Client, String>,
    output: &mpsc::Sender<Output>,
    run: fn(&Result<Client, String>, Job) -> Output,
) -> mpsc::Sender<Job> {
    let (sender, jobs) = mpsc::channel();
    let jobs = Arc::new(Mutex::new(jobs));
    for _ in 0..count {
        let (jobs, output, ctx, client) =
            (jobs.clone(), output.clone(), ctx.clone(), client.clone());
        thread::spawn(move || {
            loop {
                let job = jobs.lock().expect("job queue").recv();
                let Ok(job) = job else {
                    break;
                };
                if output.send(run(&client, job)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
    sender
}

impl Network {
    pub fn new(ctx: &egui::Context) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(concat!("VortexDesktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| error.to_string());
        let (output, replies) = mpsc::channel();
        let (image_output, images) = mpsc::channel();
        Self {
            metadata: worker_pool(3, ctx, &client, &output, execute),
            background: worker_pool(3, ctx, &client, &output, execute),
            artwork: worker_pool(4, ctx, &client, &image_output, load_image),
            replies,
            images,
        }
    }

    pub fn send(&self, job: Job) {
        let _ = self.metadata.send(job);
    }

    pub fn send_background(&self, job: Job) {
        let _ = self.background.send(job);
    }

    pub fn send_image(&self, url: String) {
        let _ = self.artwork.send(url);
    }
}

fn request<T: DeserializeOwned>(
    client: &Result<Client, String>,
    query: Query,
) -> Result<T, String> {
    let endpoint = option_env!("VORTEX_CATALOG_API_URL")
        .filter(|url| url.starts_with("https://"))
        .ok_or_else(|| "This build has no configured catalog API.".to_owned())?;
    client
        .as_ref()
        .map_err(Clone::clone)?
        .get(endpoint)
        .query(&[("path", query.path)])
        .query(&query.params)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(|error| {
            if let Some(status) = error.status() {
                format!("Vortex returned HTTP {status}.")
            } else if error.is_decode() {
                "Vortex returned an unreadable response.".to_owned()
            } else {
                "Could not connect to Vortex. Check your connection and try again.".to_owned()
            }
        })
}

fn execute(client: &Result<Client, String>, job: Job) -> Reply {
    match job {
        Job::Catalog(token, query) => Reply::Catalog(
            token,
            request::<Catalog>(client, query).map(|catalog| {
                let titles = catalog
                    .results
                    .into_iter()
                    .filter(|title| title.kind() != Kind::Other)
                    .collect();
                (titles, catalog.total_pages.min(500))
            }),
        ),
        Job::Details(token, kind, id) => {
            Reply::Details(token, detail_info(client, kind, id).map(Box::new))
        }
        Job::Episodes(token, id, season) => Reply::Episodes(
            token,
            request::<SeasonDetails>(client, Query::new(format!("tv/{id}/season/{season}")))
                .map(|data| data.episodes),
        ),
        Job::Providers(token) => Reply::Providers(
            token,
            request::<ProviderList>(
                client,
                Query::new("watch/providers/movie").with("watch_region", "US"),
            )
            .map(|data| {
                data.results
                    .into_iter()
                    .filter(|provider| provider.logo_path.is_some())
                    .collect()
            }),
        ),
        Job::Related(token, kind, title_id) => {
            Reply::Related(token, related_titles(client, kind, title_id))
        }
    }
}

fn load_image(client: &Result<Client, String>, url: String) -> (String, Option<egui::ColorImage>) {
    let image = (|| {
        let bytes = client
            .as_ref()
            .ok()?
            .get(&url)
            .timeout(Duration::from_secs(8))
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .ok()?;
        let image = image::load_from_memory(&bytes).ok()?.into_rgba8();
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ))
    })();
    (url, image)
}

fn detail_info(client: &Result<Client, String>, kind: Kind, id: u32) -> Result<DetailInfo, String> {
    let path = format!("{}/{id}", kind.path());
    let mut details: DetailInfo = request(
        client,
        Query::new(&path).with("append_to_response", "credits,videos"),
    )?;
    // Proxies that omit appended resources can still serve the original endpoints.
    if details.credits.is_none() {
        details.credits = request(client, Query::new(format!("{path}/credits"))).ok();
    }
    if details.videos.is_none() {
        details.videos = request(client, Query::new(format!("{path}/videos"))).ok();
    }
    if let Some(credits) = &mut details.credits {
        credits.cast.truncate(12);
    }
    if let Some(videos) = &mut details.videos {
        videos.results.retain(Trailer::supported);
        videos.results.sort_by_key(|video| {
            let order = match video.kind.as_str() {
                "Trailer" => 0,
                "Teaser" => 1,
                _ => 2,
            };
            (order, !video.official)
        });
        videos.results.truncate(12);
    }
    Ok(details)
}

fn related_titles(
    client: &Result<Client, String>,
    kind: Kind,
    id: u32,
) -> Result<Vec<Title>, String> {
    let get_titles = |query| {
        request::<Catalog>(client, query).map(|catalog| {
            catalog
                .results
                .into_iter()
                .filter(|title| title.kind() == kind && title.id != id)
                .collect::<Vec<_>>()
        })
    };
    let path = kind.path();
    for relation in ["recommendations", "similar"] {
        let query = Query::new(format!("{path}/{id}/{relation}")).with("include_adult", false);
        let titles = get_titles(query)?;
        if !titles.is_empty() {
            return Ok(titles);
        }
    }
    let title: Title = request(client, Query::new(format!("{path}/{id}")))?;
    let Some(genre) = title.genres.iter().find_map(|genre| genre.id) else {
        return Ok(Vec::new());
    };
    get_titles(
        Query::new(format!("discover/{path}"))
            .with("with_genres", genre)
            .with("include_adult", false),
    )
}

pub fn server_urls(title: &Title, season: u32, episode: u32) -> [String; 7] {
    let id = title.id;
    let kind = title.kind().path();
    if title.kind() == Kind::Tv {
        [
            format!("https://nxsha.space/embed/tv/{id}/{season}/{episode}"),
            format!("https://rozgarlelo.modiplay.xyz/embed/tmdb/tv?id={id}&s={season}&e={episode}"),
            format!("https://primesrc.me/embed/tv?tmdb={id}&season={season}&episode={episode}"),
            format!("https://ythd.org/embed/{id}/{season}-{episode}"),
            format!("https://vixsrc.to/embed/tv/{id}/{season}/{episode}"),
            format!("https://vidcore.io/embed/tv/{id}/{season}/{episode}"),
            format!("https://vidrock.net/embed/tv/{id}/{season}/{episode}"),
        ]
    } else {
        [
            format!("https://nxsha.space/embed/{kind}/{id}"),
            format!("https://rozgarlelo.modiplay.xyz/embed/tmdb/{kind}?id={id}"),
            format!("https://primesrc.me/embed/{kind}?tmdb={id}"),
            format!("https://ythd.org/embed/{id}"),
            format!("https://vixsrc.to/embed/{kind}/{id}"),
            format!("https://vidcore.io/embed/{kind}/{id}"),
            format!("https://vidrock.net/embed/{kind}/{id}"),
        ]
    }
}
