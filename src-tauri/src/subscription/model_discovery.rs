//! 调用 `{base_url}/v1/models` 并缓存结果（设计稿 §8）。
//!
//! snapshot 模型: 全部连接信息从订阅 row 自身字段读, 不再回查 state.providers。
//!
//! Envelope 解析按 `auth_type` 分发:
//! - `ApiKey` / `ChatgptOauth` / `KiroOauth` → OpenAI 风格 `{data:[{id,model,display_name}]}`
//! - `GeminiApiKey` / `GeminiInteractionsApiKey` → Gemini 风格 `{models:[{name:"models/gemini-...", displayName, supportedGenerationMethods}]}`
//!
//! 解析不再硬过滤 (旧版本只留对话模型): Cloudflare 的 task.name、Gemini 的
//! supportedGenerationMethods 全部保留到 ModelInfo.native_capabilities, 供前端展示给用户选分类。

use std::collections::BTreeMap;

use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};

use crate::error::AppError;
use crate::provider::model::{join_base_path, AuthType, ModelsEnvelopeKind};
use crate::subscription::{
    model::{ModelCache, ModelInfo, SubscriptionRow},
    store,
};

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("http {0}")]
    Http(u16),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("app error: {0}")]
    App(#[from] AppError),
}

impl From<serde_json::Error> for DiscoveryError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidResponse(e.to_string())
    }
}

/// 一次发现成功的结果. `url` 是实际打通的完整地址 —— 自定义订阅靠候选探测得到,
/// 调用方把它写回 snapshot 的 `model_discovery.url`, 之后刷新就不必再猜。
#[derive(Debug, Clone)]
pub struct Discovered {
    pub models: Vec<ModelInfo>,
    pub url: String,
}

/// 不依赖已落库订阅的探测目标: 新建自定义订阅的表单在保存前就要拉模型列表 (issue #44)。
#[derive(Debug, Clone)]
pub struct ProbeTarget {
    pub base_url: String,
    /// 协议默认的 models 路径, 如 `/v1/models` / `/v1beta/models`。
    pub path: String,
    pub auth_type: AuthType,
    pub auth_header_name: String,
    pub auth_header_value: String,
    pub required_headers: BTreeMap<String, String>,
    pub envelope: Option<ModelsEnvelopeKind>,
}

/// 协议家族约定俗成的模型列表路径 (相对 base_url)。
pub fn default_models_path(auth_type: AuthType) -> &'static str {
    match auth_type {
        AuthType::GeminiApiKey | AuthType::GeminiInteractionsApiKey => "/v1beta/models",
        _ => "/v1/models",
    }
}

impl ProbeTarget {
    fn from_row(row: &SubscriptionRow) -> Self {
        // 老的自定义 Anthropic 订阅 snapshot 里 path 是空串 (当年 derive(Default) 的产物,
        // 因为 enabled=false 从不发请求所以没暴露)。空 path 会把 base_url 本身当 models 地址去请求。
        let mut path = match row.model_discovery.path.trim() {
            "" => default_models_path(row.auth_type).to_string(),
            p => p.to_string(),
        };

        // 优化: 如果用户没有显式改过 path (即走的 default), 且 base_url 结尾自带 /v1 或 /v1beta,
        // 就把默认的 `/v1/models` 改成 `/models`, 避免 join_base_path 时产生双重前缀
        // (DSH 风格: baseUrl 自带版本号, 模型列表只用 `/models`).
        let user_edited_path = !row.model_discovery.path.trim().is_empty();
        if !user_edited_path {
            let trimmed = row.base_url.trim_end_matches('/');
            let ends_v1 = trimmed.ends_with("/v1");
            let ends_v1beta = trimmed.ends_with("/v1beta");
            if (ends_v1 || ends_v1beta) && path == "/v1/models" {
                path = "/models".to_string();
            }
        }

        let target = Self {
            base_url: row.base_url.clone(),
            path,
            auth_type: row.auth_type,
            auth_header_name: row.auth_header_name.clone(),
            auth_header_value: row.auth_header_value(),
            required_headers: row.resolved_required_headers(),
            envelope: row.model_discovery.envelope,
        };
        if row.is_user_defined {
            target.with_custom_defaults()
        } else {
            target
        }
    }

