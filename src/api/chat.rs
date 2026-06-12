use crate::ai::ConversationDriver;
use crate::config::Config;
use crate::mcp::client::McpClient;
use anyhow::Result;
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive},
        Sse,
    },
    Json,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

#[derive(Debug, Clone)]
pub enum SseEvent {
    Thinking(String),
    Content(String),
    Done,
    Error(String),
}

impl SseEvent {
    pub fn to_json(&self) -> String {
        match self {
            SseEvent::Thinking(text) => {
                serde_json::json!({"type":"thinking","text":text}).to_string()
            }
            SseEvent::Content(text) => {
                serde_json::json!({"type":"content","text":text}).to_string()
            }
            SseEvent::Done => serde_json::json!({"type":"done"}).to_string(),
            SseEvent::Error(msg) => serde_json::json!({"type":"error","text":msg}).to_string(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub model_ids: Option<Vec<String>>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 应用共享状态
pub struct AppState {
    pub config: Arc<Config>,
    pub mcp_client: Arc<McpClient>,
    pub cancel_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
}

// ── 分析入口 ───────────────────────────────────────────────────

pub async fn run_analysis(
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
    request: ChatRequest,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<mpsc::Receiver<SseEvent>> {
    let (tx, rx) = mpsc::channel(64);

    let history: Vec<(String, String)> = request
        .messages
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();


    tokio::spawn(async move {
        let driver = ConversationDriver::new(
            config,
            mcp_client,
            request.model_ids,
            history,
            tx.clone(),
            cancel,
        );
        match driver.run().await {
            Ok(()) => {
                let _ = tx.send(SseEvent::Done).await;
            }
            Err(e) => {
                tracing::error!("分析失败: {:?}", e);
                let _ = tx.send(SseEvent::Error(format!("系统错误: {}", e))).await;
            }
        }
    });

    Ok(rx)
}

// ── HTTP handlers ──────────────────────────────────────────────

pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
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

    let rx = run_analysis(config, mcp, request, cancel)
        .await
        .unwrap_or_else(|e| {
            let (tx, rx) = mpsc::channel(1);
            let _ = tx.try_send(SseEvent::Error(format!("请求处理失败: {}", e)));
            rx
        });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|event| Ok(Event::default().data(event.to_json())));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn cancel_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut guard = state.cancel_token.lock().await;
    if let Some(token) = guard.take() {
        token.cancel();
        tracing::info!("已取消当前分析任务");
        Json(serde_json::json!({"ok": true, "message": "已取消"}))
    } else {
        Json(serde_json::json!({"ok": true, "message": "无进行中的任务"}))
    }
}
