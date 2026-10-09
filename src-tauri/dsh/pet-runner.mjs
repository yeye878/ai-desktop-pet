/**
 * dsh-pet runner —— 桌宠专用的一次性 runner 插件（由 ai-desktop-pet 生成，请勿手改）。
 *
 * 它替换随包发布的 headless-runner：官方 headless 应用只把最后一条 assistant 文本
 * 写到 stdout，桌宠拿不到推理过程和工具调用，也无法续接会话。这个 runner 走同一套
 * 核心 API（agentDefaultModel / agents / sessions），但：
 *
 *   1. 任务文本从 --prompt-file 读取，不再受命令行长度限制；
 *   2. 订阅 session/event，把推理增量、正文增量、工具调用与结果按 JSONL 写到 stdout；
 *   3. 支持 --resume <session-id>，配合 sessions.flush 让下一轮真正续接同一会话。
 *
 * stdout 协议（每行一个 JSON 对象，未经压缩）：
 *   {"type":"log","text":...}                       诊断信息，桌宠忽略
 *   {"type":"session","session_id":...,"resumed":bool}
 *   {"type":"thinking","text":...}                  推理增量
 *   {"type":"text","text":...}                      正文增量
 *   {"type":"message","text":...}                   一步结束后的完整 assistant 文本
 *   {"type":"tool","phase":"start"|"call"|"result",...}
 *   {"type":"done","reason":"completed"|"error"|...}
 *   {"type":"error","code":...,"message":...}
 *
 * 授权往返（比上面单向的流多一条回程）：需要在桌宠里确认的操作会先写一行
 *   {"type":"approval-request","id":...,"tool":...,"reason":...,"call_id":...}
 * 然后等 stdin 上的一行回答
 *   {"type":"approval-decision","id":...,"decision":"allow"|"deny"|"allow-session"|"cancel"}
 * 收到前一个应答者不会解除等待：桌宠不在（stdin 关闭）或超时都按拒绝处理，与
 * DSH 自己的「没有应答者就 fail closed」保持一致。
 */

import { randomUUID } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { createInterface } from 'node:readline'
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import { SessionId } from '@deepseek-ai/dsh-session'
import { installModelSelection } from '@deepseek-ai/dsh-agent'

export const name = 'pet-runner'
export const inject = ['agentDefaultModel', 'agents', 'sessions', 'cmdlineArgs', 'appExit']

/** 单个字段写进协议前的上限，避免一次工具输出把整行撑爆。 */
const FIELD_LIMIT = 4000

/** 桌宠不回话时的兜底上限：到点按拒绝处理，绝不让 DSH 无限等待。 */
const APPROVAL_TIMEOUT_MS = 10 * 60 * 1000

/** 等待桌宠决定的授权请求（key 是协议里的请求 id）。 */
const pendingApprovals = new Map()

/** 桌宠选择「本次会话都允许」的工具名；seam 只支持一次性授权，所以记在这里。 */
const sessionAllows = new Set()

const USAGE = [
  'dsh --profile dsh-pet --prompt-file <path> [--resume <session-id>]',
  '',
  '桌宠专用 runner：读取提示词文件，流式输出推理/正文/工具事件，转发授权请求，可选续接已有会话。',
  '',
  'Options:',
  '  --prompt-file <path>    UTF-8 提示词文件（必需）',
  '  --resume <session-id>   续接该会话，失败时自动新建',
  '  -h, --help              显示本帮助',
  ''
].join('\n')

function emit(record) {
  process.stdout.write(JSON.stringify(record) + '\n')
}

function clip(value, limit) {
  const text = typeof value === 'string' ? value : value === undefined || value === null ? '' : String(value)
  const bound = typeof limit === 'number' ? limit : FIELD_LIMIT
  if (text.length <= bound) return text
  return text.slice(0, bound) + '\n...[已截断 ' + (text.length - bound) + ' 字符]'
}

function parseArgs(argv) {
  const options = { promptFile: undefined, resume: undefined, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--prompt-file') options.promptFile = argv[index += 1]
    else if (arg === '--resume') options.resume = argv[index += 1]
    else if (arg === '-h' || arg === '--help') options.help = true
  }
  return options
}