    /// 自定义订阅专用的补头. Anthropic 官方 /v1/models 缺 `anthropic-version` 直接 400,
    /// 而自定义 Anthropic 订阅的 required_headers 默认是空的。用户自己填过则不覆盖。
    /// 内置 provider 不走这里 —— 它们的头以 yaml 为准。
    pub fn with_custom_defaults(mut self) -> Self {
        if self.auth_type == AuthType::ApiKey
            && !self
                .required_headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("anthropic-version"))
        {
            self.required_headers
                .insert("anthropic-version".into(), "2023-06-01".into());
        }
        self
    }
}

/// 自定义订阅的 models 地址候选, 按尝试顺序排列, 第一个永远是 `base_url + path`。
///
/// 用户填的 base_url 是 messages 的前缀, 不一定也是 models 的前缀
/// (例: `https://relay.example.com/anthropic` 的模型列表常在 `https://relay.example.com/v1/models`)。
///
/// 保守策略: 最多两个候选。base_url 带路径时追加 `origin + path`, 不逐级剥路径段 ——
/// 每多一个候选, 用户点一次按钮就可能多等一次超时, 而 `/a/b` → `/a` 这种中间层命中率很低。
/// 顺带覆盖 base_url 以 `/v1` 结尾的情况 (首选拼成 `/v1/v1/models`, origin 候选正好是对的)。
pub fn candidate_model_urls(base_url: &str, path: &str) -> Vec<String> {
    let primary = join_base_path(base_url, path);
    let mut urls = vec![primary];

    // 解析失败 (validate_base_url 只查了 scheme 前缀) 就只留首选, 让请求自己报错。
    if let Ok(parsed) = reqwest::Url::parse(base_url) {
        if !matches!(parsed.path(), "" | "/") {
            // origin 的 ascii_serialization 不带尾斜杠, 且保留非默认端口
            let origin = parsed.origin().ascii_serialization();
            let fallback = join_base_path(&origin, path);
            if !urls.contains(&fallback) {
                urls.push(fallback);
            }
        }
    }
    urls
}

/// 换一个候选地址有没有意义.
/// 第一层 (用户配置的 path) 只认 404/405/InvalidResponse —— 鉴权失败直接抛出, 让用户看到真实原因.
/// 第二层 (多 path 候选) 额外把 401/403 也当作"可恢复": DashScope 不同端点用不同鉴权方式,
/// 自定义网关的 header 拿到原始模型列表端点上可能就是 401, 应该继续试下一个.
fn worth_trying_next(e: &DiscoveryError, layer: u8) -> bool {
    match e {
        DiscoveryError::Http(404) | DiscoveryError::Http(405) | DiscoveryError::InvalidResponse(_) => true,
        DiscoveryError::Http(401) | DiscoveryError::Http(403) => layer > 1, // 只在第二层尝试下一个
        _ => false,
    }
}

/// 第二层候选 path —— 借鉴 DSH: 用户配的 path 跑不通时, 试一些常见的替代路径.
/// 比如 DashScope 的 `/api/v1/models` 就不是 OpenAI 兼容的 `/v1/models`.
/// 按常见度排序, 越靠前越可能命中.
///
/// 注意: 顺序里 `/models` 在 `/v1/models` **前面** —— 因为 DSH 的做法是 baseUrl
/// 自带版本号 (如 `.../compatible-mode/v1`), 模型列表只用 `/models`. 直接带 `/v1`
/// 会产生双重前缀 `.../v1/v1/models`. 详见 [`path_variants_for_base`].
fn common_model_paths(auth_type: AuthType) -> Vec<&'static str> {
    match auth_type {
        AuthType::GeminiApiKey | AuthType::GeminiInteractionsApiKey => {
            vec!["/v1beta/models"]
        }
        _ => vec![
            "/models",                       // DSH 风格: baseUrl 自带 /v1, 只用 /models
            "/v1/models",                    // OpenAI 标准 (最常见)
            "/api/v1/models",                // DashScope 原始端点
            "/compatible-mode/v1/models",    // DashScope 兼容模式 (完整路径, 直接拼 origin 域名)
            "/api/v1/deployments/models",    // DashScope 部署列表
            "/v1beta/models",                // 部分 Anthropic 兼容端点
        ],
    }
}

