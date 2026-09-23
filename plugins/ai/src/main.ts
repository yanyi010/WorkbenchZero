/**
 * AI plugin (spec §46–51): Quick Ask + chat over any OpenAI-compatible
 * endpoint. Streaming via the kernel network capability; the API key
 * lives in the secrets vault, never in settings or the DOM. Tool calls
 * are routed to tools registered by other plugins through the kernel
 * (`ai.callTool`), with high-risk tools requiring explicit confirmation
 * (spec §54). Before sending, the privacy line states exactly which
 * endpoint the text goes to (spec §51).
 */
import { definePlugin, h, render } from '@eigendesk/plugin-sdk';
import { renderMarkdown } from '@eigendesk/plugin-sdk/markdown';
import type { PluginContext } from '@eigendesk/plugin-sdk';

interface ChatMsg {
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string | null;
  tool_calls?: { id: string; type: 'function'; function: { name: string; arguments: string } }[];
  tool_call_id?: string;
}

interface ToolSpec {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  highRisk: boolean;
}

interface ToolCall {
  id: string;
  name: string;
  arguments: string;
}

interface UiMessage {
  role: 'user' | 'assistant' | 'error' | 'tool';
  content: string;
  toolCalls?: ToolCall[];
}

let ctx: PluginContext;
let conversation: ChatMsg[] = [];
let ui: UiMessage[] = [];
let streaming = false;
let configured = false;
let endpoint = '';
let modelId = '';

// ---------------------------------------------------------------------------
// OpenAI-compatible transport (SSE over network.fetchStream)
// ---------------------------------------------------------------------------

interface StreamEvents {
  onDelta: (text: string) => void;
  onToolCall: (call: ToolCall) => void;
  onDone: (finishReason: string) => void;
  onError: (message: string) => void;
}

async function apiKey(): Promise<string | undefined> {
  const name = (await ctx.settings.get<string>('eigendesk.ai.apiKeySecret')) ?? 'eigendesk.ai.apiKey';
  try {
    const value = await ctx.secrets.get(name);
    return value ?? undefined;
  } catch {
    return undefined; // vault locked / not granted
  }
}

async function chatRequest(messages: ChatMsg[], tools: ToolSpec[], ev: StreamEvents): Promise<void> {
  const base = (await ctx.settings.get<string>('eigendesk.ai.baseUrl')) ?? '';
  modelId = (await ctx.settings.get<string>('eigendesk.ai.model')) ?? '';
  const maxTokens = (await ctx.settings.get<number>('eigendesk.ai.maxTokens')) ?? 2048;
  if (!base || !modelId) throw new Error('Quick Ask is not configured — set Base URL and Model in Settings');
  const key = await apiKey();

  const body: Record<string, unknown> = {
    model: modelId,
    messages,
    stream: true,
    max_tokens: maxTokens,
  };
  if (tools.length > 0) {
    body.tools = tools.map((t) => ({
      type: 'function',
      function: { name: t.name, description: t.description, parameters: t.parameters },
    }));
    body.tool_choice = 'auto';
  }

  const url = `${base.replace(/\/+$/, '')}/chat/completions`;
  sseBuffer = '';
  await ctx.network.fetchStream(
    url,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Accept: 'text/event-stream',
        ...(key ? { Authorization: `Bearer ${key}` } : {}),
      },
      body: JSON.stringify(body),
    },
    {
      onChunk: (delta) => parseSse(delta, ev),
      onEnd: () => ev.onDone('stop'),
      onError: (message) => ev.onError(message),
    },
  );
}

// Minimal SSE frame parser: frames are `event:…\ndata:…\n\n`. Network
// chunks can split frames arbitrarily, so a carry buffer is required.
let sseBuffer = '';

