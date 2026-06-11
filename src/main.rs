mod api;
mod ai;
mod config;
mod mcp;

use tracing;
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive},
        Sse,
    },
    routing::post,
    Json, Router,
};
use config::Config;
use mcp::client::McpClient;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

/// 应用共享状态
struct AppState {
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载 .env 文件（敏感信息放这里，不写 config.yaml）
    let _ = dotenvy::dotenv();

    // 加载配置（环境变量会自动覆盖 config.yaml 中的敏感字段）
    let config = Arc::new(Config::load("config.yaml")?);
    
    // 初始化日志（用配置中的日志级别）
    let log_level = config.server.log_level.to_lowercase();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(format!("build_wise={}", log_level).parse()?)
        )
        .init();
    
    tracing::info!("配置加载完成，端口: {}，日志级别: {}", config.server.port, log_level);

    // 启动 MCP 子进程并连接
    tracing::info!("正在连接 MCP Server...");
    let mcp_client = McpClient::connect(
        &config.mcp.server_command,
        &config.mcp.server_args,
        &config.mcp_mongodb_uri,
    )
    .await?;
    tracing::info!("MCP Server 已就绪");

    let state = Arc::new(AppState {
        config,
        mcp_client,
    });

    let app = Router::new()
        .route("/api/chat", post(chat_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{}", state.config.server.port);
    tracing::info!("服务启动于 {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<api::chat::ChatRequest>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    tracing::info!("收到请求，模型 IDs: {:?}", request.model_ids);
    let config = state.config.clone();
    let mcp = state.mcp_client.clone();

    let rx = api::chat::run_analysis(config, mcp, request)
        .await
        .unwrap_or_else(|e| {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let _ = tx.try_send(api::chat::SseEvent::Error(format!("请求处理失败: {}", e)));
            rx
        });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|event| Ok(Event::default().data(event.to_json())));

    Sse::new(stream).keep_alive(KeepAlive::default())
}
