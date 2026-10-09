/**
 * 桌宠 DSH 接入的本地假模型服务（仅用于离线验证，不参与产品运行）。
 *
 * 它实现 OpenAI 兼容的 /v1/chat/completions 流式接口，用来在没有凭据、不联网、
 * 不写用户 DSH 主目录的前提下，端到端验证 pet-runner 插件与桌宠侧的解析：
 *
 *   * 每个请求都先流式返回 reasoning_content，用于验证推理增量通道；
 *   * 回复正文里回显从用户消息里抓到的 PET-MARKER-xxx 与 seen1=yes/no；
 *     seen1 表示请求历史里是否带着第一轮的标记，用来证明 --resume 真的续接了会话；
 *   * 用户消息里出现 USE_TOOL 时，先返回一次 pwsh 工具调用，等下一轮带着工具结果
 *     再给最终回答，用来验证工具事件通道。
 *
 * 用法：node scripts/dsh-mock-provider.cjs --port-file <path>
 * 端口写在 --port-file 指定的文件里（0 表示由系统分配）。
 */
const http = require('node:http')
const fs = require('node:fs')

const argv = process.argv.slice(2)
const portFileIndex = argv.indexOf('--port-file')
const portFile = portFileIndex >= 0 ? argv[portFileIndex + 1] : undefined

function textOf(content) {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    return content.map((part) => (part && typeof part.text === 'string' ? part.text : '')).join('')
  }
  return ''
}

function plan(messages) {
  const history = messages.map((message) => textOf(message.content)).join('\n')
  // 提示词可能不是最后一条 user 消息：DSH 会在任务之后注入上下文消息，
  // 所以标记要在全部 user 消息里找。
  const prompt = messages
    .filter((message) => message.role === 'user')
    .map((message) => textOf(message.content))
    .join('\n')
  const marker = (prompt.match(/PET-MARKER-[0-9a-zA-Z-]+/) ?? ['PET-MARKER-none'])[0]
  const hasToolResult = messages.some((message) => message.role === 'tool')
  const toolResult = [...messages].reverse().find((message) => message.role === 'tool')
  const wantsTool = prompt.includes('USE_TOOL')

  if (wantsTool && !hasToolResult) {
    return {
      reasoning: '先执行命令确认结果。',
      text: '',
      // pwsh 工具要求 command 与 description 同时存在，缺一个会在参数校验阶段就失败。
      // 同时请求升权（sandbox_permissions + justification）：这正是 DSH 会走
      // approval/request 向用户征求同意的路径，用来验证授权往返。
      toolCall: {
        name: 'pwsh',
        arguments: JSON.stringify({
          command: 'echo pet-tool-check',
          description: 'Echo the pet tool check marker',
          sandbox_permissions: 'danger-full-access',
          justification: '验证桌宠的授权确认通道'
        })
      }
    }
  }
  // 只有在请求历史里看到过我自己上一轮的回复，才算真的续接了会话：
  // 本轮提示词里的标记不算，否则第一轮就会误判成续接成功。
  const seenFirstTurn = messages.some(
    (message) => message.role === 'assistant' && textOf(message.content).includes('PET-MARKER-FIRST-TURN-')
  )
  const echoed = hasToolResult ? ' 工具返回：' + textOf(toolResult?.content).replace(/\s+/g, ' ').trim() : ''
  return {
    reasoning: '先确认用户要求。',
    text: '收到。' + marker + ' seen1=' + (seenFirstTurn ? 'yes' : 'no') + echoed,
    toolCall: undefined
  }
}

function chunk(payload) {
  return 'data: ' + JSON.stringify(payload) + '\n\n'
}

const server = http.createServer((request, response) => {
  if (!request.url || !request.url.endsWith('/chat/completions')) {
    response.writeHead(404, { 'content-type': 'application/json' }).end('{"error":"not found"}')
    return
  }
  let body = ''
  request.on('data', (part) => { body += part })
  request.on('end', () => {
    let parsed
    try {
      parsed = JSON.parse(body)
    } catch {
      response.writeHead(400, { 'content-type': 'application/json' }).end('{"error":"bad json"}')
      return
    }
    const envelope = (delta, finish) => ({
      id: 'chatcmpl-pet-mock',
      object: 'chat.completion.chunk',
      created: Math.floor(Date.now() / 1000),
      model: parsed.model ?? 'pet-mock-model',
      choices: [{ index: 0, delta, finish_reason: finish ?? null }]
    })
    const outcome = plan(Array.isArray(parsed.messages) ? parsed.messages : [])
    response.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive'
    })
    response.write(chunk(envelope({ role: 'assistant', reasoning_content: outcome.reasoning })))
    if (outcome.toolCall) {
      response.write(chunk(envelope({
        tool_calls: [{
          index: 0,
          id: 'call_pet_1',
          type: 'function',
          function: { name: outcome.toolCall.name, arguments: outcome.toolCall.arguments }
        }]
      })))
      response.write(chunk(envelope({}, 'tool_calls')))
    } else {
      // 分片发送，确保桌宠侧看到的是真正的增量而不是一次性文本。
      for (const piece of outcome.text.match(/.{1,8}/gs) ?? []) {
        response.write(chunk(envelope({ content: piece })))
      }
      response.write(chunk(envelope({}, 'stop')))
    }
    response.write(chunk({ id: 'chatcmpl-pet-mock', object: 'chat.completion.chunk', created: Math.floor(Date.now() / 1000), model: parsed.model ?? 'pet-mock-model', choices: [], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } }))
    response.write('data: [DONE]\n\n')
    response.end()
  })
})

server.listen(0, '127.0.0.1', () => {
  const port = server.address().port
  if (portFile) fs.writeFileSync(portFile, String(port))
  process.stdout.write('dsh-mock-provider listening on ' + port + '\n')
})