function parseSse(delta: string, ev: StreamEvents): void {
  sseBuffer += delta;
  let boundary: number;
  while ((boundary = sseBuffer.indexOf('\n\n')) >= 0) {
    const frame = sseBuffer.slice(0, boundary);
    sseBuffer = sseBuffer.slice(boundary + 2);
    for (const line of frame.split('\n')) {
      if (!line.startsWith('data:')) continue;
      const payload = line.slice(5).trim();
      if (!payload || payload === '[DONE]') continue;
      let parsed: {
        choices?: {
          delta?: { content?: string; tool_calls?: { index: number; id?: string; function?: { name?: string; arguments?: string } }[] };
          finish_reason?: string | null;
        }[];
        error?: { message?: string };
      };
      try {
        parsed = JSON.parse(payload);
      } catch {
        continue;
      }
      if (parsed.error?.message) {
        ev.onError(parsed.error.message);
        return;
      }
      const choice = parsed.choices?.[0];
      if (!choice) continue;
      if (choice.delta?.content) ev.onDelta(choice.delta.content);
      for (const call of choice.delta?.tool_calls ?? []) {
        ev.onToolCall({
          id: call.id ?? '',
          name: call.function?.name ?? '',
          arguments: call.function?.arguments ?? '',
        });
      }
      if (choice.finish_reason) ev.onDone(choice.finish_reason);
    }
  }
}

// ---------------------------------------------------------------------------
// conversation flow
// ---------------------------------------------------------------------------

function appendUi(msg: UiMessage): void {
  ui.push(msg);
  draw();
}

async function knownTools(): Promise<ToolSpec[]> {
  const registered = await ctx.ai.listTools().catch(() => []);
  return registered.map((t) => ({
    name: t.name,
    description: t.description,
    parameters: (t.parameters ?? { type: 'object', properties: {} }) as Record<string, unknown>,
    highRisk: t.highRisk,
  }));
}

/** One assistant turn: stream deltas, collect tool calls, loop on results. */
async function runTurn(): Promise<void> {
  streaming = true;
  draw();

  const tools = await knownTools();
  let assistant = '';
  const toolCalls = new Map<number, ToolCall>();

  await chatRequest(conversation, tools, {
    onDelta: (t) => {
      assistant += t;
      updateLastAssistant(assistant);
    },
    onToolCall: (call) => {
      const existing = [...toolCalls.entries()].find(([, c]) => c.id && c.id === call.id)?.[1];
      if (existing) existing.arguments += call.arguments;
      else if (call.name) toolCalls.set(toolCalls.size, { ...call });
    },
    onDone: (reason) => {
      void (async () => {
        if (reason === 'tool_calls' && toolCalls.size > 0) {
          conversation.push({
            role: 'assistant',
            content: assistant || null,
            tool_calls: [...toolCalls.values()].map((c) => ({
              id: c.id,
              type: 'function' as const,
              function: { name: c.name, arguments: c.arguments },
            })),
          });
          await runToolCalls([...toolCalls.values()]);
          await runTurn(); // feed results back for the next turn
          return;
        }
        if (assistant) conversation.push({ role: 'assistant', content: assistant });
        streaming = false;
        draw();
      })();
    },
    onError: (message) => {
      appendUi({ role: 'error', content: `request failed: ${message}` });
      streaming = false;
      draw();
    },
  });
}

async function send(text: string): Promise<void> {
  if (streaming || !text.trim()) return;
  conversation.push({ role: 'user', content: text });
  appendUi({ role: 'user', content: text });
  await runTurn();
}

async function runToolCalls(calls: ToolCall[]): Promise<void> {
  for (const call of calls) {
    appendUi({ role: 'tool', content: `⚙ ${call.name}(${call.arguments})` });
    // High-risk tools need a visible confirmation (spec §54).
    if (call.name.startsWith('shell') || call.name.includes('delete')) {
      if (!confirm(`The assistant wants to run tool “${call.name}”. Allow?`)) {
        conversation.push({ role: 'tool', content: JSON.stringify({ error: 'denied by user' }) });
        continue;
      }
    }
    try {
      const args = JSON.parse(call.arguments || '{}') as Record<string, unknown>;
      const outcome = (await ctx.ai.callTool(call.name, args)) as { ok?: boolean; result?: unknown; error?: string };
      conversation.push({
        role: 'tool',
        tool_call_id: call.id,
        content: JSON.stringify(outcome?.ok === false ? { error: outcome.error } : (outcome?.result ?? null)),
      });
    } catch (err) {
      conversation.push({
        role: 'tool',
        tool_call_id: call.id,
        content: JSON.stringify({ error: String(err) }),
      });
    }
  }
}

