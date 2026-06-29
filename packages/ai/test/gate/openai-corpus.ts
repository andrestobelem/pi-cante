// Adversarial corpus for the OpenAI / Ollama chunk-object SSE-decoder contract gate.
//
// Unlike the Anthropic gate (which records raw SSE bytes), the OpenAI SDK decodes SSE internally and
// exposes an AsyncIterable<ChatCompletionChunk>. There is no byte-injection seam, so this gate operates
// one level up: each fixture is an ordered array of already-parsed chunk OBJECTS. The array order IS the
// chunk-boundary contract (the analog of split boundaries in the byte gate) — to stress tool-argument
// reassembly we author multiple chunks each carrying a fragment of `function.arguments`.
//
// OpenAI and Ollama share the exact same `stream()` code path (openai-completions.ts); `provider` only
// selects which Model literal the replay harness builds, it does NOT change decode logic. The TS decoder
// is the oracle; the Rust `decode_openai` must reproduce the normalized transcript byte-for-byte.
//
// NOTE: the encrypted `reasoning_details` / `thoughtSignature` path is intentionally OUT OF SCOPE for this
// increment (deferred to a dedicated increment that adds serde_json `preserve_order`, since JSON.stringify
// key-order parity is unachievable with the current crate config). No fixture here emits reasoning_details.

export type OpenAIFixture = {
	name: string;
	provider: "openai" | "ollama";
	// Already-parsed chunk objects (plus deliberate junk in robustness fixtures). Typed `unknown` because
	// some fixtures feed non-object junk (null/number) to exercise the `typeof chunk !== "object"` guard.
	chunks: unknown[];
};

// Minimal chunk: a deterministic id (so responseId is reproducible) and an empty choices array. `model` is
// deliberately omitted by default so `responseModel` stays suppressed (the production guard requires a
// non-empty chunk.model that differs from model.id). Override any field via `overrides`.
function chunk(overrides: Record<string, unknown>): Record<string, unknown> {
	return { id: "chatcmpl_test", choices: [], ...overrides };
}

const usageStd = {
	prompt_tokens: 100,
	completion_tokens: 20,
	prompt_tokens_details: { cached_tokens: 30, cache_write_tokens: 10 },
	completion_tokens_details: { reasoning_tokens: 5 },
};

// Mirrors the Anthropic gate's tool fragments: tokens split across deltas (the #1 reassembly risk).
const toolFragments = ['{"path":"', "/foo/b", 'ar","text":"col1\\tcol2', '"}'];

