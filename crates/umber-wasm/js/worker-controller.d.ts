import type { CompileOutput, SessionOptions } from "./umber_wasm.js";
import type { HttpManifestResolverOptions } from "./manifest-resolver.js";

export interface WorkerCompileControl {
	signal?: AbortSignal;
	timeoutMs?: number;
	workerUrl?: string | URL;
	wasmUrl?: string | URL;
	Worker?: typeof Worker;
}

export class WorkerCompileError extends Error {
	readonly code: string;
	readonly diagnostic?: import("./umber_wasm.js").Diagnostic;
}

export function compileInWorker(
	options: SessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: Pick<
		HttpManifestResolverOptions,
		| "manifestUrl"
		| "manifestSha256"
		| "persistentCache"
		| "offline"
		| "concurrency"
	> & {
		format?: string;
		/** Exact application/private responses, matched by the complete typed request key. */
		resourceResponses?: import("./resource-resolver.js").TypedResourceResponse[];
	},
	control?: WorkerCompileControl,
): Promise<CompileOutput>;