function updateLastAssistant(text: string): void {
  const last = ui[ui.length - 1];
  if (last?.role === 'assistant') last.content = text;
  else ui.push({ role: 'assistant', content: text });
  draw();
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

function actionsBar(text: string): HTMLElement {
  return h(
    'div',
    { class: 'actions' },
    h(
      'button',
      {
        onclick: () => {
          void navigator.clipboard.writeText(text);
        },
      },
      'Copy',
    ),
    h(
      'button',
      {
        onclick: () => void ctx.events.emit('memo.createFromText', { text }),
      },
      'Save as Memo',
    ),
  );
}

function messageNode(m: UiMessage): HTMLElement {
  if (m.role === 'user') {
    return h('div', { class: 'msg', 'data-role': 'user' }, m.content);
  }
  if (m.role === 'error') {
    return h('div', { class: 'msg', 'data-role': 'error' }, m.content);
  }
  if (m.role === 'tool') {
    return h('div', { class: 'msg', 'data-role': 'tool' }, m.content);
  }
  return h(
    'div',
    { class: 'msg', 'data-role': 'assistant' },
    renderMarkdown(m.content),
    m.content ? actionsBar(m.content) : h('span', { class: 'meta' }, '…'),
  );
}

function draw(): void {
  const app = document.getElementById('app');
  if (!app || ctx.surface !== 'view:eigendesk.ai.chat') return;
  const log = document.querySelector('.ai-log');
  const atBottom = !!log && log.scrollHeight - log.scrollTop - log.clientHeight < 40;

  const input = h('textarea', {
    placeholder: 'Ask anything… (Enter to send, Shift+Enter for newline)',
    'aria-label': 'Message',
  }) as HTMLTextAreaElement;
  const sendBtn = h('button', { disabled: streaming }, streaming ? '…' : 'Send');
  const submit = () => {
    const text = input.value;
    if (!text.trim() || streaming) return;
    input.value = '';
    void send(text);
  };
  input.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && !ev.shiftKey) {
      ev.preventDefault();
      submit();
    }
  });
  sendBtn.addEventListener('click', submit);

  render(
    app,
    h(
      'div',
      { class: 'ai-app' },
      h(
        'div',
        { class: 'ai-header' },
        h('span', { class: 'dot', 'data-ok': String(configured) }),
        configured ? `${modelId} · ${endpoint}` : 'not configured',
      ),
      h('div', { class: 'ai-log' }, ui.map(messageNode)),
      h('div', { class: 'ai-privacy' }, configured ? `Sends to ${endpoint} · model ${modelId}` : ''),
      h('div', { class: 'ai-input' }, input, sendBtn),
    ),
  );

  const newLog = document.querySelector('.ai-log');
  if (newLog && atBottom) newLog.scrollTop = newLog.scrollHeight;
  if (document.activeElement === document.body) {
    const ta = document.querySelector<HTMLTextAreaElement>('.ai-input textarea');
    if (ta && ui.length > 0) ta.focus();
  }
}

async function checkConfig(): Promise<void> {
  const base = (await ctx.settings.get<string>('eigendesk.ai.baseUrl')) ?? '';
  const model = (await ctx.settings.get<string>('eigendesk.ai.model')) ?? '';
  endpoint = base;
  modelId = model;
  configured = !!(base && model);
}

definePlugin({
  async activate(context) {
    ctx = context;

    // Quick Ask entry (from the capture router `?` prefix).
    ctx.commands.onCommand((id, args) => {
      if (id === 'eigendesk.ai.quickAsk') {
        const question = (args ?? '').trim();
        if (!question) return Promise.resolve();
        ui = [];
        conversation = [];
        appendUi({ role: 'user', content: question });
        return send(question);
      }
      if (id === 'eigendesk.ai.open') {
        return checkConfig().then(draw);
      }
      return undefined;
    });

    ctx.events.on('ai.quickAskText', (data) => {
      const text = typeof (data as { text?: string })?.text === 'string' ? (data as { text: string }).text : '';
      if (text.trim()) void send(text);
    });

    if (ctx.surface === 'view:eigendesk.ai.chat') {
      await checkConfig();
      draw();
    }
  },
});
