/* tslint:disable */
/* eslint-disable */

/**
 * Verify a precomputed-block JSON against `network`'s embedded verification key.
 *
 * `network` is "devnet" or "mainnet" (uses the embedded VK without touching any
 * process-global config). Returns a JSON string
 * `{ "height", "stateHash", "previousStateHash", "stagedLedgerHash" }` on success, or
 * a JS error (string) if the JSON is malformed or the proof does not verify — in which
 * case the block must NOT be ingested.
 */
export function verifyPrecomputed(network: string, precomputed_json: string): string;

/**
 * Entry point for web workers
 */
export function wasm_thread_entry_point(ptr: number): void;