function blocksToText(blocks) {
  if (!Array.isArray(blocks)) return ''
  return blocks
    .filter((block) => block && typeof block === 'object' && block.type === 'text' && typeof block.text === 'string')
    .map((block) => block.text)
    .join('\n')
}

/**
 * 读取桌宠写在 stdin 上的决定。stdin 关闭表示桌宠已经不在了：把仍在等待的
 * 请求全部按拒绝解开，避免工具调用挂死。
 */
function readApprovalDecisions() {
  const reader = createInterface({ input: process.stdin })
  reader.on('line', (line) => {
    let record
    try {
      record = JSON.parse(line)
    } catch {
      return
    }
    if (!record || record.type !== 'approval-decision') return
    settleApproval(String(record.id ?? ''), String(record.decision ?? 'deny'))
  })
  reader.on('close', () => {
    for (const id of [...pendingApprovals.keys()]) settleApproval(id, 'deny')
  })
  return reader
}

function settleApproval(id, decision) {
  const entry = pendingApprovals.get(id)
  if (entry === undefined) return
  pendingApprovals.delete(id)
  clearTimeout(entry.timer)
  entry.resolve(decision)
}

/** 把一次授权请求交给桌宠，并等它的决定。 */
function askPet(request, sessionId) {
  return new Promise((resolve) => {
    const id = 'approval-' + randomUUID()
    const finish = (decision) => settleApproval(id, decision)
    const timer = setTimeout(() => {
      emit({ type: 'log', text: '授权请求 ' + id + ' 超时未获答复，按拒绝处理' })
      finish('deny')
    }, APPROVAL_TIMEOUT_MS)
    pendingApprovals.set(id, { resolve, timer })
    // 轮次被中止时不必再等：服务自己会把结果记成 cancelled。
    request?.signal?.addEventListener?.('abort', () => finish('deny'), { once: true })
    emit({
      type: 'approval-request',
      id,
      session_id: sessionId,
      tool: request?.toolName === undefined ? '' : String(request.toolName),
      reason: request?.reason === undefined ? undefined : String(request.reason),
      call_id: request?.callId === undefined ? undefined : String(request.callId)
    })
  })
}

