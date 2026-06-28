// Adversarial corpus for the Anthropic byte-level SSE-decoder contract gate.
//
// Each fixture is a list of raw SSE wire chunks (strings, UTF-8 encoded to bytes at gate time).
// Splitting the wire across chunks — including mid-line and across an SSE event — exercises the
// incremental framer (TextDecoder{stream:true} + consumeLine). The TS decoder is the oracle; the
// Rust port must reproduce the normalized transcript byte-for-byte.

export type AnthropicFixture = { name: string; chunks: string[] };

// Build the SSE wire exactly like the production tests' createSseResponse:
//   each event -> `event: <event>\ndata: <data>\n`, joined with "\n" (so events are separated by a blank line).
function wire(events: Array<{ event: string; data: string }>): string {
	return events.map(({ event, data }) => `event: ${event}\ndata: ${data}\n`).join("\n");
}

function j(value: unknown): string {
	return JSON.stringify(value);
}

const usage0 = { input_tokens: 12, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 };
const usageFinal = { input_tokens: 12, output_tokens: 5, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 };

const helloEvents = [
	{ event: "message_start", data: j({ type: "message_start", message: { id: "msg_test", usage: usage0 } }) },
	{
		event: "content_block_start",
		data: j({ type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }),
	},
	{
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hel" } }),
	},
	{
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "lo" } }),
	},
	{ event: "content_block_stop", data: j({ type: "content_block_stop", index: 0 }) },
	{
		event: "message_delta",
		data: j({ type: "message_delta", delta: { stop_reason: "end_turn" }, usage: usageFinal }),
	},
	{ event: "message_stop", data: j({ type: "message_stop" }) },
];

// A tool call whose JSON arguments arrive in fragments that split JSON tokens across deltas
// (the #1 partial-JSON-reassembly risk, now at the streaming level).
const toolFragments = ['{"path":"', "/foo/b", 'ar","text":"col1\\tcol2', '"}'];
const toolEvents = [
	{ event: "message_start", data: j({ type: "message_start", message: { id: "msg_tool", usage: usage0 } }) },
	{
		event: "content_block_start",
		data: j({
			type: "content_block_start",
			index: 0,
			content_block: { type: "tool_use", id: "toolu_1", name: "edit", input: {} },
		}),
	},
	...toolFragments.map((frag) => ({
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: frag } }),
	})),
	{ event: "content_block_stop", data: j({ type: "content_block_stop", index: 0 }) },
	{
		event: "message_delta",
		data: j({ type: "message_delta", delta: { stop_reason: "tool_use" }, usage: usageFinal }),
	},
	{ event: "message_stop", data: j({ type: "message_stop" }) },
];

// Malformed streamed tool JSON (invalid \H escape + raw tab) — repaired by repairJson at both the
// SSE-data and the partial-json levels. Mirrors the production anthropic-sse-parsing.test.ts case.
const malformedToolDelta = String.raw`{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"A\H\",\"text\":\"col1	col2\"}"}}`;
const malformedToolEvents = [
	{ event: "message_start", data: j({ type: "message_start", message: { id: "msg_mal", usage: usage0 } }) },
	{
		event: "content_block_start",
		data: j({
			type: "content_block_start",
			index: 0,
			content_block: { type: "tool_use", id: "toolu_mal", name: "edit", input: {} },
		}),
	},
	{ event: "content_block_delta", data: malformedToolDelta },
	{ event: "content_block_stop", data: j({ type: "content_block_stop", index: 0 }) },
	{
		event: "message_delta",
		data: j({ type: "message_delta", delta: { stop_reason: "tool_use" }, usage: usageFinal }),
	},
	{ event: "message_stop", data: j({ type: "message_stop" }) },
];

const refusalExplanation = "This request triggered restrictions and was blocked under Anthropic's Usage Policy.";
const refusalEvents = [
	{
		event: "message_start",
		data: j({
			type: "message_start",
			message: {
				id: "msg_ref",
				usage: { input_tokens: 412, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 },
			},
		}),
	},
	{
		event: "message_delta",
		data: j({
			type: "message_delta",
			delta: {
				stop_reason: "refusal",
				stop_details: { type: "refusal", category: "cyber", explanation: refusalExplanation },
			},
			usage: { input_tokens: 412, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 },
		}),
	},
	{ event: "message_stop", data: j({ type: "message_stop" }) },
];

const thinkingEvents = [
	{ event: "message_start", data: j({ type: "message_start", message: { id: "msg_think", usage: usage0 } }) },
	{
		event: "content_block_start",
		data: j({ type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "" } }),
	},
	{
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "Let me think" } }),
	},
	{
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sig-abc" } }),
	},
	{ event: "content_block_stop", data: j({ type: "content_block_stop", index: 0 }) },
	{
		event: "content_block_start",
		data: j({ type: "content_block_start", index: 1, content_block: { type: "text", text: "" } }),
	},
	{
		event: "content_block_delta",
		data: j({ type: "content_block_delta", index: 1, delta: { type: "text_delta", text: "Answer" } }),
	},
	{ event: "content_block_stop", data: j({ type: "content_block_stop", index: 1 }) },
	{
		event: "message_delta",
		data: j({
			type: "message_delta",
			delta: { stop_reason: "end_turn" },
			usage: { ...usageFinal, output_tokens_details: { thinking_tokens: 3 } },
		}),
	},
	{ event: "message_stop", data: j({ type: "message_stop" }) },
];

const helloWire = wire(helloEvents);

// Split a string into n roughly-equal pieces (mid-line / mid-event splits stress the framer).
function splitEvenly(text: string, n: number): string[] {
	const size = Math.ceil(text.length / n);
	const out: string[] = [];
	for (let i = 0; i < text.length; i += size) {
		out.push(text.slice(i, i + size));
	}
	return out;
}

export const fixtures: AnthropicFixture[] = [
	{ name: "hello-one-chunk", chunks: [helloWire] },
	{ name: "hello-split-per-char-region", chunks: splitEvenly(helloWire, 7) },
	{ name: "hello-split-mid", chunks: [helloWire.slice(0, 30), helloWire.slice(30)] },
	{ name: "tool-partial-json-fragments", chunks: [wire(toolEvents)] },
	{ name: "tool-partial-json-fragments-split", chunks: splitEvenly(wire(toolEvents), 9) },
	{ name: "tool-malformed-json-repair", chunks: [wire(malformedToolEvents)] },
	{ name: "refusal", chunks: [wire(refusalEvents)] },
	{ name: "thinking-then-text", chunks: [wire(thinkingEvents)] },
	{ name: "thinking-then-text-split", chunks: splitEvenly(wire(thinkingEvents), 11) },
	{
		name: "unknown-events-after-stop",
		chunks: [wire([...helloEvents, { event: "done", data: "[DONE]" }, { event: "proxy.stats", data: "not json" }])],
	},
	{ name: "comment-and-heartbeat-lines", chunks: [`:heartbeat\n${helloWire}`] },
	{ name: "ended-before-message-stop", chunks: [wire(helloEvents.slice(0, 5))] },
	{ name: "event-error-midstream", chunks: [wire([helloEvents[0]]) + "\nevent: error\ndata: upstream exploded\n"] },
];
