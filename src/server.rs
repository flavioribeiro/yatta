use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::Stdio;
use std::sync::Arc;

use axum::{response, response::Html, Router, routing::get};
use axum::extract::Extension;
use axum::http::{header, Method, StatusCode};
use axum::response::IntoResponse;
use gst::glib;
use gst::glib::bitflags::Flags;
use gst::prelude::*;
use once_cell::unsync::Lazy;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

type SharedState = Arc<RwLock<State>>;

struct State {
    pipeline: glib::WeakRef<gst::Pipeline>,
}

impl State {
    fn new(pipeline: glib::WeakRef<gst::Pipeline>) -> Self {
        Self { pipeline }
    }
}

pub async fn run(port: u16, pipeline_weak: glib::WeakRef<gst::Pipeline>) {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
    // add CORS headers to files
    let cors = CorsLayer::permissive();
    let hls_dir = ServeDir::new("hls_live_stream");
    let router = Router::new()
        .route("/healthcheck", get(healthcheck))
        .route("/pipeline-diagram", get(pipeline_diagram))
        .route("/pipeline-diagram.png", get(pipeline_diagram_image))
        .nest_service("/live", hls_dir.clone())
        .layer(cors)
        .layer(
            ServiceBuilder::new()
                .layer(Extension(SharedState::new(RwLock::new(State::new(
                    pipeline_weak,
                )))))
                .into_inner(),
        );
    //let server = Server::bind(&addr).serve(router.into_make_service());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    let server = axum::serve(listener, router.into_make_service());
    if let Err(e) = server.await {
        log::error!("server error: {}", e);
    }
}

async fn healthcheck(Extension(state): Extension<SharedState>) -> Html<String> {
    if let Some(_pipeline) = &state.read().await.pipeline.upgrade() {
        Html("<h1>Info</h1><br>Add some interesting stats here...".into())
    } else {
        Html("<h1>Pipeline gone...</h1>".into())
    }
}

async fn pipeline_diagram(Extension(state): Extension<SharedState>) -> Html<String> {
    if let Some(pipeline) = &state.read().await.pipeline.upgrade() {
        Html(dot_graph(pipeline))
    } else {
        Html("<h1>Pipeline gone...</h1>".into())
    }
}

async fn pipeline_diagram_image(Extension(state): Extension<SharedState>) -> impl IntoResponse {
    if let Some(pipeline) = &state.read().await.pipeline.upgrade() {
        let headers = response::AppendHeaders([(header::CONTENT_TYPE, "image/png")]);

        let mut dot_cmd = Command::new("dot")
            .arg("-Tpng")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Sub-command error: {:?}", err.to_string()),
                )
            })?;

        dot_cmd
            .stdin
            .take()
            .as_mut()
            .expect("Command to accept stdin")
            .write_all(dot_graph(pipeline).as_bytes())
            .await
            .expect("Write dot graph contents to pipe");

        let res = dot_cmd
            .wait_with_output()
            .await
            .expect("Wait final execution of dot command");

        if !res.status.success() {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Command exited: {:?}", res.status.to_string()),
            ));
        }

        Ok((headers, res.stdout))
    } else {
        Err((StatusCode::NOT_FOUND, "Pipeline is gone!".to_string()))
    }
}

pub fn dot_graph(pipeline: &gst::Pipeline) -> String {
    pipeline
        .debug_to_dot_data(gst::DebugGraphDetails::ALL)
        .to_string()
}
