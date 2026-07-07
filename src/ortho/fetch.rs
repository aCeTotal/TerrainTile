use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use image::RgbImage;

use crate::ortho::source::OrthoSource;

/// Downloads orthophoto data with a persistent disk cache. Cached data makes
/// re-runs and resume free of network traffic.
pub struct Fetcher {
    agent: ureq::Agent,
    src: OrthoSource,
    /// Set when the provider is Norge i bilder (GeoID login).
    pub nib: Option<crate::ortho::nib::NibClient>,
}

impl Fetcher {
    pub fn new(src: OrthoSource) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .user_agent("terraintile/0.1")
            .build();
        let nib = match &src.provider {
            crate::ortho::source::Provider::Nib { username, password } => {
                Some(crate::ortho::nib::NibClient::new(
                    agent.clone(),
                    username.clone(),
                    password.clone(),
                    src.cache_dir.clone(),
                ))
            }
            _ => None,
        };
        Self { agent, src, nib }
    }

    /// One XYZ tile (WebMercator pyramid).
    pub fn get_xyz(&self, url_template: &str, z: u8, x: u64, y: u64) -> Result<RgbImage> {
        let url = url_template
            .replace("{z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string());
        let cache = self.src.cache_dir.join(format!("xyz/{z}/{x}/{y}.bin"));
        self.cached_image(&url, &cache)
    }

    /// WMS GetMap for an exact bbox in the dataset CRS. Missing standard
    /// parameters are appended so the user only supplies the base URL
    /// (with LAYERS and BAAT ticket for Norge i bilder).
    pub fn get_wms(
        &self,
        base_url: &str,
        crs: &str,
        bbox: (f64, f64, f64, f64),
        size: usize,
        cache_key: &str,
    ) -> Result<RgbImage> {
        // The user may paste a full request URL copied from the browser
        // (norgeibilder.no devtools) — drop all per-request parameters and
        // keep only what identifies the service (LAYERS, ticket, ...).
        const OURS: [&str; 9] = [
            "service", "version", "request", "styles", "format", "crs", "srs", "width", "height",
        ];
        let (base, query) = base_url.split_once('?').unwrap_or((base_url, ""));
        let mut url = format!("{base}?");
        for kv in query.split('&').filter(|kv| !kv.is_empty()) {
            let key = kv.split('=').next().unwrap_or("").to_lowercase();
            if key != "bbox" && !OURS.contains(&key.as_str()) {
                url.push_str(kv);
                url.push('&');
            }
        }
        for (k, v) in [
            ("service", "WMS".to_string()),
            ("version", "1.3.0".to_string()),
            ("request", "GetMap".to_string()),
            ("styles", String::new()),
            ("format", "image/png".to_string()),
            ("crs", crs.to_string()),
            ("width", size.to_string()),
            ("height", size.to_string()),
            (
                "bbox",
                format!("{},{},{},{}", bbox.0, bbox.1, bbox.2, bbox.3),
            ),
        ] {
            url.push_str(&format!("{k}={v}&"));
        }
        let url = url.trim_end_matches('&').to_string();
        let cache = self.src.cache_dir.join(format!("wms/{cache_key}.bin"));
        let img = self.cached_image(&url, &cache)?;
        if img.width() as usize != size || img.height() as usize != size {
            bail!(
                "WMS returnerte {}x{} px, forventet {size}x{size} — sjekk URL/ticket",
                img.width(),
                img.height()
            );
        }
        Ok(img)
    }

    fn cached_image(&self, url: &str, cache: &PathBuf) -> Result<RgbImage> {
        let bytes = if cache.exists() {
            std::fs::read(cache)?
        } else {
            let bytes = self.download(url)?;
            std::fs::create_dir_all(cache.parent().unwrap())?;
            let tmp = cache.with_extension("tmp");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, cache)?;
            bytes
        };
        if bytes.starts_with(b"<") {
            // WMS errors come back as XML — don't cache-poison silently.
            let _ = std::fs::remove_file(cache);
            bail!("tjenesten svarte med feil: {}", xml_error_text(&bytes));
        }
        Ok(image::load_from_memory(&bytes)
            .with_context(|| format!("ugyldig bildedata fra {url}"))?
            .to_rgb8())
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 << attempt));
            }
            match self.agent.get(url).call() {
                Ok(resp) => {
                    let mut bytes = Vec::new();
                    resp.into_reader()
                        .take(64 * 1024 * 1024)
                        .read_to_end(&mut bytes)?;
                    return Ok(bytes);
                }
                Err(e) => last_err = Some(e),
            }
        }
        bail!("nedlasting feilet for {url}: {}", last_err.unwrap());
    }
}

/// Human-readable one-liner from a WMS XML error response — the raw XML is
/// multi-line and floods the log.
fn xml_error_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let inner: String = text
        .split('>')
        .filter_map(|part| {
            let content = part.split('<').next()?.trim();
            (!content.is_empty()).then_some(content)
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut msg = if inner.is_empty() { text.into_owned() } else { inner };
    msg = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if msg.len() > 200 {
        msg.truncate(200);
        msg.push('…');
    }
    msg
}
