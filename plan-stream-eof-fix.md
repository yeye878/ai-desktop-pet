# Agent 流式响应 EOF 错误修复计划

## Context

用户报告 agent 回复存在问题：思考到一半停止、输出不完整、报错 `流式读取失败: request or response body error: error reading a body from connection: unexpected EOF during chunk size line`。

根本原因：
1. 现有恢复条件过严（要求已有文本内容 + 无工具调用），thinking 阶段断开会直接报错
2. 没有自动重试机制，网络瞬断直接失败
3. HTTP 客户端缺少 `read_timeout`、`tcp_keepalive` 等配置

## 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `src-tauri/src/direct_api/agent.rs` | 核心：放宽恢复条件、增加自动重试 |
| `src-tauri/src/lib.rs` | HTTP 客户端配置优化 |
| `src/components/ChatBubble.vue` | 前端错误处理改进 |
| `src/components/Dashboard.vue` | 同步 ChatBubble 的错误处理改进 |

---

## Task 1: HTTP 客户端配置优化

**文件**: `src-tauri/src/lib.rs` 第 2274-2277 行

```rust
// 现有代码
let http_client = reqwest::Client::builder()
    .connect_timeout(std::time::Duration::from_secs(15))
    .build()
    .expect("Failed to create HTTP client");

// 修改为
let http_client = reqwest::Client::builder()
    .connect_timeout(std::time::Duration::from_secs(15))
    .tcp_keepalive(std::time::Duration::from_secs(60))
    .pool_idle_timeout(std::time::Duration::from_secs(90))
    .build()
    .expect("Failed to create HTTP client");
```

**说明**：
- `tcp_keepalive(60s)`: 每 60 秒发送 keepalive 探测，防止中间代理/NAT 回收空闲连接
- `pool_idle_timeout(90s)`: 连接池中空闲连接 90 秒后关闭，避免使用过期连接
- 不设置 `timeout()` 和 `read_timeout()`：流式响应需要长时间读取，由应用层的 25 秒超时控制

---

## Task 2: 扩展可恢复错误类型

**文件**: `src-tauri/src/direct_api/agent.rs` 第 687-690 行

```rust
// 现有代码
fn is_recoverable_stream_eof(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("unexpected eof") && message.contains("chunk")
}

// 修改为
fn is_recoverable_stream_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    (lower.contains("unexpected eof") && lower.contains("chunk"))
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("connection closed")
        || (lower.contains("unexpected eof") && lower.contains("body"))
}
```

**说明**：扩展匹配范围，覆盖更多传输层瞬断错误。这些错误在流式场景下与 `unexpected EOF during chunk size line` 等效。

---

## Task 3: 放宽恢复条件（分层恢复策略）

**文件**: `src-tauri/src/direct_api/agent.rs` 第 194-206 行

```rust
// 现有代码
let chunk_bytes = match chunk_res {
    Ok(bytes) => bytes,
    Err(e) => {
        let message = e.to_string();
        if is_recoverable_stream_eof(&message)
            && !turn_text.trim().is_empty()
            && accumulated_tool_calls.is_empty()
        {
            break;
        }
        send_error(&app_handle, format!("流式读取失败: {message}"), &full_thinking).await;
        return;
    }
};

// 修改为分层恢复策略
let chunk_bytes = match chunk_res {
    Ok(bytes) => bytes,
    Err(e) => {
        let message = e.to_string();
        if is_recoverable_stream_error(&message) {
            if !turn_text.trim().is_empty() {
                // 有文本内容 -> 完全恢复
                break;
            }
            if !full_thinking.trim().is_empty() {
                // 仅有 thinking -> 作为回复展示
                turn_text = full_thinking.clone();
                turn_text.push_str("\n（连接中断，已展示思考内容）");
                break;
            }
            if !accumulated_tool_calls.is_empty() {
                // 工具调用不完整 -> 丢弃并恢复
                accumulated_tool_calls.clear();
                turn_text = "（连接中断，工具调用未完成）".to_string();
                break;
            }
            // 完全无内容 -> 标记中断
            turn_text = "（流式连接中断，未收到内容）".to_string();
            break;
        }
        send_error(&app_handle, format!("流式读取失败: {message}"), &full_thinking).await;
        return;
    }
};
```

