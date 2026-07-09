use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// Frontend files compiled into the binary — the server has no working-dir
/// dependency and `nix run` works from anywhere.
pub async fn index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], include_str!("../../web/index.html"))
}

pub async fn style() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../../web/style.css"))
}

/// GET /materials/{name} — embedded PBR texture sets (ambientCG 2K) used
/// by the viewer's terrain material.
pub async fn material(Path(name): Path<String>) -> Result<Response, StatusCode> {
    let body: &'static [u8] = match name.as_str() {
        "Ground003_2K-PNG_Color.png" => include_bytes!("../../web/materials/Ground003_2K-PNG_Color.png"),
        "Ground003_2K-PNG_NormalGL.png" => include_bytes!("../../web/materials/Ground003_2K-PNG_NormalGL.png"),
        "Ground003_2K-PNG_Roughness.png" => include_bytes!("../../web/materials/Ground003_2K-PNG_Roughness.png"),
        "Ground003_2K-PNG_Displacement.png" => include_bytes!("../../web/materials/Ground003_2K-PNG_Displacement.png"),
        "Ground037_2K-PNG_Color.png" => include_bytes!("../../web/materials/Ground037_2K-PNG_Color.png"),
        "Ground037_2K-PNG_NormalGL.png" => include_bytes!("../../web/materials/Ground037_2K-PNG_NormalGL.png"),
        "Ground037_2K-PNG_Roughness.png" => include_bytes!("../../web/materials/Ground037_2K-PNG_Roughness.png"),
        "Ground037_2K-PNG_Displacement.png" => include_bytes!("../../web/materials/Ground037_2K-PNG_Displacement.png"),
        "Rock051_2K-PNG_Color.png" => include_bytes!("../../web/materials/Rock051_2K-PNG_Color.png"),
        "Rock051_2K-PNG_NormalGL.png" => include_bytes!("../../web/materials/Rock051_2K-PNG_NormalGL.png"),
        "Rock051_2K-PNG_Roughness.png" => include_bytes!("../../web/materials/Rock051_2K-PNG_Roughness.png"),
        "Rock051_2K-PNG_Displacement.png" => include_bytes!("../../web/materials/Rock051_2K-PNG_Displacement.png"),
        _ => return Err(StatusCode::NOT_FOUND),
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response())
}

/// GET /js/{*path} — the embedded JS modules.
pub async fn js(Path(name): Path<String>) -> Result<Response, StatusCode> {
    let body = match name.as_str() {
        "app.js" => include_str!("../../web/app.js"),
        "start.js" => include_str!("../../web/start.js"),
        "browse.js" => include_str!("../../web/browse.js"),
        "viewer.js" => include_str!("../../web/viewer.js"),
        "near.js" => include_str!("../../web/near.js"),
        "far.js" => include_str!("../../web/far.js"),
        "ttm.js" => include_str!("../../web/ttm.js"),
        "terrain-material.js" => include_str!("../../web/terrain-material.js"),
        "terrain-simple.js" => include_str!("../../web/terrain-simple.js"),
        "editor/editor.js" => include_str!("../../web/editor/editor.js"),
        "editor/toolbar.js" => include_str!("../../web/editor/toolbar.js"),
        "editor/brush.js" => include_str!("../../web/editor/brush.js"),
        "editor/sculpt.js" => include_str!("../../web/editor/sculpt.js"),
        "editor/classes.js" => include_str!("../../web/editor/classes.js"),
        "editor/meshes.js" => include_str!("../../web/editor/meshes.js"),
        "editor/spline.js" => include_str!("../../web/editor/spline.js"),
        "editor/aerial.js" => include_str!("../../web/editor/aerial.js"),
        "editor/lasso.js" => include_str!("../../web/editor/lasso.js"),
        "editor/overlay.js" => include_str!("../../web/editor/overlay.js"),
        "editor/scatter.js" => include_str!("../../web/editor/scatter.js"),
        "editor/ground.js" => include_str!("../../web/editor/ground.js"),
        "editor/plots.js" => include_str!("../../web/editor/plots.js"),
        _ => return Err(StatusCode::NOT_FOUND),
    };
    Ok(([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], body).into_response())
}