/// 根据 base_url 是否以版本号结尾, 生成"智能" path 变体.
/// 例如 base = `https://dashscope.aliyuncs.com/compatible-mode/v1`:
///   - 标准 path `/v1/models` → 直接用会变 `.../v1/v1/models` (错)
///   - 变体: `/v1/models` (保持原样, 让 join_base_path 处理) + `/models` (去掉 v1, 变成 `.../v1/models`)
///
/// 但更简单的做法: 直接让 common_model_paths 返回完整 path 列表, 在这里根据
/// base 是否带 `/v1` 结尾, 额外 push 一个不带 v1 的变体.
fn path_variants_for_base<'a>(
    paths: &'a [&'static str],
    base_url: &str,
) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = paths.to_vec();
    let trimmed = base_url.trim_end_matches('/');
    let ends_with_v1 = trimmed.ends_with("/v1");
    let ends_with_v1beta = trimmed.ends_with("/v1beta");
    if ends_with_v1 {
        // base 以 /v1 结尾, 把含 `/v1/models` 的 path 替换成 `/models` 的版本
        // 但我们已经在 common_model_paths 里把 `/models` 放在前面了, 所以只要保证
        // `candidate_model_urls` 里也会生成 origin (不带 /v1 的域名) 版本就行.
        // 这里额外加一个: 如果 path 是 `/v1/models` 且 base 结尾是 `/v1`, 再额外推一个 `/models`
        for &p in paths {
            if p == "/v1/models" {
                out.push("/models");
            }
        }
    }
    if ends_with_v1beta {
        for &p in paths {
            if p == "/v1beta/models" {
                out.push("/models");
            }
        }
    }
    // 去重
    out.sort_unstable();
    out.dedup();
    out
}

/// 把一个 base_url 和一组 path 组合成去重的候选 URL 列表.
fn all_candidate_urls(base_url: &str, paths: &[&str]) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &path in paths {
        for url in candidate_model_urls(base_url, path) {
            if seen.insert(url.clone()) {
                urls.push(url);
            }
        }
    }
    urls
}

async fn fetch_url(
    client: &reqwest::Client,
    url: &str,
    target: &ProbeTarget,
) -> Result<Vec<ModelInfo>, DiscoveryError> {
    let mut req = client
        .get(url)
        .header(&target.auth_header_name, &target.auth_header_value);
    for (k, v) in target.required_headers.iter() {
        req = req.header(k, v);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(DiscoveryError::Http(status.as_u16()));
    }
    let text = resp.text().await?;

    parse_by_kind(target.envelope, target.auth_type, &text)
}

/// 两层探测 (借鉴 DSH):
///   第一层 — 只试用户配置的 path (默认 /v1/models), 多 base 候选
///   第二层 — 第一层全败后, 试常见替代 path × 多 base 候选
/// 全部失败时返回首选地址的错误 (它最可能是用户预期的那个).
pub async fn probe(
    client: &reqwest::Client,
    target: &ProbeTarget,
) -> Result<Discovered, DiscoveryError> {
    let mut first_err: Option<DiscoveryError> = None;

    // ===== 第一层: 原始 path (和原来一样) =====
    {
        let mut got_recoverable = false;
        for url in candidate_model_urls(&target.base_url, &target.path) {
            match fetch_url(client, &url, target).await {
                Ok(models) => return Ok(Discovered { models, url }),
                Err(e) if worth_trying_next(&e, 1) => {
                    warn!(%url, error = %e, "models 候选地址失败, 尝试下一个");
                    first_err.get_or_insert(e);
                    got_recoverable = true;
                }
                Err(e) => return Err(e),
            }
        }
        // 第一层没报致命错但全败 → 进第二层
        if !got_recoverable {
            return Err(first_err.unwrap_or_else(|| DiscoveryError::InvalidResponse("没有可尝试的 models 地址".into())));
        }
    }

    // ===== 第二层: DSH 风格多 path 候选 =====
    // 用户配的 path 已经在第一层试过了, 这里跳过它避免重复请求.
    let common_paths = common_model_paths(target.auth_type);
    // 根据 base_url 是否自带版本号生成智能变体 (避免 /v1/v1/models 双重前缀)
    let smart_paths = path_variants_for_base(&common_paths, &target.base_url);
    let unique_paths: Vec<&str> = smart_paths
        .into_iter()
        .filter(|p| *p != target.path.as_str())
        .collect();

    // 把用户配置的 path 挪到最后 (它已经在第一层失败过, 但可能有极端场景)
    let mut urls = all_candidate_urls(&target.base_url, &unique_paths);
    // 用户原始 path 的 fallback 候选也加回来 (已去重)
    urls.extend(candidate_model_urls(&target.base_url, &target.path));

    info!(%target.base_url, ?unique_paths, "第一层失败, 进入第二层多 path 探测");
    for url in urls {
        match fetch_url(client, &url, target).await {
            Ok(models) => {
                info!(%url, "第二层探测命中模型列表");
                return Ok(Discovered { models, url });
            }
            Err(e) if worth_trying_next(&e, 2) => {
                debug!(%url, error = %e, "第二层候选失败, 继续下一个");
            }
            Err(e) => return Err(e), // 第二层都试完了还是致命错 → 放弃
        }
    }

    Err(first_err.unwrap_or_else(|| DiscoveryError::InvalidResponse("没有可尝试的 models 地址".into())))
}

