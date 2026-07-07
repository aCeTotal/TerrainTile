pub mod api;
pub mod assets;
pub mod data;
pub mod events;
pub mod far;
pub mod inspect;
pub mod overview;
pub mod run;
pub mod state;

use axum::routing::{get, post};
use axum::Router;

pub async fn serve(host: String, port: u16) -> anyhow::Result<()> {
    let state = state::AppState::new();
    let app = Router::new()
        .route("/", get(assets::index))
        .route("/style.css", get(assets::style))
        .route("/js/{name}", get(assets::js))
        .route("/api/status", get(api::status))
        .route("/api/browse", get(api::browse))
        .route("/api/scan", post(api::scan))
        .route("/api/grid", get(api::grid))
        .route("/api/start", post(api::start))
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