export const fixtures: OpenAIFixture[] = [
	// ── Text path ───────────────────────────────────────────────────────────────────────────────────
	{
		name: "text-single-chunk",
		provider: "openai",
		chunks: [
			chunk({
				choices: [{ index: 0, delta: { content: "Hello" }, finish_reason: "stop" }],
				usage: { prompt_tokens: 12, completion_tokens: 5 },
			}),
		],
	},
	{
		name: "text-multi-delta",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { content: "Hel" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { content: "lo" }, finish_reason: null }] }),
			chunk({
				choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
				usage: { prompt_tokens: 12, completion_tokens: 5 },
			}),
		],
	},
	{
		name: "text-empty-and-null-content",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { content: "" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { content: null }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { content: "Hi" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }),
		],
	},

	// ── Tool-call path ──────────────────────────────────────────────────────────────────────────────
	{
		name: "tool-call-fragments",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: {
							tool_calls: [{ index: 0, id: "call_1", function: { name: "edit", arguments: toolFragments[0] } }],
						},
						finish_reason: null,
					},
				],
			}),
			...toolFragments.slice(1).map((frag) =>
				chunk({
					choices: [
						{
							index: 0,
							delta: { tool_calls: [{ index: 0, function: { arguments: frag } }] },
							finish_reason: null,
						},
					],
				}),
			),
			chunk({
				choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
				usage: { prompt_tokens: 12, completion_tokens: 8 },
			}),
		],
	},
	{
		name: "tool-call-id-late",
		provider: "ollama",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, function: { name: "search", arguments: '{"q":"' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "call_late", function: { arguments: 'hi"}' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		name: "tool-call-id-only-no-index",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ id: "call_x", function: { name: "f", arguments: "{}" } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		name: "tool-call-name-resend",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c1", function: { name: "first", arguments: "" } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, function: { name: "second", arguments: '{"a":1}' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		name: "tool-call-empty-arg-delta",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c1", function: { name: "f" } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		name: "two-concurrent-tool-calls",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c0", function: { name: "a", arguments: '{"x":' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 1, id: "c1", function: { name: "b", arguments: '{"y":' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{ index: 0, delta: { tool_calls: [{ index: 0, function: { arguments: "1}" } }] }, finish_reason: null },
				],
			}),
			chunk({
				choices: [
					{ index: 0, delta: { tool_calls: [{ index: 1, function: { arguments: "2}" } }] }, finish_reason: null },
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		name: "tool-malformed-json-repair",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						// Invalid \H escape + a raw tab char inside the args string; repaired by parseStreamingJson.
						delta: {
							tool_calls: [
								{
									index: 0,
									id: "c1",
									function: { name: "edit", arguments: '{"path":"A\\H","text":"col1\tcol2"}' },
								},
							],
						},
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},

	// ── Reasoning path ──────────────────────────────────────────────────────────────────────────────
	{
		name: "reasoning-then-text",
		provider: "ollama",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { reasoning_content: "Let me" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { reasoning_content: " think" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { content: "Answer" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }),
		],
	},
	{
		name: "reasoning-field-precedence",
		provider: "openai",
		chunks: [
			// All three fields present in one delta: first-non-empty [reasoning_content, reasoning, reasoning_text] wins.
			chunk({
				choices: [
					{
						index: 0,
						delta: { reasoning_content: "RC", reasoning: "R", reasoning_text: "RT" },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }),
		],
	},
	{
		name: "reasoning-signature-locked-on-first",
		provider: "openai",
		chunks: [
			// chunk1 uses field `reasoning` (signature "reasoning"); chunk2 carries `reasoning_content` (higher
			// precedence) but the block already exists, so the signature must stay "reasoning" (locked on creation).
			chunk({ choices: [{ index: 0, delta: { reasoning: "a" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { reasoning_content: "b" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }),
		],
	},

	// ── Stop-reason / termination path ──────────────────────────────────────────────────────────────
	{
		name: "length-finish-reason",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "x" }, finish_reason: "length" }] })],
	},
	{
		name: "end-alias-finish-reason",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "x" }, finish_reason: "end" }] })],
	},
	{
		name: "toolUse-via-function_call",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c1", function: { name: "f", arguments: "{}" } }] },
						finish_reason: "function_call",
					},
				],
			}),
		],
	},
	{
		name: "content_filter-error",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "partial" }, finish_reason: "content_filter" }] })],
	},
	{
		name: "network_error-error",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "partial" }, finish_reason: "network_error" }] })],
	},
	{
		name: "unknown-finish-reason",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "partial" }, finish_reason: "weird_reason" }] })],
	},
	{
		name: "stream-ended-without-finish-reason",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { content: "Hel" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: { content: "lo" }, finish_reason: null }] }),
		],
	},

	// ── Robustness path ─────────────────────────────────────────────────────────────────────────────
	{
		name: "unknown-trailing-chunks",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { content: "hi" }, finish_reason: "stop" }] }),
			null,
			42,
			chunk({ choices: [] }),
		],
	},
	{
		name: "responseModel-differs",
		provider: "openai",
		chunks: [
			chunk({ model: "served-model-v2", choices: [{ index: 0, delta: { content: "hi" }, finish_reason: "stop" }] }),
		],
	},
	{
		name: "responseId-omitted",
		provider: "openai",
		chunks: [chunk({ id: undefined, choices: [{ index: 0, delta: { content: "hi" }, finish_reason: "stop" }] })],
	},
	{
		name: "responseId-late",
		provider: "openai",
		chunks: [
			chunk({ id: undefined, choices: [{ index: 0, delta: { content: "a" }, finish_reason: null }] }),
			chunk({ id: "chatcmpl_late", choices: [{ index: 0, delta: { content: "b" }, finish_reason: "stop" }] }),
		],
	},

	// ── Usage path ──────────────────────────────────────────────────────────────────────────────────
	{
		name: "usage-chunk-standard",
		provider: "openai",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "x" }, finish_reason: "stop" }], usage: usageStd })],
	},
	{
		name: "usage-cached-zero-not-fallthrough",
		provider: "openai",
		chunks: [
			// cached_tokens is a literal 0; the `??` chain must NOT fall through to prompt_cache_hit_tokens (7).
			chunk({
				choices: [{ index: 0, delta: { content: "x" }, finish_reason: "stop" }],
				usage: {
					prompt_tokens: 50,
					completion_tokens: 10,
					prompt_cache_hit_tokens: 7,
					prompt_tokens_details: { cached_tokens: 0 },
				},
			}),
		],
	},
	{
		name: "usage-prompt-cache-hit-fallback",
		provider: "openai",
		chunks: [
			// No prompt_tokens_details.cached_tokens → the `??` chain falls back to prompt_cache_hit_tokens (7).
			chunk({
				choices: [{ index: 0, delta: { content: "x" }, finish_reason: "stop" }],
				usage: { prompt_tokens: 50, completion_tokens: 10, prompt_cache_hit_tokens: 7 },
			}),
		],
	},
	{
		name: "usage-moonshot-choice-fallback",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { content: "x" },
						finish_reason: "stop",
						usage: { prompt_tokens: 10, completion_tokens: 2 },
					},
				],
			}),
		],
	},
	{
		name: "usage-only-final-chunk",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { content: "hi" }, finish_reason: "stop" }] }),
			chunk({ choices: [], usage: { prompt_tokens: 10, completion_tokens: 5 } }),
		],
	},
	{
		name: "usage-absent-ollama",
		provider: "ollama",
		chunks: [chunk({ choices: [{ index: 0, delta: { content: "hi" }, finish_reason: "stop" }] })],
	},
	{
		name: "usage-last-write-wins",
		provider: "openai",
		chunks: [
			chunk({
				choices: [{ index: 0, delta: { content: "a" }, finish_reason: null }],
				usage: { prompt_tokens: 1, completion_tokens: 1 },
			}),
			chunk({
				// chunk.usage B present → choice.usage C is ignored; B overwrites A.
				choices: [
					{
						index: 0,
						delta: { content: "b" },
						finish_reason: "stop",
						usage: { prompt_tokens: 99, completion_tokens: 99 },
					},
				],
				usage: { prompt_tokens: 5, completion_tokens: 5 },
			}),
		],
	},

	// ── Intra-chunk ordering (adversarial-review additions) ───────────────────────────────────────────
	{
		name: "multimodal-single-delta",
		provider: "openai",
		chunks: [
			// One delta with content + reasoning + tool_calls: blocks must be created in source order
			// text(0) -> thinking(1) -> toolCall(2).
			chunk({
				choices: [
					{
						index: 0,
						delta: {
							content: "T",
							reasoning_content: "R",
							tool_calls: [{ index: 0, id: "c0", function: { name: "f", arguments: "{}" } }],
						},
						finish_reason: "stop",
					},
				],
			}),
		],
	},
	{
		name: "text-after-tool",
		provider: "openai",
		chunks: [
			// Tool block created first (contentIndex 0); text block created lazily later (contentIndex 1, not forced to 0).
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c0", function: { name: "f", arguments: "{}" } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: { content: "text" }, finish_reason: "stop" }] }),
		],
	},
	{
		name: "finish-with-toolcall-same-chunk",
		provider: "openai",
		chunks: [
			// finish_reason and a tool-call args fragment arrive in the same delta: toolcall_delta still emits
			// before the post-loop toolcall_end.
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ index: 0, id: "c0", function: { name: "f", arguments: '{"a":1}' } }] },
						finish_reason: "tool_calls",
					},
				],
			}),
		],
	},

	// ── Adversarial-review coverage additions (untested branches that a future Rust edit could break) ──
	{
		// Two tool calls in ONE delta array (canonical OpenAI parallel-tool-call wire). Every other tool
		// fixture iterates the per-delta loop once; this creates two blocks in a single iteration and pins
		// the block-creation order + contentIndex assignment + back-to-back start/delta pairs.
		name: "two-tool-calls-one-delta",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: {
							tool_calls: [
								{ index: 0, id: "c0", function: { name: "a", arguments: '{"x":1}' } },
								{ index: 1, id: "c1", function: { name: "b", arguments: '{"y":2}' } },
							],
						},
						finish_reason: null,
					},
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		// streamIndex back-fill onto an id-only block: chunk1 has id but no index; chunk2 supplies the index
		// (matched by id) -> back-fills toolCallBlocksByIndex; chunk3 is index-only and MUST route to that
		// same block via the back-filled index (not spawn a second block). A regression dropping the back-fill
		// makes chunk3 create a second block -> final would have two tool blocks instead of one.
		name: "tool-call-index-backfill",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ id: "cx", function: { name: "f", arguments: '{"a":' } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{
						index: 0,
						delta: { tool_calls: [{ id: "cx", index: 0, function: { arguments: "1}" } }] },
						finish_reason: null,
					},
				],
			}),
			chunk({
				choices: [
					{ index: 0, delta: { tool_calls: [{ index: 0, function: { arguments: "" } }] }, finish_reason: null },
				],
			}),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] }),
		],
	},
	{
		// Reasoning field present but EMPTY (reasoning_content:"") with a later non-empty field (reasoning):
		// pins first-NON-EMPTY selection (signature "reasoning"), not first-PRESENT (which would be
		// "reasoning_content"). The existing precedence fixture never has an empty higher-precedence field.
		name: "reasoning-empty-field-skipped",
		provider: "openai",
		chunks: [
			chunk({ choices: [{ index: 0, delta: { reasoning_content: "", reasoning: "R" }, finish_reason: null }] }),
			chunk({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }),
		],
	},
	{
		// Content + tool_calls in one delta WITHOUT reasoning: the tool block must land at contentIndex 1
		// (right after text), not at index 2 with a phantom thinking slot reserved. The only other intra-delta
		// multi-block fixture always includes reasoning.
		name: "content-and-tool-one-delta",
		provider: "openai",
		chunks: [
			chunk({
				choices: [
					{
						index: 0,
						delta: {
							content: "hi",
							tool_calls: [{ index: 0, id: "c0", function: { name: "f", arguments: "{}" } }],
						},
						finish_reason: "stop",
					},
				],
			}),
		],
	},
];
