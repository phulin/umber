import {
	SessionDriverError as CompileFacadeError,
	SessionDriver,
} from "./session-driver.js";

const DEFAULT_LIMITS = Object.freeze({
	attempts: 32,
	userFiles: 512,
	resolvedFiles: 512,
	oneFileBytes: 96 * 1024 * 1024,
	cachedFileBytes: 64 * 1024 * 1024,
	userSourceBytes: 16 * 1024 * 1024,
	outputBytes: 64 * 1024 * 1024,
	engineFuel: 100_000_000,
	engineSteps: 10_000_000,
	inputFrames: 100_000,
	journalBytes: 256 * 1024 * 1024,
	effects: 1_000_000,
});

const HARD_LIMITS = Object.freeze({
	attempts: 128,
	userFiles: 4096,
	resolvedFiles: 4096,
	oneFileBytes: 128 * 1024 * 1024,
	cachedFileBytes: 256 * 1024 * 1024,
	userSourceBytes: 64 * 1024 * 1024,
	outputBytes: 256 * 1024 * 1024,
	engineFuel: 100_000_000_000,
	engineSteps: 100_000_000,
	inputFrames: 1_000_000,
	journalBytes: 1024 * 1024 * 1024,
	effects: 10_000_000,
});

export { CompileFacadeError };

export async function compile(options, userFiles, resolver, signal, bindings) {
	validateResolver(resolver);
	const limits = validateSessionLimits(options?.limits);
	throwIfAborted(signal);
	const Session = await sessionClass(
		bindings,
		options?.bibliography !== undefined,
	);
	throwIfAborted(signal);
	const session = new Session(options);
	const driver = new SessionDriver(session, resolver);
	try {
		addUserFiles(session, userFiles, limits);
		const result = await driver.drive({
			phase: "compile",
			attempt: (current) =>
				typeof current.advance === "function"
					? current.advance()
					: current.compileAttempt(),
			isComplete: (attempt) => attempt?.kind === "complete",
			signal,
			attemptLimit: limits.attempts,
		});
		return result.output;
	} finally {
		driver.dispose();
	}
}

/** Creates a retained editor session whose hot pass and stabilization are explicit. */
export async function createEditorSession(
	options,
	userFiles,
	resolver,
	signal,
	bindings,
) {
	validateResolver(resolver);
	const limits = validateSessionLimits(options?.limits);
	throwIfAborted(signal);
	const module = bindings ?? (await import("./umber_wasm.js"));
	if (bindings === undefined) await module.default();
	if (typeof module?.EditorSession !== "function") {
		throw new CompileFacadeError(
			"invalid-binding",
			"EditorSession binding is unavailable",
		);
	}
	throwIfAborted(signal);
	const session = new module.EditorSession(options);
	try {
		addUserFiles(session, userFiles, limits);
		return new EditorCompileFacade(session, resolver);
	} catch (error) {
		session.dispose();
		throw error;
	}
}

export class EditorCompileFacade {
	#session;
	#driver;

	constructor(session, resolver) {
		this.#session = session;
		this.#driver = new SessionDriver(session, resolver);
	}

	get disposed() {
		return this.#driver.disposed;
	}

	get status() {
		return this.#requireSession().status;
	}

	get revision() {
		return this.#requireSession().revision;
	}

	get contentHash() {
		return this.#requireSession().contentHash;
	}

	get renderUpdate() {
		return this.#requireSession().renderUpdate?.() ?? null;
	}

	acknowledgeRenderUpdate(revision, digest) {
		this.#requireSession().acknowledgeRenderUpdate(revision, digest);
	}

	renderResync() {
		return this.#requireSession().renderResync?.() ?? null;
	}

	applyPatch(patch) {
		this.#requireSession().applyPatch(patch);
	}

