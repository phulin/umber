import type {
	CompileOutput,
	EditorAttemptResult,
	EditorSessionOptions,
	EditorStatus,
	SessionOptions,
	SourcePatch,
} from "./umber_wasm.js";
import type { EditorCancellationResult } from "./compile.js";
import type { HtmlRenderUpdate } from "./html-patch.js";
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
		| "manifestAHash64"
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

export class EditorWorkerFacade {
	readonly disposed: boolean;
	readonly status: EditorStatus | undefined;
	advance(onProgress?: EditorProgressCallback): Promise<
		| (Extract<EditorAttemptResult, { kind: "provisional" | "stable" }> & {
				renderUpdate: HtmlRenderUpdate | null;
		  })
		| EditorCancellationResult
	>;
	stabilize(onProgress?: EditorProgressCallback): Promise<
		| (Extract<EditorAttemptResult, { kind: "stable" }> & {
				renderUpdate: HtmlRenderUpdate | null;
		  })
		| EditorCancellationResult
	>;
	applyPatch(
		patch: SourcePatch,
	): Promise<{ kind: "patched"; status: EditorStatus | undefined }>;
	acknowledgeRenderUpdate(
		revision: number,
		digest: string,
	): Promise<{ kind: "render-acknowledged" }>;
	renderResync(): Promise<{
		kind: "render-resync";
		update: HtmlRenderUpdate | null;
	}>;
	renderedSourceLocation(
		page: number,
		event: number,
		unit: number | undefined,
		outputId: string,
		revision: number,
	): Promise<import("./umber_wasm.js").RenderedSourceResult | undefined>;
	cancelStabilization(): Promise<{
		kind: "cancelled";
		cancelled: boolean;
		status: EditorStatus | undefined;
	}>;
	cancelPendingPatch(): Promise<{
		kind: "cancelled";
		cancelled: boolean;
		status: EditorStatus | undefined;
	}>;
	dispose(): Promise<void>;
	terminate(): void;
}

export type EditorProgressCallback = (
	result: Extract<EditorAttemptResult, { kind: "need-resources" }>,
) => void;

export function createEditorSessionInWorker(
	options: EditorSessionOptions,
	userFiles: ReadonlyMap<string, Uint8Array>,
	resolver: Pick<
		HttpManifestResolverOptions,
		| "manifestUrl"
		| "manifestAHash64"
		| "persistentCache"
		| "offline"
		| "concurrency"
	> & {
		format?: string;
		resourceResponses?: import("./resource-resolver.js").TypedResourceResponse[];
	},
	control?: WorkerCompileControl,
): Promise<EditorWorkerFacade>;
