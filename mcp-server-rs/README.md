# TencentDB Agent Memory MCP Server (Rust)

TencentDB Agent Memory 的 MCP (Model Context Protocol) 适配器，用 Rust 编写，编译为单个二进制文件。

将 TDAI Hermes Gateway 的 HTTP API 包装成 MCP 协议，让 Claude Code 能通过标准 MCP stdio 通道调用记忆能力。

## 架构

```
Claude Code <-- MCP (stdio) --> tencentdb-memory-mcp.exe <-- HTTP --> TDAI Gateway (:8420)
```

- 左侧: Claude Code 通过 stdin/stdout 与本程序通信 (MCP 协议)
- 右侧: 本程序通过 HTTP 请求转发到 TDAI Hermes Gateway

## 构建

需要 Rust 工具链 (rustup.rs)。

```bash
cd mcp-server-rs
cargo build --release
```

产物在 `target/release/tencentdb-memory-mcp.exe`，约 2.8MB (已开启 strip + LTO + size 优化)。

## 配置

通过环境变量配置:

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `TDAI_GATEWAY_URL` | Gateway 地址 | `http://127.0.0.1:8420` |
| `TDAI_GATEWAY_API_KEY` | Gateway API 密钥 (Bearer token) | 空 (不鉴权) |
| `TDAI_SESSION_KEY` | 默认会话标识 | `claude-code` |

## Claude Code 接入

在 `~/.mcp.json` 中添加:

```json
{
  "mcpServers": {
    "tencentdb-memory": {
      "type": "stdio",
      "command": "<绝对路径>/tencentdb-memory-mcp.exe",
      "args": [],
      "env": {}
    }
  }
}
```

重启 Claude Code 后即可使用。

## 工具列表

共 7 个 MCP 工具:

| 工具 | 说明 | Gateway 路径 |
|------|------|-------------|
| `tdai_health` | 检查 Gateway 健康状态，返回版本、运行时间、存储可用性 | `GET /health` |
| `tdai_memory_search` | 搜索 L1 结构化记忆 (从历史对话中提取的原子事实)，支持 BM25 关键词 + 向量混合搜索 | `POST /search/memories` |
| `tdai_conversation_search` | 搜索 L0 原始对话记录 (用户/助手的完整消息) | `POST /search/conversations` |
| `tdai_recall` | 根据当前查询召回相关记忆，返回 L1 记忆 + L3 人格上下文，为 LLM 上下文注入优化 | `POST /recall` |
| `tdai_capture` | 捕获一轮对话到记忆系统，记录为 L0 数据并触发后台流水线 (L1 提取 -> L2 场景 -> L3 人格) | `POST /capture` |
| `tdai_session_end` | 标记会话结束，刷新缓冲区中的待处理流水线任务 | `POST /session/end` |
| `tdai_seed` | 批量导入历史对话数据，走完整 L0->L1 流水线，用于数据迁移或从日志引导记忆 | `POST /seed` |

## 记忆层级说明

- **L0**: 原始对话记录 (用户说了什么，助手回了什么)
- **L1**: 结构化记忆 (从对话中提取的原子事实，如偏好、指令、知识点)
- **L2**: 场景聚合 (将相关记忆归类到场景，如 "编码风格"、"项目配置")
- **L3**: 人格画像 (综合所有记忆形成的用户画像，用于上下文注入)

## 相比 TypeScript 版本

| 对比项 | TypeScript (旧) | Rust (新) |
|--------|----------------|-----------|
| 启动方式 | `npx tsx index.ts` | 单二进制直接运行 |
| 依赖 | Node.js + npm 包 | 无外部依赖 |
| 体积 | node_modules 约 50MB+ | 二进制约 2.8MB |
| 启动速度 | 约 1-2 秒 | 约 10ms |
| 内存占用 | 约 50-80MB | 约 5-10MB |

## 开发

```bash
# 调试构建 (更快编译，无优化)
cargo build

# 运行 (不接 MCP 客户端会报 ConnectionClosed，属正常行为)
cargo run

# 检查编译
cargo check
```

## 许可证

与主项目保持一致。