pub async fn fetch(
    client: &reqwest::Client,
    row: &SubscriptionRow,
) -> Result<Discovered, DiscoveryError> {
    let target = ProbeTarget::from_row(row);

    // models 接口与 messages 不同域时, 订阅 snapshot 里的 model_discovery.url 是完整 URL.
    if let Some(full) = row.model_discovery.url.as_deref() {
        let models = fetch_url(client, full, &target).await?;
        return Ok(Discovered {
            models,
            url: full.to_string(),
        });
    }

    // 借鉴 DSH: 任何订阅都先尝试 probe (多 URL 候选探测), 不再依赖 yaml 里的 enabled 配置.
    // DSH 不会预先假设某个 API 端点有没有 /models, 总是先去探测.
    if row.is_user_defined {
        // 自定义订阅直接 probe (候选 URL 含 base + path 和 origin + path)
        probe(client, &target).await
    } else {
        // 内置订阅: 先按配置的 path 打, 如果 404/InvalidResponse 自动回退 probe.
        let configured_url = join_base_path(&row.base_url, &row.model_discovery.path);
        match fetch_url(client, &configured_url, &target).await {
            Ok(models) => Ok(Discovered { models, url: configured_url }),
            Err(e) if worth_trying_next(&e, 1) => {
                warn!(
                    configured = %configured_url,
                    error = %e,
                    "配置的 models 地址失败, 回退 probe 多候选"
                );
                probe(client, &target).await
            }
            Err(e) => Err(e),
        }
    }
}

/// Envelope chosen by yaml `model_discovery.envelope`, else by auth type (historical rule).
fn parse_by_kind(kind: Option<ModelsEnvelopeKind>, auth_type: AuthType, text: &str) -> Result<Vec<ModelInfo>, DiscoveryError> {
    match kind {
        Some(ModelsEnvelopeKind::Cloudflare) => parse_cloudflare_envelope(text),
        Some(ModelsEnvelopeKind::Gemini) => parse_gemini_envelope(text),
        Some(ModelsEnvelopeKind::Openai) => parse_openai_envelope(text),
        None => match auth_type {
            // Gemini generateContent 与 Interactions 都用 /v1beta/models (Gemini envelope 格式)。
            AuthType::GeminiApiKey | AuthType::GeminiInteractionsApiKey => parse_gemini_envelope(text),
            _ => parse_openai_envelope(text),
        },
    }
}

/// Workers AI `/ai/models/search`: `{"result": [{name, task: {name}}]}`.
/// 不再硬过滤 — 全部保留, task.name 写入 native_capabilities 供前端分类.
/// 仅过滤 Clef (System One 决策模型) 和 LoRA 基类 (需要 adapter, 不能直接用).
fn parse_cloudflare_envelope(text: &str) -> Result<Vec<ModelInfo>, DiscoveryError> {
    let parsed: Value = serde_json::from_str(text)?;
    let arr = parsed
        .get("result")
        .and_then(|m| m.as_array())
        .ok_or_else(|| DiscoveryError::InvalidResponse("Cloudflare 响应缺少 result 数组".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for m in arr {
        let Some(name) = m.get("name").and_then(Value::as_str) else { continue };
        // 绝对不能直接用的: Clef 决策模型、LoRA 基类
        if name.starts_with("@cf/cloudflare/clef") || name.ends_with("-lora") {
            continue;
        }
        // 保留 task.name 到 native_capabilities
        let native_task = m
            .pointer("/task/name")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .into_iter()
            .collect::<Vec<_>>();
        let inferred = infer_capability(name, &native_task);
        out.push(ModelInfo {
            id: name.to_string(),
            display_name: None,
            native_capabilities: native_task,
            inferred_capability: inferred,
        });
    }
    if out.is_empty() {
        return Err(DiscoveryError::InvalidResponse(
            "Cloudflare 响应中没有可用模型".into(),
        ));
    }
    Ok(out)
}

fn parse_openai_envelope(text: &str) -> Result<Vec<ModelInfo>, DiscoveryError> {
    // 宽松解析 —— 兼容各种 OpenAI-compatible 网关的不同返回形状.
    // 借鉴 DSH: 不依赖严格结构体, 用 serde_json::Value 探测数组位置.
    let parsed: Value = serde_json::from_str(text)?;
    let arr: &[Value] = if let Some(a) = parsed.pointer("/data").and_then(Value::as_array) {
        a
    } else if let Some(a) = parsed.pointer("/items").and_then(Value::as_array) {
        a
    } else if parsed.is_array() {
        // 极少数网关直接返回顶层数组 [{id: "..."}]
        parsed.as_array().unwrap()
    } else {
        return Err(DiscoveryError::InvalidResponse(
            "未找到模型列表数组 (data/items 或顶层数组)".into(),
        ));
    };

    if arr.is_empty() {
        return Err(DiscoveryError::InvalidResponse("data 为空".into()));
    }

    let mut models = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("model").and_then(Value::as_str));
        let Some(id) = id else { continue };
        let display_name = item
            .get("display_name")
            .and_then(Value::as_str)
            .map(String::from);
        // 顺便看看上游有没有传能力标签 (部分网关会带 abilities/capabilities 字段)
        let native_capabilities: Vec<String> = item
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let inferred = infer_capability(id, &native_capabilities);
        models.push(ModelInfo {
            id: id.to_string(),
            display_name,
            native_capabilities,
            inferred_capability: inferred,
        });
    }
    if models.is_empty() {
        return Err(DiscoveryError::InvalidResponse("无法解析模型 id".into()));
    }
    Ok(models)
}