async function run(ctx) {
  const exit = ctx.get('appExit')
  const argv = ctx.get('cmdlineArgs').get()
  const options = parseArgs(argv)
  if (options.help) {
    process.stdout.write(USAGE)
    exit(0)
    return
  }
  if (!options.promptFile) throw new Error('缺少 --prompt-file 参数')

  const task = await readFile(options.promptFile, 'utf8')
  if (task.trim() === '') throw new Error('提示词文件为空')

  // 等配置树结算，保证工具与沙箱等能力都已挂载后再创建 agent。
  await ctx.get('loader')?.await()

  const defaultModel = ctx.get('agentDefaultModel')
  const selection = defaultModel.currentSelection()
  const agentOptions = { provider: selection.provider, model: selection.model }
  const setup = (agentCtx) => {
    installModelSelection(agentCtx, { current: selection, assembled: undefined })
  }

  let sessionId = options.resume
  let resumed = false
  let handle
  if (sessionId) {
    try {
      handle = await ctx.agents.resume({ resumeSessionId: sessionId, agentOptions, setup })
      resumed = true
    } catch (error) {
      emit({ type: 'log', text: '恢复会话 ' + sessionId + ' 失败，改为新会话：' + (error?.message ?? String(error)) })
      handle = undefined
    }
  }
  if (!handle) {
    sessionId = 'session-' + randomUUID()
    handle = await ctx.agents.create({
      sessionId: SessionId(sessionId),
      meta: { cwd: process.cwd() },
      agentOptions,
      setup
    })
  }

  const agent = handle.agent
  // 先接上桌宠的决定通道，再开始这一轮：授权请求随时可能到来。
  const approvalReader = readApprovalDecisions()
  // 桌宠的 stdin 是一条常开的管道。不 unref 的话，它会在这个句柄上保持事件循环
  // 存活：树拆完之后进程不退出，桌宠那边就会一直等 EOF。
  process.stdin.unref?.()
  /** 收尾：先断开决定通道，再走启动器的退出流程。 */
  const finish = (code) => {
    try {
      approvalReader.close()
    } catch {}
    try {
      process.stdin.pause()
    } catch {}
    exit(code)
  }
  emit({ type: 'session', session_id: sessionId, resumed })

  let reason
  const streamEvent = (event) => {
    const data = event.data
    if (!data || typeof data !== 'object') return
    switch (event.type) {
      case 'assistant/chunk': {
        const chunk = data.chunk
        if (!chunk || typeof chunk !== 'object') return
        if (chunk.type === 'text-delta' && chunk.text) emit({ type: 'text', text: chunk.text })
        else if (chunk.type === 'reasoning-delta' && chunk.text) emit({ type: 'thinking', text: chunk.text })
        else if (chunk.type === 'tool-call-delta' && chunk.id) {
          emit({ type: 'tool', phase: 'start', id: String(chunk.id), name: chunk.name ? String(chunk.name) : undefined })
        }
        return
      }
      case 'tool/call':
        emit({
          type: 'tool',
          phase: 'call',
          id: String(data.callId ?? ''),
          name: String(data.name ?? ''),
          arguments: clip(data.arguments)
        })
        return
      case 'tool/result': {
        const block = Array.isArray(data.message?.content) ? data.message.content[0] : undefined
        emit({
          type: 'tool',
          phase: 'result',
          id: String(block?.toolCallId ?? ''),
          is_error: data.error !== undefined || block?.isError === true,
          content: clip(blocksToText(block?.content))
        })
        return
      }
      case 'assistant/message': {
        const text = blocksToText(data.message?.content)
        if (text !== '') emit({ type: 'message', text })
        return
      }
      case 'turn/end':
        reason = data.reason
        return
      default:
        return
    }
  }

  // 授权应答者：DSH 的工具流水线在需要确认时会走 approval/request waterfall，
  // 这里把请求转发给桌宠，再把桌宠的决定翻译回 seam 的结果词表。
  ctx.on('approval/request', async (request, next) => {
    if (request?.agent?.session?.id !== sessionId) return next()
    const tool = request?.toolName === undefined ? '' : String(request.toolName)
    if (tool !== '' && sessionAllows.has(tool)) return 'allowed-once'
    const decision = await askPet(request, sessionId)
    if (decision === 'allow') return 'allowed-once'
    if (decision === 'allow-session') {
      if (tool !== '') sessionAllows.add(tool)
      return 'allowed-once'
    }
    if (decision === 'cancel') return 'cancelled'
    return 'rejected'
  })

  const approvalPolicy = ctx.get('approval')?.effectivePolicy?.(agent.session)
  if (approvalPolicy !== undefined && approvalPolicy !== 'ask') {
    emit({
      type: 'log',
      text: '本机 dsh 的审批策略是 "' + approvalPolicy + '"：需要授权的操作会直接被拒绝，桌宠不会收到确认请求。'
    })
  }

  await agent.whenIdle()
  const firstSeq = agent.session.seq
  // 只关心本轮触发的 turn，replay/resume 带进来的历史事件直接跳过。
  ctx.on('session/event', (session, event) => {
    if (session.id !== sessionId) return
    if (typeof event.seq === 'number' && event.seq < firstSeq) return
    try {
      streamEvent(event)
    } catch (error) {
      emit({ type: 'log', text: '忽略无法处理的事件 ' + event.type + '：' + (error?.message ?? String(error)) })
    }
  })

  agent.followup(createUserMessage({ content: [{ type: 'text', text: task }], source: { kind: 'user' } }))
  await agent.whenIdle()
  // 落盘后再退出，下一轮 --resume 才拿得到这一轮的记录。
  await ctx.get('sessions')?.flush(agent.session)

  const kind = reason?.kind ?? 'unknown'
  emit({ type: 'done', reason: kind })
  if (kind !== 'completed') {
    const failure = reason?.error
    emit({
      type: 'error',
      code: String(failure?.code ?? kind),
      message: String(failure?.message ?? (kind === 'aborted' ? '本轮被中止' : '本轮未正常完成'))
    })
  }
  finish(kind === 'completed' ? 0 : 1)
}

export function apply(ctx) {
  const exit = ctx.get('appExit')
  if (exit === undefined) throw new Error('pet-runner: the launcher must provide ctx.appExit before the tree mounts')
  run(ctx).catch((error) => {
    emit({ type: 'error', code: 'PET_RUNNER_FAILED', message: String(error?.message ?? error) })
    exit(1)
  })
}
