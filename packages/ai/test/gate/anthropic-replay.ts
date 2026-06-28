// Replay harness for the Anthropic byte-level contract gate.
//
// Feeds recorded raw SSE byte chunks (in order, preserving split boundaries) through the production
// TS decoder via the `options.client` seam, and captures the normalized transcript (the ordered
// AssistantMessageEvent sequence with per-event message snapshots, plus the final message).
//
// CRITICAL: every event aliases the same mutable `output`, and events are queued then drained, so we
// snapshot synchronously in the for-await body (normMessage value-copies primitives + clones tool
// arguments) before the producer mutates output again.

import type Anthropic from "@anthropic-ai/sdk";
import { stream as streamAnthropic } from "../../src/api/anthropic-messages.ts";
import { getModel } from "../../src/compat.ts";
import type { AssistantMessage, AssistantMessageEvent, Context } from "../../src/types.ts";

export const MODEL_PROVIDER = "anthropic";
export const MODEL_ID = "claude-haiku-4-5";

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

// Strip non-deterministic / scratch fields (timestamp, cost, block index/partialJson); keep everything else.
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
	out.content = m.content.map(normBlock);
	out.usage = usage;
	out.stopReason = m.stopReason;
	if (m.errorMessage !== undefined) {
		out.errorMessage = m.errorMessage;
	}
	return out;
}

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
	const snap = ev.type === "done" ? ev.message : ev.type === "error" ? ev.error : (rec.partial as AssistantMessage);
	// Per-event snapshot captures only the content progression (deterministic, lock-step). Message-level
	// terminal fields (stopReason/errorMessage/usage) settle deterministically only in `final`.
	out.snapshot = snap.content.map(normBlock);
	return out;
}

function fakeClient(response: Response): Anthropic {
	return {
		messages: { create: () => ({ asResponse: async () => response }) },
	} as unknown as Anthropic;
}

export function toByteChunks(wireChunks: string[]): number[][] {
	const encoder = new TextEncoder();
	return wireChunks.map((chunk) => Array.from(encoder.encode(chunk)));
}

export async function decodeTranscript(byteChunks: number[][]): Promise<{
	api: string;
	provider: string;
	model: string;
	transcript: { events: Record<string, unknown>[]; final: Record<string, unknown> };
}> {
	const model = getModel(MODEL_PROVIDER, MODEL_ID);
	const body = new ReadableStream<Uint8Array>({
		start(controller) {
			for (const bytes of byteChunks) {
				controller.enqueue(new Uint8Array(bytes));
			}
			controller.close();
		},
	});
	const response = new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
	const context: Context = { messages: [{ role: "user", content: "x", timestamp: 0 }] };

	const stream = streamAnthropic(model, context, { client: fakeClient(response) });
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