/// Gemini /v1beta/models 响应:
/// ```json
/// {"models": [{
///   "name": "models/gemini-2.5-flash",
///   "displayName": "Gemini 2.5 Flash",
///   "supportedGenerationMethods": ["generateContent", "countTokens"],
///   ...
/// }]}
/// ```
/// 不再硬过滤 — 全部保留, supportedGenerationMethods 写入 native_capabilities 供前端分类.
fn parse_gemini_envelope(text: &str) -> Result<Vec<ModelInfo>, DiscoveryError> {
    let parsed: Value = serde_json::from_str(text)?;
    let arr = parsed
        .get("models")
        .and_then(|m| m.as_array())
        .ok_or_else(|| DiscoveryError::InvalidResponse("Gemini envelope 缺少 models 数组".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        // Gemini name 形如 "models/gemini-2.5-flash" — 去前缀
        let id = name.strip_prefix("models/").unwrap_or(name).to_string();
        // 保留 supportedGenerationMethods
        let native_methods: Vec<String> = item
            .get("supportedGenerationMethods")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let display_name = item
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(String::from);
        let inferred = infer_capability(&id, &native_methods);
        out.push(ModelInfo {
            id,
            display_name,
            native_capabilities: native_methods,
            inferred_capability: inferred,
        });
    }
    if out.is_empty() {
        return Err(DiscoveryError::InvalidResponse(
            "Gemini envelope 中无可用模型".into(),
        ));
    }
    Ok(out)
}

/// 根据模型名关键词 + 上游原生能力标签自动推断属于哪个能力分类.
/// 规则按优先级排列, 第一个命中即返回. 全部不命中返回 None (由用户手动选).
///
/// 设计原则: 宁可多也不能错 — embedding/whisper 这类强特征必须优先命中,
/// 普通 LLM 名字太多可能猜错, 直接返回 None 让用户确认.
pub(crate) fn infer_capability(
    model_id: &str,
    native_capabilities: &[String],
) -> Option<super::super::virtual_model::model::CapabilityCategory> {
    use super::super::virtual_model::model::CapabilityCategory as C;
    let id_lower = model_id.to_ascii_lowercase();

    // 1. 原生标签优先: Cloudflare task.name / Gemini supportedGenerationMethods
    for cap in native_capabilities {
        match cap.as_str() {
            "Text Embeddings" | "embedContent" | "embeddings.generateEmbeddings" => {
                return Some(C::Embedding);
            }
            "Image Generation" => return Some(C::Vision),
            "Text-to-Speech" | "Speech-to-Text" | "audio" if id_lower.contains("speech")
                || id_lower.contains("whisper")
                || id_lower.contains("tts") =>
            {
                return Some(C::Speech);
            }
            _ => {}
        }
    }

    // 2. 模型名关键词 (从最明确到最模糊)
    if id_lower.contains("embed") || id_lower.contains("bge") || id_lower.contains("e5-") {
        return Some(C::Embedding);
    }
    if id_lower.contains("whisper")
        || id_lower.contains("tts")
        || id_lower.contains("speech")
        || id_lower.contains("text-to-speech")
    {
        return Some(C::Speech);
    }
    if id_lower.contains("dall")
        || id_lower.contains("imagen")
        || id_lower.contains("image-gen")
        || id_lower.contains("stable-diffusion")
    {
        return Some(C::Vision);
    }
    // 多模态: 名字里同时带 vision + 文字相关, 或带 multimodal
    if id_lower.contains("multimodal")
        || id_lower.contains("vision")
        || id_lower.contains("vl")
        || id_lower.contains("gemini") // Gemini 家族普遍多模态
        || id_lower.contains("gpt-4o")
        || id_lower.contains("claude-3.5") // Sonnet 4+ / Opus 4 视觉
        || id_lower.contains("claude-4")
        || id_lower.contains("qwen-vl")
    {
        return Some(C::Multimodal);
    }

    // 3. 默认 LLM 命中: 名字明显是对话模型 (claude/chatgpt/gpt/deepseek/mistral/llama/qwen-text)
    if id_lower.contains("claude")
        || id_lower.contains("deepseek")
        || id_lower.contains("mistral")
        || id_lower.contains("llama")
        || id_lower.contains("chat")
        || id_lower.contains("gpt-") // 但 gpt-4o 已在多模态分支命中
        || id_lower.contains("sonnet")
        || id_lower.contains("opus")
        || id_lower.contains("haiku")
        || id_lower.contains("fable")
        || id_lower.contains("nova")
        || id_lower.contains("gpt-oss")
    {
        return Some(C::Llm);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::model::ModelsEnvelopeKind;

    const CLOUDFLARE_MODELS: &str = r#"{"result":[
        {"name":"@cf/openai/gpt-oss-120b","task":{"name":"Text Generation"}},
        {"name":"@cf/cloudflare/clef","task":{"name":"Text Generation"}},
        {"name":"@cf/cloudflare/clef-flash","task":{"name":"Text Generation"}},
        {"name":"@cf/google/gemma-2b-it-lora","task":{"name":"Text Generation"}},
        {"name":"@cf/baai/bge-m3","task":{"name":"Text Embeddings"}},
        {"name":"@cf/zai-org/glm-5.3","task":{"name":"Text Generation"}}
    ],"success":true,"errors":[],"messages":[]}"#;

    #[test]
    fn cloudflare_envelope_keeps_all_models_except_clef_and_lora() {
        // 不过滤 task.name 了: Text Generation + Text Embeddings 都保留.
        // 只过滤 Clef (System One 专用, 包括 clef 和 clef-flash 都以 clef 开头) 和 LoRA 基类.
        let ids: Vec<String> = parse_cloudflare_envelope(CLOUDFLARE_MODELS).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![
            "@cf/openai/gpt-oss-120b".to_string(),
            "@cf/baai/bge-m3".to_string(),
            "@cf/zai-org/glm-5.3".to_string(),
        ]);
    }

    #[test]
    fn cloudflare_envelope_empty_is_invalid() {
        assert!(parse_cloudflare_envelope(r#"{"result":[],"success":true}"#).is_err());
        assert!(parse_cloudflare_envelope(r#"{"data":[]}"#).is_err());
    }

    #[test]
    fn envelope_kind_overrides_auth_type() {
        assert!(matches!(parse_by_kind(Some(ModelsEnvelopeKind::Cloudflare), AuthType::OpenaiChatCompletionsApiKey, CLOUDFLARE_MODELS), Ok(v) if v.len() == 3));
        assert!(parse_by_kind(None, AuthType::OpenaiChatCompletionsApiKey, CLOUDFLARE_MODELS).is_err(), "default stays OpenAI envelope");
    }

    #[test]
    fn gemini_envelope_keeps_all_models() {
        // 不再硬过滤 generateContent — embedding / imagen 都保留,
        // supportedGenerationMethods 写入 native_capabilities 供前端分类.
        let text = r#"{
          "models": [
            {"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash",
             "supportedGenerationMethods":["generateContent","countTokens"]},
            {"name":"models/embedding-001","displayName":"Embedding 001",
             "supportedGenerationMethods":["embedContent"]},
            {"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro",
             "supportedGenerationMethods":["generateContent"]}
          ]
        }"#;
        let out = parse_gemini_envelope(text).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, "gemini-2.5-flash");
        assert_eq!(out[0].display_name, Some("Gemini 2.5 Flash".into()));
        assert_eq!(out[0].native_capabilities, vec!["generateContent", "countTokens"]);
        assert_eq!(out[1].id, "embedding-001");
        assert_eq!(out[2].id, "gemini-2.5-pro");
    }

    #[test]
    fn gemini_envelope_strips_models_prefix() {
        let text = r#"{"models":[
          {"name":"models/gemini-2.5-flash","supportedGenerationMethods":["generateContent"]}
        ]}"#;
        let out = parse_gemini_envelope(text).unwrap();
        assert_eq!(out[0].id, "gemini-2.5-flash");
    }

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OPENAI_MODELS: &str = r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#;

    fn target(base_url: String, auth_type: AuthType) -> ProbeTarget {
        ProbeTarget {
            base_url,
            path: "/v1/models".into(),
            auth_type,
            auth_header_name: "authorization".into(),
            auth_header_value: "Bearer sk-test".into(),
            required_headers: BTreeMap::new(),
            envelope: None,
        }
    }

    #[tokio::test]
    async fn probe_returns_models_and_the_url_that_worked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_MODELS))
            .mount(&server)
            .await;

        let found = probe(
            &reqwest::Client::new(),
            &target(server.uri(), AuthType::OpenaiChatCompletionsApiKey),
        )
        .await
        .unwrap();
        assert_eq!(found.models.len(), 2);
        assert_eq!(found.url, format!("{}/v1/models", server.uri()));
    }

    /// 鉴权失败换地址也没用: 必须原样上抛 401, 不能被后续候选的 404 盖掉。
    #[tokio::test]
    async fn probe_surfaces_auth_failure_immediately() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/relay/v1/models"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        // 第二候选明明能通 —— 但 401 之后不该再去碰它 (expect(0) 在 server drop 时校验)
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_MODELS))
            .expect(0)
            .mount(&server)
            .await;

        let err = probe(
            &reqwest::Client::new(),
            &target(format!("{}/relay", server.uri()), AuthType::OpenaiResponsesApiKey),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::Http(401)), "{err}");
    }

    /// 中转站对未知路径返回前端页面 (200 + HTML): 归为 InvalidResponse, 且值得换下一个候选。
    #[tokio::test]
    async fn html_page_is_invalid_response_and_worth_retrying() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<!doctype html><html></html>"))
            .mount(&server)
            .await;

        let err = probe(
            &reqwest::Client::new(),
            &target(server.uri(), AuthType::OpenaiChatCompletionsApiKey),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidResponse(_)), "{err}");
        assert!(worth_trying_next(&err, 1));
        assert!(worth_trying_next(&DiscoveryError::Http(404), 1));
        assert!(!worth_trying_next(&DiscoveryError::Http(401), 1)); // 层 1 不认 401
        assert!(worth_trying_next(&DiscoveryError::Http(401), 2));  // 层 2 认 401 (DashScope 多端点鉴权不同)
        assert!(!worth_trying_next(&DiscoveryError::Http(500), 2));
    }

    #[tokio::test]
    async fn custom_anthropic_target_sends_anthropic_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_MODELS))
            .mount(&server)
            .await;

        let t = target(server.uri(), AuthType::ApiKey).with_custom_defaults();
        assert!(probe(&reqwest::Client::new(), &t).await.is_ok());
    }

    #[test]
    fn custom_defaults_keep_user_supplied_anthropic_version() {
        let mut t = target("https://example.com".into(), AuthType::ApiKey);
        t.required_headers
            .insert("Anthropic-Version".into(), "2099-01-01".into());
        let t = t.with_custom_defaults();
        assert_eq!(t.required_headers.len(), 1);

        // 非 Anthropic 协议不补
        let t = target("https://example.com".into(), AuthType::OpenaiResponsesApiKey)
            .with_custom_defaults();
        assert!(t.required_headers.is_empty());
    }

    #[test]
    fn first_candidate_is_always_base_plus_path() {
        let urls = candidate_model_urls("https://relay.example.com/anthropic/", "/v1/models");
        assert_eq!(urls[0], "https://relay.example.com/anthropic/v1/models");
    }

    /// 复刻真实日志: 老的自定义 Anthropic 订阅 snapshot 是 `enabled=false` + `path=""`,
    /// 修复前请求打到 `<base>/` 本身, 两个候选全 404。
    #[tokio::test]
    async fn legacy_custom_row_with_empty_path_still_discovers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/anthropic/v1/models"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_MODELS))
            .mount(&server)
            .await;

        let mut row = SubscriptionRow::test_fixture("custom", "custom");
        row.is_user_defined = true;
        row.base_url = format!("{}/anthropic", server.uri());
        row.auth_header_name = "authorization".into();
        row.model_discovery = crate::provider::model::ModelDiscovery {
            enabled: false,
            path: String::new(),
            url: None,
            cache_ttl_hours: 0,
            example_models: Vec::new(),
            envelope: None,
        };

        let found = fetch(&reqwest::Client::new(), &row).await.unwrap();
        assert_eq!(found.url, format!("{}/anthropic/v1/models", server.uri()));
    }

    /// `..ModelDiscovery::default()` 必须与 serde 默认值一致, 否则空 path 会再次写进 snapshot。
    #[test]
    fn model_discovery_default_matches_serde_default() {
        use crate::provider::model::ModelDiscovery;
        let from_serde: ModelDiscovery = serde_json::from_str("{}").unwrap();
        let from_default = ModelDiscovery::default();
        assert_eq!(from_default.path, "/v1/models");
        assert_eq!(from_default.path, from_serde.path);
        assert_eq!(from_default.enabled, from_serde.enabled);
        assert_eq!(from_default.cache_ttl_hours, from_serde.cache_ttl_hours);
    }

    #[test]
    fn root_base_url_has_no_extra_candidate() {
        for base in ["https://api.example.com", "https://api.example.com/"] {
            assert_eq!(
                candidate_model_urls(base, "/v1/models"),
                vec!["https://api.example.com/v1/models"],
                "{base}"
            );
        }
    }

    #[test]
    fn base_url_with_path_adds_origin_candidate_only() {
        // 多级路径也只加 origin 一个, 不逐级剥
        assert_eq!(
            candidate_model_urls("https://relay.example.com/api/anthropic", "/v1/models"),
            vec![
                "https://relay.example.com/api/anthropic/v1/models",
                "https://relay.example.com/v1/models",
            ]
        );
        // 非默认端口必须保留 (本地 Ollama / 自建网关)
        assert_eq!(
            candidate_model_urls("http://localhost:11434/v1", "/v1/models"),
            vec![
                "http://localhost:11434/v1/v1/models",
                "http://localhost:11434/v1/models",
            ]
        );
    }

    #[test]
    fn unparseable_base_url_keeps_primary_only() {
        assert_eq!(candidate_model_urls("http://", "/v1/models").len(), 1);
    }

    /// issue #44 的核心场景: base_url 带 `/anthropic` 这类 messages 专属前缀,
    /// 模型列表实际在根路径下。
    #[tokio::test]
    async fn probe_falls_back_when_base_url_has_messages_only_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_MODELS))
            .mount(&server)
            .await;
        // /anthropic/v1/models 未挂载 → wiremock 默认 404

        let t = target(format!("{}/anthropic", server.uri()), AuthType::ApiKey);
        let found = probe(&reqwest::Client::new(), &t).await.unwrap();
        assert_eq!(found.url, format!("{}/v1/models", server.uri()));
    }

    #[test]
    fn gemini_envelope_embedding_only_is_ok() {
        // 不过滤 generateContent 后, 只有 embedding 模型也能正常返回.
        let text = r#"{"models":[
          {"name":"models/embedding-001","supportedGenerationMethods":["embedContent"]}
        ]}"#;
        let out = parse_gemini_envelope(text).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "embedding-001");
        assert_eq!(out[0].inferred_capability, Some(crate::virtual_model::model::CapabilityCategory::Embedding));
    }
}

/// 返回 `(cache, 实际打通的 url)`. 调用方据此决定要不要把 url 写回订阅 snapshot。
pub async fn fetch_and_cache(
    pool: &SqlitePool,
    client: &reqwest::Client,
    row: &SubscriptionRow,
) -> Result<(ModelCache, String), DiscoveryError> {
    let found = fetch(client, row).await?;
    let mut models = found.models;
    // 统一按模型名字母序排, 所有下游 (前端下拉 / TUI wizard / cache) 自动受益.
    models.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    let cache = ModelCache {
        fetched_at: Utc::now(),
        models,
    };
    if let Err(e) = store::save_model_cache(pool, &row.id, &row.endpoint_id, &cache).await {
        warn!(?e, "model cache 持久化失败");
    } else {
        info!(subscription_id = %row.id, "model list cached");
    }
    Ok((cache, found.url))
}
