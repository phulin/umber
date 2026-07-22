import type {
	CompileOutput,
	EditorAttemptResult,
	EditorSessionOptions,
	EditorStatus,
	ProjectCompileOutput,
	ProjectSessionOptions,
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
			probes?: readonly ResourceRequest[];
			prefetchHints?: readonly ResourceRequest[];
		},
	): Promise<readonly ResourceResponse[]>;
}

export interface CompilerBindings {
	CompilerSession: new (
		options: SessionOptions,
	) => {
		addUserFile(path: string, bytes: Uint8Array): void;
		provideResources(responses: ResourceResponse[]): void;
		applyPatch(patch: import("./umber_wasm.js").SourcePatch): void;
		renderedSourceLocation(
			page: number,
			event: number,
			unit: number | undefined,
			outputId: string,
			revision: number,
		): import("./umber_wasm.js").RenderedSourceResult | undefined;
		cancelPendingPatch(): boolean;
		readonly revision: number | undefined;
		readonly contentHash: string | undefined;
		readonly reuseMetrics: import("./umber_wasm.js").ReuseMetrics | undefined;
		readonly retentionMetrics:
			| import("./umber_wasm.js").RetentionMetrics
			| undefined;
		readonly acceptedInputObservations:
			| import("./umber_wasm.js").AcceptedInputObservationLedger
			| undefined;
		advance?(): import("./umber_wasm.js").AttemptResult;
		compileAttempt(): import("./umber_wasm.js").AttemptResult;
		dispose(): void;
	};
	ProjectSession?: new (
		options: ProjectSessionOptions,
	) => {
		addUserFile(path: string, bytes: Uint8Array): void;
		provideResources(responses: ResourceResponse[]): void;
		cancelPendingPatch(): boolean;
		readonly acceptedInputObservations:
			| import("./umber_wasm.js").AcceptedInputObservationLedger
			| undefined;
		advance?(): import("./umber_wasm.js").AttemptResult;
		compileAttempt(): import("./umber_wasm.js").AttemptResult;
		dispose(): void;
	};
	EditorSession?: new (
		options: EditorSessionOptions,
	) => {
		addUserFile(path: string, bytes: Uint8Array): void;
		provideResources(responses: ResourceResponse[]): void;
		applyPatch(patch: import("./umber_wasm.js").SourcePatch): void;
		renderedSourceLocation(
			page: number,
			event: number,
			unit: number | undefined,
			outputId: string,
			revision: number,
		): import("./umber_wasm.js").RenderedSourceResult | undefined;
		cancelPendingPatch(): boolean;
		cancelStabilization(): boolean;
		readonly status: EditorStatus | undefined;
		readonly revision: number | undefined;
		readonly contentHash: string | undefined;
		readonly acceptedInputObservations:
			| import("./umber_wasm.js").AcceptedInputObservationLedger
			| undefined;
		advance(): EditorAttemptResult;
		stabilizeAttempt(): EditorAttemptResult;
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
	options: SessionOptions | ProjectSessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: ResourceResolver,
	signal?: AbortSignal,
	bindings?: CompilerBindings,
): Promise<CompileOutput | ProjectCompileOutput>;

export class EditorCompileFacade {
	readonly disposed: boolean;
	readonly status: EditorStatus | undefined;
	readonly revision: number | undefined;
	readonly contentHash: string | undefined;
	applyPatch(patch: import("./umber_wasm.js").SourcePatch): void;
	renderedSourceLocation(
		page: number,
		event: number,
		unit: number | undefined,
		outputId: string,
		revision: number,
	): import("./umber_wasm.js").RenderedSourceResult | undefined;
	cancelPendingPatch(): boolean;
	cancelStabilization(): boolean;
	advance(
		signal?: AbortSignal,
		onProgress?: (
			result: Extract<EditorAttemptResult, { kind: "need-resources" }>,
		) => void,
	): Promise<
		| Extract<EditorAttemptResult, { kind: "provisional" | "stable" }>
		| EditorCancellationResult
	>;
	stabilize(
		signal?: AbortSignal,
		onProgress?: (
			result: Extract<EditorAttemptResult, { kind: "need-resources" }>,
		) => void,
	): Promise<
		Extract<EditorAttemptResult, { kind: "stable" }> | EditorCancellationResult
	>;
	dispose(): void;
}

export interface EditorCancellationResult {
	kind: "cancelled";
	phase: "advance" | "stabilization";
	cancelled: true;
	status: EditorStatus | undefined;
}

export function createEditorSession(
	options: EditorSessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: ResourceResolver,
	signal?: AbortSignal,
	bindings?: CompilerBindings,
): Promise<EditorCompileFacade>;
