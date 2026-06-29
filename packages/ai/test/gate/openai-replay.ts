// Replay harness for the OpenAI / Ollama chunk-object contract gate.
//
// Feeds an ordered array of already-parsed ChatCompletionChunk objects through the production TS decoder
// via the `options.client` seam (a fake client whose `chat.completions.create(...).withResponse()` resolves
// to `{ data: <async iterable of the chunks>, response }`), and captures the normalized transcript (the
// ordered AssistantMessageEvent sequence with per-event content-only snapshots, plus the final message).
//
// `getModel` is builtin-only, and Ollama is not a builtin, so we build Model literals directly (mirroring
// stream.test.ts). `provider` selects the literal but does NOT change decode logic — OpenAI and Ollama
// share the exact same `stream()`.
//
// CRITICAL: every event aliases the same mutable `output`, and events are queued then drained, so we
// snapshot synchronously in the for-await body (content-only) before the producer mutates output again.
// Message-level terminal fields (stopReason/errorMessage/usage) settle deterministically only in `final`.

import type OpenAI from "openai";
import { stream as streamOpenAI } from "../../src/api/openai-completions.ts";
import type { AssistantMessage, AssistantMessageEvent, Context, Model } from "../../src/types.ts";
import type { OpenAIFixture } from "./openai-corpus.ts";

type Block = AssistantMessage["content"][number];

function normBlock(b: Block): Record<string, unknown> {
	if (b.type === "text") {
		return { type: "text", text: b.text };
	}
	if (b.type === "thinking") {
		const out: Record<string, unknown> = {
			type: "thinking",
			thinking: b.thinking,
			thinkingSignature: b.thinkingSignature ?? "",
		};
		if (b.redacted) {
			out.redacted = true;
		}
		return out;
	}
	return { type: "toolCall", id: b.id, name: b.name, arguments: structuredClone(b.arguments) };
}

// Strip non-deterministic / scratch fields (timestamp, cost, block index/partialArgs/streamIndex); keep
// everything else. `responseModel` is emitted only when defined (the OpenAI loop sets it when chunk.model
// differs from model.id; Anthropic never sets it).
function normMessage(m: AssistantMessage): Record<string, unknown> {
	const usage: Record<string, unknown> = {
		input: m.usage.input,
		output: m.usage.output,
		cacheRead: m.usage.cacheRead,
		cacheWrite: m.usage.cacheWrite,
	};
	if (m.usage.cacheWrite1h !== undefined) {
		usage.cacheWrite1h = m.usage.cacheWrite1h;
	}
	if (m.usage.reasoning !== undefined) {
		usage.reasoning = m.usage.reasoning;
	}
	usage.totalTokens = m.usage.totalTokens;

	const out: Record<string, unknown> = { role: m.role, api: m.api, provider: m.provider, model: m.model };
	if (m.responseId !== undefined) {
		out.responseId = m.responseId;
	}
	if (m.responseModel !== undefined) {
		out.responseModel = m.responseModel;
	}
	out.content = m.content.map(normBlock);
	out.usage = usage;
	out.stopReason = m.stopReason;
	if (m.errorMessage !== undefined) {
		out.errorMessage = m.errorMessage;
	}
	return out;
}

// NOTE: unlike the Anthropic gate, we do NOT capture a per-event snapshot. Every event carries
// `partial: output` aliasing the same mutable message, and the EventStream delivers queued events to the
// consumer in a tight microtask burst (utils/event-stream.ts yields `queue.shift()` with no await) while
// the producer has already raced ahead mutating `output`. So a per-event snapshot reflects a content state
// AHEAD of synchronous emission — deterministic per run, but a consumer-drain timing artifact that a
// synchronous Rust decoder cannot reproduce. (The Anthropic producer awaits between events, so its
// snapshots are progressive; OpenAI's are not.) We gate the deterministic, reproducible contract instead:
// the event SEQUENCE (type/contentIndex/delta/content/reason) plus the fully-settled `final` message.
// (Capturing parsed `arguments` on the post-loop, settled `toolcall_end` event was considered — it is
// deterministic — but it is redundant: those args are the same block.arguments the `final` message already
// gates per tool block. So we keep the event sequence clean.)
function normEvent(ev: AssistantMessageEvent): Record<string, unknown> {
	const rec = ev as unknown as Record<string, unknown>;
	const out: Record<string, unknown> = { type: ev.type };
	if ("contentIndex" in rec) {
		out.contentIndex = rec.contentIndex;
	}
	if ("delta" in rec) {
		out.delta = rec.delta;
	}
	if ("content" in rec) {
		out.content = rec.content;
	}
	if ("reason" in rec) {
		out.reason = rec.reason;
	}
	return out;
}

const OPENAI_MODEL: Model<"openai-completions"> = {
	id: "gpt-4o-mini",
	name: "GPT-4o mini (gate)",
	api: "openai-completions",
	provider: "openai",
	baseUrl: "https://api.openai.com/v1",
	reasoning: true,
	input: ["text"],
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
	contextWindow: 128000,
	maxTokens: 16000,
};

const OLLAMA_MODEL: Model<"openai-completions"> = {
	id: "gpt-oss:20b",
	name: "Ollama GPT-OSS 20B (gate)",
	api: "openai-completions",
	provider: "ollama",
	baseUrl: "http://localhost:11434/v1",
	reasoning: true,
	input: ["text"],
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
	contextWindow: 128000,
	maxTokens: 16000,
};

// Fake client: `create(params, opts).withResponse()` -> { data: <async iterable of chunks>, response }.
// Only response.status / response.headers are read downstream (by onResponse, which the gate never passes).
function fakeClient(chunks: unknown[]): OpenAI {
	async function* gen(): AsyncGenerator<unknown> {
		for (const c of chunks) {
			yield c;
		}
	}
	return {
		chat: {
			completions: {
				create: () => ({
					withResponse: async () => ({ data: gen(), response: new Response(null, { status: 200 }) }),
				}),
			},
		},
	} as unknown as OpenAI;
}

export async function decodeTranscript(fixture: OpenAIFixture): Promise<{
	api: string;
	provider: string;
	model: string;
	transcript: { events: Record<string, unknown>[]; final: Record<string, unknown> };
}> {
	const model = fixture.provider === "ollama" ? OLLAMA_MODEL : OPENAI_MODEL;
	const context: Context = { messages: [{ role: "user", content: "x", timestamp: 0 }] };

	const stream = streamOpenAI(model, context, { client: fakeClient(fixture.chunks), apiKey: "test" });
	const events: Record<string, unknown>[] = [];
	for await (const ev of stream) {
		events.push(normEvent(ev));
	}
	const final = normMessage(await stream.result());

	return {
		api: model.api,
		provider: model.provider,
		model: model.id,
		transcript: { events, final },
	};
}