	renderedSourceLocation(page, event, unit, outputId, revision) {
		return this.#requireSession().renderedSourceLocation(
			page,
			event,
			unit,
			outputId,
			revision,
		);
	}

	cancelPendingPatch() {
		const cancelled = this.#requireSession().cancelPendingPatch();
		if (cancelled) this.#driver.cancel("advance");
		return cancelled;
	}

	cancelStabilization() {
		const cancelled = this.#requireSession().cancelStabilization();
		if (cancelled) this.#driver.cancel("stabilization");
		return cancelled;
	}

	async advance(signal, onProgress) {
		return this.#drive("advance", signal, onProgress);
	}

	async stabilize(signal, onProgress) {
		return this.#drive("stabilization", signal, onProgress);
	}

	dispose() {
		if (this.#session === undefined) return;
		this.#driver.dispose();
		this.#session = undefined;
	}

	async #drive(phase, signal, onProgress) {
		this.#requireSession();
		return this.#driver.drive({
			phase,
			attempt: (session) =>
				phase === "advance" ? session.advance() : session.stabilizeAttempt(),
			isComplete: (attempt) =>
				attempt?.kind === "provisional" || attempt?.kind === "stable",
			signal,
			onProgress,
			pendingMessage: "an editor operation is already pending",
		});
	}

	#requireSession() {
		if (this.#driver.disposed) {
			throw new CompileFacadeError(
				"disposed",
				"editor session has been disposed",
			);
		}
		return this.#session;
	}
}

async function sessionClass(bindings, project) {
	const module = bindings ?? (await import("./umber_wasm.js"));
	if (bindings === undefined) await module.default();
	const Session = project ? module?.ProjectSession : module?.CompilerSession;
	if (typeof Session !== "function") {
		throw new CompileFacadeError(
			"invalid-binding",
			`${project ? "ProjectSession" : "CompilerSession"} binding is unavailable`,
		);
	}
	return Session;
}

function addUserFiles(session, userFiles, limits) {
	if (!userFiles || typeof userFiles[Symbol.iterator] !== "function") {
		throw new CompileFacadeError(
			"invalid-options",
			"userFiles must be an iterable map",
		);
	}
	let total = 0;
	let count = 0;
	for (const item of userFiles) {
		if (
			!Array.isArray(item) ||
			item.length !== 2 ||
			typeof item[0] !== "string"
		) {
			throw new CompileFacadeError(
				"invalid-options",
				"userFiles entries must be [path, Uint8Array]",
			);
		}
		const [path, bytes] = item;
		count += 1;
		if (count > limits.userFiles) {
			throw limitError("user files", limits.userFiles, count);
		}
		requireBytes(bytes, `user file ${path}`);
		if (bytes.byteLength > limits.oneFileBytes) {
			throw limitError(
				"one user file bytes",
				limits.oneFileBytes,
				bytes.byteLength,
			);
		}
		total = checkedAdd(total, bytes.byteLength);
		if (total > limits.userSourceBytes) {
			throw limitError("user source bytes", limits.userSourceBytes, total);
		}
		session.addUserFile(path, bytes);
	}
}

function validateResolver(resolver) {
	if (!resolver || typeof resolver.resolve !== "function") {
		throw new CompileFacadeError(
			"invalid-options",
			"resolver.resolve is required",
		);
	}
}

export function validateSessionLimits(partial = {}) {
	const limits = { ...DEFAULT_LIMITS, ...(partial ?? {}) };
	for (const [name, hard] of Object.entries(HARD_LIMITS)) {
		const value = limits[name];
		if (!Number.isSafeInteger(value) || value < 0 || value > hard) {
			throw new CompileFacadeError(
				"invalid-options",
				`${name} must be an integer from 0 through ${hard}`,
			);
		}
	}
	return limits;
}

function requireBytes(value, label) {
	if (!(value instanceof Uint8Array)) {
		throw new CompileFacadeError(
			"invalid-options",
			`${label} must be a Uint8Array`,
		);
	}
}

function checkedAdd(left, right) {
	const total = left + right;
	if (!Number.isSafeInteger(total)) {
		throw new CompileFacadeError(
			"limit",
			"byte accounting exceeded JavaScript safe integers",
		);
	}
	return total;
}

function limitError(resource, limit, attempted) {
	return new CompileFacadeError(
		"limit",
		`${resource} requires ${attempted}, exceeding limit ${limit}`,
	);
}

function throwIfAborted(signal) {
	if (signal?.aborted) throw abortReason(signal);
}

function abortReason(signal) {
	return (
		signal.reason ?? new DOMException("The operation was aborted", "AbortError")
	);
}
