import { compile, createEditorSession } from "./compile.js";
import { HttpManifestResolver } from "./manifest-resolver.js";
import {
	CompositeResourceResolver,
	resourceRequestIdentity,
	resourceResponseIdentity,
} from "./resource-resolver.js";

export async function runCompileMessage(message, dependencies = {}) {
	if (message?.kind !== "compile" || !Array.isArray(message.userFiles)) {
		throw new Error("invalid compile worker request");
	}
	const bindings = dependencies.bindings ?? (await import("./umber_wasm.js"));
	if (dependencies.bindings === undefined) {
		const initialize = bindings.default;
		await initialize(
			message.wasmUrl === undefined
				? undefined
				: { module_or_path: message.wasmUrl },
		);
	}
	const manifestResolver =
		dependencies.resolver ??
		(await (dependencies.createResolver ?? HttpManifestResolver.create)({
			...message.resolver,
			maxFiles: message.options?.limits?.resolvedFiles,
			maxBytes: message.options?.limits?.cachedFileBytes,
		}));
	const resourceResponses = new Map(
		(message.resolver.resourceResponses ?? []).map((response) => [
			resourceResponseIdentity(response),
			response,
		]),
	);
	const resolver =
		resourceResponses.size === 0
			? manifestResolver
			: new CompositeResourceResolver([
					{
						async resolve(requests, options) {
							return requests.concat(options?.probes ?? []).map(
								(request) =>
									resourceResponses.get(resourceRequestIdentity(request)) ?? {
										...request,
										type: `${request.type}-unavailable`,
									},
							);
						},
					},
					manifestResolver,
				]);
	let options = message.options;
	if (message.resolver.format !== undefined) {
		const format = await manifestResolver.resolveFormat(
			message.resolver.format,
			{
				engineVersion: bindings.packageVersion(),
				formatSchema: bindings.formatSchemaVersion(),
			},
		);
		const formatPrefetchHints =
			manifestResolver.formatPrefetchHints?.(message.resolver.format) ?? [];
		options = { ...options, format, formatPrefetchHints };
	}
	return compile(
		options,
		new Map(message.userFiles),
		resolver,
		undefined,
		bindings,
	);
}

export async function createEditorFromMessage(message, dependencies = {}) {
	if (message?.kind !== "editor-create" || !Array.isArray(message.userFiles)) {
		throw new Error("invalid editor worker request");
	}
	const bindings = dependencies.bindings ?? (await import("./umber_wasm.js"));
	if (dependencies.bindings === undefined) {
		const initialize = bindings.default;
		await initialize(
			message.wasmUrl === undefined
				? undefined
				: { module_or_path: message.wasmUrl },
		);
	}
	const manifestResolver =
		dependencies.resolver ??
		(await (dependencies.createResolver ?? HttpManifestResolver.create)({
			...message.resolver,
			maxFiles: message.options?.limits?.resolvedFiles,
			maxBytes: message.options?.limits?.cachedFileBytes,
		}));
	const resolver = composeResolver(message, manifestResolver);
	let options = message.options;
	if (message.resolver.format !== undefined) {
		const format = await manifestResolver.resolveFormat(
			message.resolver.format,
			{
				engineVersion: bindings.packageVersion(),
				formatSchema: bindings.formatSchemaVersion(),
			},
		);
		const formatPrefetchHints =
			manifestResolver.formatPrefetchHints?.(message.resolver.format) ?? [];
		options = { ...options, format, formatPrefetchHints };
	}
	return createEditorSession(
		options,
		new Map(message.userFiles),
		resolver,
		undefined,
		bindings,
	);
}

function composeResolver(message, manifestResolver) {
	const resourceResponses = new Map(
		(message.resolver.resourceResponses ?? []).map((response) => [
			resourceResponseIdentity(response),
			response,
		]),
	);
	return resourceResponses.size === 0
		? manifestResolver
		: new CompositeResourceResolver([
				{
					async resolve(requests, options) {
						return requests.concat(options?.probes ?? []).map(
							(request) =>
								resourceResponses.get(resourceRequestIdentity(request)) ?? {
									...request,
									type: `${request.type}-unavailable`,
								},
						);
					},
				},
				manifestResolver,
			]);
}

export function outputTransfers(output) {
	if (output.tex) {
		return [
			...new Set([
				...outputTransfers(output.tex),
				...(output.bibliography?.files ?? []).map((file) => file.bytes.buffer),
				...output.generatedFiles.map((file) => file.bytes.buffer),
			]),
		];
	}
	const transfers = [output.log.buffer, output.dvi.buffer];
	if (output.html) transfers.push(output.html.buffer);
	for (const file of output.htmlAssets ?? []) transfers.push(file.bytes.buffer);
	for (const file of output.files) transfers.push(file.bytes.buffer);
	return [...new Set(transfers)];
}

const scope = globalThis;
const isWorkerRealm =
	typeof WorkerGlobalScope !== "undefined" &&
	scope instanceof WorkerGlobalScope;
if (isWorkerRealm) {
	let mode;
	let editor;
	scope.addEventListener("message", async (event) => {
		try {
			if (mode === undefined && event.data?.kind === "compile") {
				mode = "compile";
				const output = await runCompileMessage(event.data);
				scope.postMessage(
					{ kind: "complete", output },
					outputTransfers(output),
				);
				return;
			}
			if (mode === undefined && event.data?.kind === "editor-create") {
				mode = "editor";
				editor = await createEditorFromMessage(event.data);
				scope.postMessage({ kind: "editor-ready", id: event.data.id });
				return;
			}
			if (mode !== "editor" || event.data?.id === undefined) return;
			let result;
			const progress = (value) =>
				scope.postMessage({
					kind: "editor-progress",
					id: event.data.id,
					result: value,
				});
			switch (event.data.kind) {
				case "editor-advance":
					result = await editor.advance(undefined, progress);
					break;
				case "editor-stabilize":
					result = await editor.stabilize(undefined, progress);
					break;
				case "editor-apply-patch":
					editor.applyPatch(event.data.patch);
					result = { kind: "patched", status: editor.status };
					break;
				case "editor-rendered-source":
					result = {
						kind: "rendered-source",
						location: editor.renderedSourceLocation(
							event.data.page,
							event.data.event,
							event.data.unit,
							event.data.outputId,
							event.data.revision,
						),
					};
					break;
				case "editor-cancel-stabilization":
					result = {
						kind: "cancelled",
						cancelled: editor.cancelStabilization(),
						status: editor.status,
					};
					break;
				case "editor-cancel-pending-patch":
					result = {
						kind: "cancelled",
						cancelled: editor.cancelPendingPatch(),
						status: editor.status,
					};
					break;
				case "editor-dispose":
					editor.dispose();
					editor = undefined;
					result = { kind: "disposed" };
					break;
				default:
					throw new Error("invalid editor worker operation");
			}
			const transfers = result.output ? outputTransfers(result.output) : [];
			scope.postMessage(
				{ kind: "editor-result", id: event.data.id, result },
				transfers,
			);
		} catch (error) {
			scope.postMessage({
				kind: mode === "editor" ? "editor-error" : "error",
				id: event.data?.id,
				error: serializedError(error),
			});
		}
	});
}

function serializedError(error) {
	return {
		name: error instanceof Error ? error.name : "Error",
		code: error?.code,
		message: error instanceof Error ? error.message : String(error),
		diagnostic: error?.diagnostic,
		stack: error instanceof Error ? error.stack : undefined,
	};
}
