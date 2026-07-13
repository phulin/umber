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
		"manifestUrl" | "persistentCache" | "concurrency"
	> & { format?: string },
	control?: WorkerCompileControl,
): Promise<CompileOutput>;
