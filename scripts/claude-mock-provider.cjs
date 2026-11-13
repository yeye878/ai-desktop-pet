/**
 * 桌宠 Claude Code 授权桥的本地假模型服务（仅用于离线验证）。
 *
 * 实现 Anthropic Messages 流式接口的最小可用子集：第一次请求返回一个 Bash 工具调用，
 * 拿到工具结果后再给最终回答。配合 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN 就能
 * 在没有真实账号、不联网的前提下验证 claude -p 是否会把权限提示交给桌宠的 MCP 工具。
 *
 * 用法：node scripts/claude-mock-provider.cjs --port-file <path>
 */
const http = require('node:http')
const fs = require('node:fs')

const argv = process.argv.slice(2)
const portFileIndex = argv.indexOf('--port-file')
const portFile = portFileIndex >= 0 ? argv[portFileIndex + 1] : undefined

function textOf(content) {
  if (typeof content === 'string') return content
  if (!Array.isArray(content)) return ''
  return content
    .map((block) => {
      if (!block || typeof block !== 'object') return ''
      if (typeof block.text === 'string') return block.text
      if (typeof block.content === 'string') return block.content
      if (Array.isArray(block.content)) return textOf(block.content)
      return ''
    })
    .join('\n')
}

/** 从请求里挑出这个平台的 shell 工具名（Windows 上是 PowerShell，其他地方是 Bash）。 */
function pickShellTool(tools) {
  const names = (Array.isArray(tools) ? tools : [])
    .map((tool) => (tool && typeof tool.name === 'string' ? tool.name : ''))
    .filter(Boolean)
  for (const candidate of ['Bash', 'PowerShell', 'shell']) {
    if (names.includes(candidate)) return candidate
  }
  return 'Bash'
}

function sawToolResult(messages) {
  return messages.some(
    (message) =>
      Array.isArray(message.content) &&
      message.content.some((block) => block && block.type === 'tool_result')
  )
}

function sse(response, event, payload) {
  response.write('event: ' + event + '\n')
  response.write('data: ' + JSON.stringify(payload) + '\n\n')
}

const server = http.createServer((request, response) => {
  if (request.method !== 'POST') {
    response.writeHead(405).end()
    return
  }
  let body = ''
  request.on('data', (part) => { body += part })
  request.on('end', () => {
    let parsed = {}
    try {
      parsed = JSON.parse(body)
    } catch {}
    const path = (request.url || '').split('?')[0]

    if (path.endsWith('/count_tokens')) {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ input_tokens: 10 }))
      return
    }
    if (!path.endsWith('/messages')) {
      response.writeHead(404, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ type: 'error', error: { type: 'not_found_error', message: path } }))
      return
    }

    const messages = Array.isArray(parsed.messages) ? parsed.messages : []
    const hasToolResult = sawToolResult(messages)
    const userText = messages
      .filter((message) => message.role === 'user')
      .map((message) => textOf(message.content))
      .join('\n')
    const marker = (userText.match(/PET-MARKER-[0-9a-zA-Z-]+/) ?? ['PET-MARKER-none'])[0]

    response.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive'
    })
    sse(response, 'message_start', {
      type: 'message_start',
      message: {
        id: 'msg_pet_mock',
        type: 'message',
        role: 'assistant',
        model: parsed.model ?? 'pet-mock',
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 10, output_tokens: 1 }
      }
    })

    const memoryRequested = userText.includes('USE_MEMORY')
    const memoryCalled = messages.some(message => Array.isArray(message.content) && message.content.some(block => block.type === 'tool_use' && block.name?.endsWith('__save_memory')))
    const useMemory = hasToolResult && memoryRequested && !memoryCalled
    const memoryTool = (parsed.tools || []).find(tool => tool.name?.endsWith('__save_memory'))?.name
    if (!hasToolResult || useMemory) {
      // 请求一次 Bash 调用：这正是需要授权、会走 --permission-prompt-tool 的场景。
      sse(response, 'content_block_start', {
        type: 'content_block_start',
        index: 0,
        content_block: {
          type: 'tool_use',
          id: useMemory ? 'toolu_pet_memory' : 'toolu_pet_1',
          name: useMemory ? (memoryTool || 'missing_memory_tool') : pickShellTool(parsed.tools),
          input: {}
        }
      })
      sse(response, 'content_block_delta', {
        type: 'content_block_delta',
        index: 0,
        delta: {
          type: 'input_json_delta',
          partial_json: JSON.stringify(useMemory ? { category: 'preference', key: 'claude-language', value: '中文' } : { command: 'echo pet-claude-check', description: 'Echo the pet check marker' })
        }
      })
      sse(response, 'content_block_stop', { type: 'content_block_stop', index: 0 })
      sse(response, 'message_delta', {
        type: 'message_delta',
        delta: { stop_reason: 'tool_use', stop_sequence: null },
        usage: { output_tokens: 5 }
      })
    } else {
      const toolText = messages
        .flatMap((message) => (Array.isArray(message.content) ? message.content : []))
        .filter((block) => block && block.type === 'tool_result')
        .map((block) => textOf(block.content))
        .join(' ')
        .replace(/\s+/g, ' ')
        .trim()
      const text = '收到。' + marker + ' 工具返回：' + toolText
      sse(response, 'content_block_start', {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'text', text: '' }
      })
      for (const piece of text.match(/.{1,12}/gs) ?? []) {
        sse(response, 'content_block_delta', {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: piece }
        })
      }
      sse(response, 'content_block_stop', { type: 'content_block_stop', index: 0 })
      sse(response, 'message_delta', {
        type: 'message_delta',
        delta: { stop_reason: 'end_turn', stop_sequence: null },
        usage: { output_tokens: 20 }
      })
    }
    sse(response, 'message_stop', { type: 'message_stop' })
    response.end()
  })
})

server.listen(0, '127.0.0.1', () => {
  const port = server.address().port
  if (portFile) fs.writeFileSync(portFile, String(port))
  process.stdout.write('claude-mock-provider listening on ' + port + '\n')
})
