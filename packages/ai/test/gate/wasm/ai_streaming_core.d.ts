/* tslint:disable */
/* eslint-disable */
/**
 * `chunks_json` = `JSON.stringify` of a golden row's `chunks` (an array of chunk objects).
 * Deserializes to the exact `Vec<serde_json::Value>` the native conformance test feeds `decode_openai`.
 */
export function decode_openai_canonical(chunks_json: string, api: string, provider: string, model: string): string;
/**
 * `chunks_json` = `JSON.stringify` of a golden row's `chunks` (an array of byte-arrays).
 * Deserializes to the exact `Vec<Vec<u8>>` the native conformance test feeds `decode_anthropic`.
 */
export function decode_anthropic_canonical(chunks_json: string, api: string, provider: string, model: string): string;
