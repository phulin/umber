import type {
	CompileOutput,
	ResourceRequest,
	ResourceResponse,
	SessionLimits,
	SessionOptions,
} from "./umber_wasm.js";

export interface ResourceResolver {
	resolve(
		requests: readonly ResourceRequest[],
		options?: {
			signal?: AbortSignal;
			prefetchHints?: readonly ResourceRequest[];
		},
	): Promise<readonly ResourceResponse[]>;
}

export interface CompilerBindings {
	CompilerSession: new (
		options: SessionOptions,
	) => {
		addUserFile(path: string, bytes: Uint8Array): void;
		addHtmlFont(font: import("./umber_wasm.js").HtmlFontInput): void;
		provideResources(responses: ResourceResponse[]): void;
		advance?(): import("./umber_wasm.js").AttemptResult;
		compileAttempt(): import("./umber_wasm.js").AttemptResult;
		dispose(): void;
	};
}

export class CompileFacadeError extends Error {
	readonly code: string;
	readonly diagnostic?: import("./umber_wasm.js").Diagnostic;
}

/** Validates and fills the limits shared by local and worker compilation. */
export function validateSessionLimits(
	limits?: Partial<SessionLimits>,
): SessionLimits;

export function compile(
	options: SessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: ResourceResolver,
	signal?: AbortSignal,
	bindings?: CompilerBindings,
): Promise<CompileOutput>;
