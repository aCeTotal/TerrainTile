use std::path::PathBuf;

/// Where orthophotos come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Provider {
    /// norgeibilder.no with a GeoID account: tokens are fetched and
    /// refreshed automatically, tiles come from the Nibcache services in
    /// native UTM — pixel-perfect against Norwegian height data.
    Nib { username: String, password: String },
    /// WMS GetMap in the dataset CRS (requires a BAAT ticket in the URL).
    /// Exact bbox per tile = pixel-perfect alignment with the vertex grid.
    Wms { base_url: String },
    /// XYZ tile server in WebMercator (e.g. ESRI World Imagery). No auth.
    Xyz { url_template: String, zoom: u8 },
}

#[derive(Clone, Debug)]
pub struct OrthoSource {
    pub provider: Provider,
    pub cache_dir: PathBuf,
}

pub const DEFAULT_NIB_WMS: &str =
    "https://wms.geonorge.no/skwms1/wms.nib?LAYERS=ortofoto&ticket=DIN_BAAT_TICKET";

pub const DEFAULT_XYZ: &str =
    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
