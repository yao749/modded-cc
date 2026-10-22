//! 配置导入导出: 订阅 + 虚拟模型绑定 → 单个 JSON 文件, 密钥段可选用口令加密。
//! 设计稿: docs/superpowers/specs/2026-09-30-config-export-import-design.md

pub mod crypto;
pub mod export;
pub mod format;
pub mod import;
