//! Norge i bilder via the same backend norgeibilder.no itself uses:
//! an ArcGIS token is fetched from the NiB backend with GeoID credentials
//! (Basic auth) and refreshed automatically, then orthophoto tiles are read
//! from the Nibcache tile services (native UTM zones, e.g. EPSG:25833 —
//! the same grid as hoydedata.no, so no reprojection).

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use image::RgbImage;

pub const TOKEN_URL: &str =
    "https://backend-api.klienter-prod-k8s2.norgeibilder.no/token/tilecache";
pub const TILECACHE_BASE: &str = "https://tilecache.norgeibilder.no/arcgis/rest/services";

/// Tokens are short-lived; refresh well before the site's own 10 min cycle.
const TOKEN_MAX_AGE: Duration = Duration::from_secs(8 * 60);

#[derive(Clone, Debug)]
pub struct TileInfo {
    pub origin: (f64, f64),
    pub tile_px: u32,
    /// (level, resolution m/px) as published by the service.
    pub lods: Vec<(u8, f64)>,
    pub wkid: u32,
}

pub struct NibClient {
    agent: ureq::Agent,
    username: String,
    password: String,
    cache_dir: PathBuf,
    token: Mutex<Option<(String, Instant)>>,
    info: Mutex<HashMap<String, TileInfo>>,
}

impl NibClient {
    pub fn new(agent: ureq::Agent, username: String, password: String, cache_dir: PathBuf) -> Self {
        Self {
            agent,
            username,
            password,
            cache_dir,
            token: Mutex::new(None),
            info: Mutex::new(HashMap::new()),
        }
    }

    /// Pick the Nibcache service matching the dataset's UTM zone; other CRS
    /// fall back to UTM33 with coordinate transformation.
    pub fn service_for_epsg(epsg: u32) -> &'static str {
        match epsg {
            25832 => "Nibcache_UTM32_EUREF89_v2",
            25835 => "Nibcache_UTM35_EUREF89_v2",
            _ => "Nibcache_UTM33_EUREF89_v2",
        }
    }

    fn token(&self, force_refresh: bool) -> Result<String> {
        let mut guard = self.token.lock().unwrap();
        if !force_refresh {
            if let Some((tok, at)) = guard.as_ref() {
                if at.elapsed() < TOKEN_MAX_AGE {
                    return Ok(tok.clone());
                }
            }
        }
        let resp = self
            .agent
            .get(TOKEN_URL)
            .set("Authorization", &format!("Basic {}", b64(&format!("{}:{}", self.username, self.password))))
            .set("Accept", "application/json")
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(401 | 403, _) => {
                    anyhow::anyhow!("GeoID-innlogging avvist — sjekk brukernavn/passord")
                }
                e => anyhow::anyhow!("token-henting feilet: {e}"),
            })?;
        let body = resp.into_string()?.trim().to_string();
        let tok = if body.starts_with('{') {
            let v: serde_json::Value = serde_json::from_str(&body)?;
            v.get("token")
                .or_else(|| v.get("access_token"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
                .with_context(|| format!("uventet token-svar: {}", &body[..body.len().min(200)]))?
        } else if body.contains('<') || body.is_empty() {
            bail!("uventet token-svar: {}", &body[..body.len().min(200)]);
        } else {
            body
        };
        *guard = Some((tok.clone(), Instant::now()));
        Ok(tok)
    }

    /// Tiling scheme of one Nibcache service (cached per service).
    pub fn tile_info(&self, service: &str) -> Result<TileInfo> {
        if let Some(i) = self.info.lock().unwrap().get(service) {
            return Ok(i.clone());
        }
        let token = self.token(false)?;
        let url = format!("{TILECACHE_BASE}/{service}/MapServer?f=json&token={token}");
        let body = self.agent.get(&url).call()?.into_string()?;
        let v: serde_json::Value =
            serde_json::from_str(&body).context("ugyldig JSON fra tilecache")?;
        if let Some(err) = v.get("error") {
            bail!("tilecache-feil: {err}");
        }
        let ti = v.get("tileInfo").context("tjenesten mangler tileInfo")?;
        let info = TileInfo {
            origin: (
                ti["origin"]["x"].as_f64().context("tileInfo.origin")?,
                ti["origin"]["y"].as_f64().context("tileInfo.origin")?,
            ),
            tile_px: ti["rows"].as_u64().unwrap_or(256) as u32,
            lods: ti["lods"]
                .as_array()
                .context("tileInfo.lods")?
                .iter()
                .filter_map(|l| {
                    Some((l["level"].as_u64()? as u8, l["resolution"].as_f64()?))
                })
                .collect(),
            wkid: v["spatialReference"]["latestWkid"]
                .as_u64()
                .or_else(|| v["spatialReference"]["wkid"].as_u64())
                .context("spatialReference")? as u32,
        };
        if info.lods.is_empty() {
            bail!("tjenesten {service} har ingen LOD-er");
        }
        self.info.lock().unwrap().insert(service.to_string(), info.clone());
        Ok(info)
    }

    /// One cached tile. An expired token mid-run is refreshed and retried
    /// transparently.
    pub fn get_tile(&self, service: &str, level: u8, row: u64, col: u64) -> Result<RgbImage> {
        let cache = self.cache_dir.join(format!("nib/{service}/{level}/{row}/{col}.bin"));
        if cache.exists() {
            let bytes = std::fs::read(&cache)?;
            if let Ok(img) = image::load_from_memory(&bytes) {
                return Ok(img.to_rgb8());
            }
            let _ = std::fs::remove_file(&cache);
        }
        let mut bytes = self.download_tile(service, level, row, col, false)?;
        if bytes.starts_with(b"{") || bytes.starts_with(b"<") {
            // Most likely an expired/invalid token — refresh once and retry.
            bytes = self.download_tile(service, level, row, col, true)?;
            if bytes.starts_with(b"{") || bytes.starts_with(b"<") {
                bail!(
                    "tjenesten svarte med feil: {}",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
                );
            }
        }
        std::fs::create_dir_all(cache.parent().unwrap())?;
        let tmp = cache.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &cache)?;
        Ok(image::load_from_memory(&bytes).context("ugyldig flisbilde")?.to_rgb8())
    }

    fn download_tile(
        &self,
        service: &str,
        level: u8,
        row: u64,
        col: u64,
        force_refresh: bool,
    ) -> Result<Vec<u8>> {
        let token = self.token(force_refresh)?;
        let url = format!("{TILECACHE_BASE}/{service}/MapServer/tile/{level}/{row}/{col}?token={token}");
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 << attempt));
            }
            match self.agent.get(&url).call() {
                Ok(resp) => {
                    let mut bytes = Vec::new();
                    resp.into_reader().take(32 * 1024 * 1024).read_to_end(&mut bytes)?;
                    return Ok(bytes);
                }
                Err(e) => last_err = Some(e),
            }
        }
        bail!("nedlasting feilet: {}", last_err.unwrap());
    }
}

fn b64(input: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}