**关键改动**：
- 移除 `!turn_text.trim().is_empty()` 限制：thinking 阶段断开也能恢复
- 移除 `accumulated_tool_calls.is_empty()` 限制：工具调用中断时丢弃并恢复
- 各层级都设置 `turn_text` 以确保后续保存逻辑正常执行

---

## Task 4: 增加流式读取自动重试

**文件**: `src-tauri/src/direct_api/agent.rs` 第 151-264 行

将流式读取逻辑包裹在重试循环中：

```rust
"auto" | _ => {
    const MAX_STREAM_RETRIES: u32 = 3;
    let mut stream_success = false;

    for attempt in 0..MAX_STREAM_RETRIES {
        if attempt > 0 {
            let delay = Duration::from_secs(2u64.pow(attempt as u32));
            let _ = app_handle.emit(
                "ai-thinking",
                format!("\n[连接中断，{}秒后重试 ({}/{})]\n", delay.as_secs(), attempt, MAX_STREAM_RETRIES),
            );
            tokio::time::sleep(delay).await;
        }

        let res = match call_chat_completions_stream(
            &state.http_client, &config, &api_messages, tools.clone(),
        ).await {
            Ok(res) => res,
            Err(e) => {
                if attempt < MAX_STREAM_RETRIES - 1 { continue; }
                send_error(&app_handle, format!("API 请求失败: {e}"), &full_thinking).await;
                return;
            }
        };

        let mut stream = res.bytes_stream();
        let mut sse = SseParser::new();
        let mut raw_buffer = String::new();
        let mut done = false;
        let mut stream_eof = false;

        loop {
            // ... 现有的 chunk 读取和 SSE 解析逻辑 ...

            // 在 EOF 处理中，如果有内容则 break，无内容则标记 stream_eof 并 break
            Err(e) => {
                let message = e.to_string();
                if is_recoverable_stream_error(&message) {
                    if !turn_text.trim().is_empty() || !full_thinking.trim().is_empty() {
                        // 有内容 -> 恢复
                        break;
                    }
                    // 无内容 -> 标记 EOF，让外层重试
                    stream_eof = true;
                    break;
                }
                send_error(...);
                return;
            }
        }

        if done || !stream_eof {
            stream_success = true;
            break;
        }
        // stream_eof 且无内容 -> 继续重试
    }

    if !stream_success && turn_text.is_empty() {
        send_error(&app_handle, "流式连接多次中断，无法获取响应".to_string(), &full_thinking).await;
        return;
    }

    // ... 后续的 fallback 和工具处理逻辑 ...
}
```

---

## Task 5: 前端错误处理改进

### 5a. ChatBubble.vue

**文件**: `src/components/ChatBubble.vue` 第 201-210 行

```typescript
// 现有代码
unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    const text = event.payload.aborted ? "已中止" : `出错了: ${event.payload.message}`;
    // ...
});

// 修改为
unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    let text: string;
    if (event.payload.aborted) {
        text = "已中止";
    } else if (streamingAnswer.value || thinkingContent.value) {
        // 有部分内容时，显示警告而非错误
        text = `⚠️ ${event.payload.message}`;
    } else {
        text = `出错了: ${event.payload.message}`;
    }
    appendAssistantOnce(text, event.payload.thinking ?? (thinkingContent.value || undefined));
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    isThinkingCollapsed.value = true;
    pendingConfirm.value = null;
    // 有部分内容时保持 idle，无内容时才 confused
    pet.setState(event.payload.aborted ? "idle" : 
        ((streamingAnswer.value || thinkingContent.value) ? "idle" : "confused"));
});
```

### 5b. Dashboard.vue

同步 ChatBubble.vue 的修改。

---

## 验证方案

1. **编译验证**: `cd src-tauri && cargo build` 确保 Rust 代码无编译错误
2. **单元测试**: `cargo test` 运行现有测试 + 新增测试用例
3. **功能测试**:
   - 正常对话：确保正常流式响应不受影响
   - 模拟网络中断：在流式传输过程中断开网络，验证恢复行为
   - thinking 阶段中断：确保 thinking 内容能作为回复展示
4. **前端测试**: 验证错误提示显示正确，宠物状态切换正确

## 执行顺序

Task 1 → Task 2 → Task 3 → Task 4 → Task 5（按依赖顺序执行）
