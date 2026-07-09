pub mod api;
pub mod assets;
pub mod classes;
pub mod data;
pub mod edit;
pub mod events;
pub mod far;
pub mod glb;
pub mod overview;
pub mod plots;
pub mod project;
pub mod run;
pub mod scatter;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::Router;

pub async fn serve(host: String, port: u16) -> anyhow::Result<()> {
    let state = state::AppState::new();
    let app = Router::new()
        .route("/", get(assets::index))
        .route("/style.css", get(assets::style))
        .route("/js/{*path}", get(assets::js))
        .route("/materials/{name}", get(assets::material))
        .route("/api/status", get(api::status))
        .route("/api/browse", get(api::browse))
        .route("/api/project/new", post(api::project_new))
        .route("/api/open", post(project::open))
        .route("/api/edit/height", post(edit::height))
        .route("/api/edit/spline", post(edit::spline))
        .route("/api/edit/conform", post(edit::conform))
        .route("/api/scatter", put(scatter::put))
        .route("/api/plots", put(plots::put))
        .route("/api/classes", get(classes::list).put(classes::put))
        .route("/api/classes/paint", post(classes::paint))
        .route(
            "/api/classes/{id}/material/{name}",
            post(classes::upload).layer(DefaultBodyLimit::max(classes::MAX_UPLOAD)),
        )
        .route("/api/classes/{id}/material/{name}/delete", post(classes::remove))
        .route("/api/placements", put(project::save_placements))
        .route(
            "/api/assets/{name}",
            post(glb::upload).layer(DefaultBodyLimit::max(glb::MAX_GLB)),
        )
        .route("/api/assets", get(glb::list))
        .route("/api/cancel", post(api::cancel))
        .route("/api/events", get(events::events))
        .route("/data/far.bin", get(far::far_bin))
        .route("/data/overview.png", get(overview::overview))
        .route("/data/{*path}", get(data::file))
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("TerrainTile kjører på http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
