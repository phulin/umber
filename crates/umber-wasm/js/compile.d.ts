import type {
	CompileOutput,
	FileRequest,
	FileRequestKey,
	SessionOptions,
} from "./umber_wasm.js";

export interface ResolvedDownload {
	request: FileRequestKey;
	virtualPath: string;
	bytes: Uint8Array;
}

export interface FileResolver {
	resolve(
		requests: readonly FileRequest[],
		signal?: AbortSignal,
	): Promise<readonly ResolvedDownload[]>;
}

export interface CompilerBindings {
	CompilerSession: new (
		options: SessionOptions,
	) => {
		addUserFile(path: string, bytes: Uint8Array): void;
		provideResolvedFile(
			request: FileRequestKey,
			virtualPath: string,
			bytes: Uint8Array,
		): void;
		compileAttempt(): import("./umber_wasm.js").AttemptResult;
		dispose(): void;
	};
}

export class CompileFacadeError extends Error {
	readonly code: string;
	readonly diagnostic?: import("./umber_wasm.js").Diagnostic;
}

export function compile(
	options: SessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: FileResolver,
	signal?: AbortSignal,
	bindings?: CompilerBindings,
): Promise<CompileOutput>;
