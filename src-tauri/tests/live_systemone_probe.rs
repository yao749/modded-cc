//! `/v1/systemone` 入口的真机 live 验证 — 打**运行中的 cc-router**, 默认全 ignored.
//!
//! 前置: model-jev 至少绑定一条可用的 System One 订阅。
//! 环境变量:
//!   LIVE_CC_ROUTER_BASE    默认 http://127.0.0.1:23456
//!   LIVE_CC_ROUTER_TOKEN   cc-router 设置页的 token (关闭鉴权时可不设)
//!   LIVE_SYSTEMONE_MODEL   客户端写的 model, 默认 clef-flash
//!
//! 运行: cargo test --test live_systemone_probe -- --ignored --nocapture

use serde_json::{json, Value};

fn base() -> String {
    std::env::var("LIVE_CC_ROUTER_BASE").unwrap_or_else(|_| "http://127.0.0.1:23456".into())
}

fn model() -> String {
    std::env::var("LIVE_SYSTEMONE_MODEL").unwrap_or_else(|_| "clef-flash".into())
}

async fn post(body: &Value) -> (u16, Value) {
    let mut req = reqwest::Client::new()
        .post(format!("{}/v1/systemone", base()))
        .header("content-type", "application/json")
        .json(body);
    if let Ok(t) = std::env::var("LIVE_CC_ROUTER_TOKEN") {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.expect("cc-router 没有在运行?");
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    println!("[{status}] {text}");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore]
async fn official_noul_example_succeeds() {
    let (status, body) = post(&json!({
        "model": model(),
        "state": "Hello World",
        "questions": { "says_hello": {
            "type": "noul",
            "instructions": "Does the state text contain a greeting?",
            "criteria": { "true": "The state text contains a greeting.", "false": "The state text does not contain a greeting." }
        }}
    }))
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["answers"]["says_hello"]["type"], "noul");
    assert!(body["usage"]["input_tokens"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
#[ignore]
async fn three_question_types_succeed() {
    let (status, body) = post(&json!({
        "model": model(),
        "state": "My card was charged twice for one order and I need this fixed today.",
        "questions": {
            "department": { "type": "choice", "instructions": "Which team handles this?",
                "criteria": { "billing": "charges and refunds", "shipping": "delivery", "technical": "bugs" } },
            "urgency": { "type": "score", "instructions": "How urgent is this?", "criteria": ["low", "medium", "high"] },
            "wants_refund": { "type": "noul", "instructions": "Is the customer asking for money back?" }
        }
    }))
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["answers"]["department"]["type"], "choice");
    assert_eq!(body["answers"]["urgency"]["type"], "score");
}

/// 所有上游都会拒绝非法题型: cc-router 逐个试完后返回最后一个上游的 4xx 原文。
#[tokio::test]
#[ignore]
async fn invalid_question_type_returns_upstream_4xx() {
    let (status, _) = post(&json!({
        "model": model(), "state": "x",
        "questions": { "a": { "type": "bogus", "instructions": "y?" } }
    }))
    .await;
    assert!((400..500).contains(&status), "got {status}");
}

#[tokio::test]
#[ignore]
async fn missing_model_is_rejected_locally() {
    let (status, body) = post(&json!({ "state": "x", "questions": {} })).await;
    assert_eq!(status, 400);
    assert_eq!(body["detail"]["error_type"], "invalid_request_error");
}
