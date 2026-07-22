import { compile } from "./compile.js";
import { HttpManifestResolver } from "./manifest-resolver.js";
import { CompositeResourceResolver } from "./resource-resolver.js";

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
	const fontResources = new Map(
		(message.resolver.fontResources ?? []).map((font) => [
			font.logicalName,
			font,
		]),
	);
	const resolver =
		fontResources.size === 0
			? manifestResolver
			: new CompositeResourceResolver([
					{
						async resolve(requests, options) {
							return requests.concat(options?.probes ?? []).map((request) => {
								if (request.type !== "font")
									return { ...request, type: `${request.type}-unavailable` };
								const resource = fontResources.get(request.logicalName);
								return resource === undefined
									? { ...request, type: "font-unavailable" }
									: { ...request, ...resource, type: "font" };
							});
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
	let started = false;
	scope.addEventListener("message", async (event) => {
		if (started) return;
		started = true;
		try {
			const output = await runCompileMessage(event.data);
			scope.postMessage({ kind: "complete", output }, outputTransfers(output));
		} catch (error) {
			scope.postMessage({ kind: "error", error: serializedError(error) });
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
