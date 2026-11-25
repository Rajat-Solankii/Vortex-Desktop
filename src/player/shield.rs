use url::Url;

const AD_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "adservice.google.com",
    "googlesyndication.com",
    "adsterra.com",
    "popads.net",
    "popcash.net",
    "propellerads.com",
    "monetag.com",
    "exoclick.com",
    "adkeeper.co.uk",
    "adkeeper.com",
    "counter.yadro.ru",
    "histats.com",
    "onclickpredictiv.com",
    "onclickmega.com",
    "zeroredirect.com",
    "whomeptoo.com",
    "ouo.io",
    "shorte.st",
    "adnxs.com",
    "criteo.com",
    "taboola.com",
    "outbrain.com",
    "mgid.com",
    "revcontent.com",
    "trafficjunky.net",
    "tsyndicate.com",
    "clarium.global",
    "coinhive.com",
    "coin-hive.com",
    "crypto-loot.com",
    "juicyads.com",
    "ad-maven.com",
    "hilltopads.net",
    "clickadu.com",
    "adcash.com",
    "inpagepush.com",
    "pushame.com",
    "propu.sh",
    "pushwelcome.com",
    "vdo.ai",
];

const AD_HOST_TERMS: &[&str] = &["betting", "casino", "1xbet", "mostbet", "parimatch"];

const AD_URL_TERMS: &[&str] = &[
    "/ads.js",
    "/advert",
    "/popunder",
    "/popup.js",
    "/track.js",
    "/analytics.js",
    "/vast.xml",
    "banner_id=",
    "ad_type=",
    "click_id=",
    "campaign_id=",
    "traffic_type=",
    "affiliate_id=",
    "/counter.js",
    "/beacon.js",
];

const STREAM_HOSTS: &[&str] = &[
    "vaplayer.ru",
    "videasy.net",
    "nhdapi.com",
    "tmdb.org",
    "nxsha.space",
    "modiplay.xyz",
    "primesrc.me",
    "ythd.org",
    "vixsrc.to",
    "vidcore.io",
    "vidrock.net",
];

pub const PAGE_SCRIPT: &str = r#"
(() => {
  window.open = () => null;
  window.alert = () => undefined;
  window.confirm = () => false;
  window.prompt = () => null;
  const addStyle = () => {
    const style = document.createElement('style');
    style.textContent = '[data-ad-slot], [data-ad-container], ins.adsbygoogle, .adsbygoogle { display: none !important; }';
    (document.head || document.documentElement).appendChild(style);
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', addStyle, { once: true });
  } else {
    addStyle();
  }
})();
"#;

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_ad(url: &Url) -> bool {
    let url_text = url.as_str().to_ascii_lowercase();
    if AD_URL_TERMS.iter().any(|term| url_text.contains(term)) {
        return true;
    }
    let host = url.host_str().unwrap_or_default().trim_end_matches('.');
    AD_DOMAINS.iter().any(|domain| host_matches(host, domain))
        || AD_HOST_TERMS.iter().any(|term| host.contains(term))
        || host_matches(host, "yandex.ru") && url.path().to_ascii_lowercase().starts_with("/ads")
}

#[cfg(target_os = "windows")]
pub fn blocks_request(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| is_ad(&url))
}

pub fn allows_navigation(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && !is_ad(&url)
        && url
            .host_str()
            .is_some_and(|host| STREAM_HOSTS.iter().any(|domain| host_matches(host, domain)))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn content_rules() -> String {
    let for_domain =
        |domain: &str| format!(r"^https?://([^/?#]+\.)?{}", domain.replace('.', r"\."));
    let patterns = AD_DOMAINS
        .iter()
        .map(|domain| format!(r"{}\.?[/:?#]", for_domain(domain)))
        .chain(
            AD_HOST_TERMS
                .iter()
                .map(|term| format!(r"^https?://[^/?#]*{term}")),
        )
        .chain(AD_URL_TERMS.iter().map(|term| term.replace('.', r"\.")))
        .chain([format!(r"{}\.?(:[0-9]+)?/ads", for_domain("yandex.ru"))]);
    let rules: Vec<_> = patterns
        .map(|pattern| {
            serde_json::json!({
                "trigger": { "url-filter": pattern },
                "action": { "type": "block" },
            })
        })
        .collect();
    serde_json::Value::Array(rules).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_allows_embed_servers_and_denies_external_navigation() {
        assert!(allows_navigation("https://primesrc.me/embed/movie?tmdb=1"));
        assert!(allows_navigation(
            "https://rozgarlelo.modiplay.xyz/embed/tmdb/tv?id=2"
        ));
        assert!(allows_navigation("https://player.videasy.net/movie/1"));
        assert!(!allows_navigation(
            "https://primesrc.me.bad-example.org/watch"
        ));
        assert!(!allows_navigation(
            "https://bad-example.org/?next=https://primesrc.me"
        ));
        assert!(!allows_navigation("https://bad-example.org@primesrc.me"));
        assert!(!allows_navigation("intent://primesrc.me/watch"));
        assert!(!allows_navigation("file:///C:/test.html"));
        assert!(!allows_navigation("https://primesrc.me/popup.js"));
    }

    #[test]
    fn shield_blocks_ads_in_subresource_urls() {
        assert!(is_ad(
            &Url::parse("https://ads.doubleclick.net./resource").unwrap()
        ));
        assert!(is_ad(
            &Url::parse("https://video.example/ANALYTICS.JS").unwrap()
        ));
        assert!(is_ad(&Url::parse("https://yandex.ru/ads/banner").unwrap()));
        assert!(!is_ad(
            &Url::parse("https://cdn.example/media/chunk.m4s").unwrap()
        ));
        assert!(!is_ad(
            &Url::parse("https://doubleclick.net.example/media").unwrap()
        ));
    }
}
