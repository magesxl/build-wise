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
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

/// Ctrl+C 信号标记（Windows handler 写入，异步代码轮询）
static CTRL_C_PRESSED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

#[cfg(windows)]
unsafe extern "system" fn console_ctrl_handler(_ctrl_type: u32) -> i32 {
    CTRL_C_PRESSED.store(true, Ordering::SeqCst);
    1 // TRUE：已处理，不要终止进程
}

/// 应用共享状态
struct AppState {
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
    cancel_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
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
        mcp_client: mcp_client.clone(),
        cancel_token: tokio::sync::Mutex::new(None),
    });

    let app = Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/cancel", post(cancel_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{}", state.config.server.port);
    tracing::info!("服务启动于 {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // 注册 Ctrl+C 处理器
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(console_ctrl_handler), 1);
    }
    #[cfg(not(windows))]
    {
        tokio::spawn(async {
            tokio::signal::ctrl_c().await.ok();
            CTRL_C_PRESSED.store(true, Ordering::SeqCst);
        });
    }

    // 优雅关闭：轮询 Ctrl+C 标记
    let shutdown_signal = async {
        loop {
            if CTRL_C_PRESSED.load(Ordering::SeqCst) {
                tracing::info!("收到终止信号，正在优雅关闭...");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    // 关闭 MCP 子进程
    mcp_client.shutdown().await;
    tracing::info!("服务已关闭");

    std::process::exit(0);
}

async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<api::chat::ChatRequest>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    tracing::info!("收到请求，模型 IDs: {:?}", request.model_ids);
    let config = state.config.clone();
    let mcp = state.mcp_client.clone();

    // 创建新的取消令牌，替换旧令牌（旧的会被 cancel 掉）
    let cancel = tokio_util::sync::CancellationToken::new();
    {
        let mut guard = state.cancel_token.lock().await;
        if let Some(old) = guard.take() {
            old.cancel();
        }
        *guard = Some(cancel.clone());
    }

    let rx = api::chat::run_analysis(config, mcp, request, cancel)
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

async fn cancel_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let mut guard = state.cancel_token.lock().await;
    if let Some(token) = guard.take() {
        token.cancel();
        tracing::info!("已取消当前分析任务");
        Json(serde_json::json!({"ok": true, "message": "已取消"}))
    } else {
        Json(serde_json::json!({"ok": true, "message": "无进行中的任务"}))
    }
}
