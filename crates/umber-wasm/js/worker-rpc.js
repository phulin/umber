/** Request-correlated timeout, abort, progress, and teardown for module workers. */
export class WorkerRpcClient {
	#worker;
	#timeoutMs;
	#signal;
	#disposedMessage;
	#pendingMessage;
	#pending = new Map();
	#onOwnerAbort;
	#onMessage;
	#onError;
	#onMessageError;

	constructor(
		worker,
		{
			timeoutMs,
			signal,
			workerFailureMessage = "worker execution failed",
			disposedMessage = "worker has been disposed",
			pendingMessage = "a worker operation is already pending",
		} = {},
	) {
		this.#worker = worker;
		this.#timeoutMs = timeoutMs;
		this.#signal = signal;
		this.#disposedMessage = disposedMessage;
		this.#pendingMessage = pendingMessage;
		this.#onOwnerAbort = () => this.terminate(abortReason(signal));
		this.#onMessage = (event) => this.#receive(event.data);
		this.#onError = (event) =>
			this.terminate(
				new WorkerRpcError("worker", event.message ?? workerFailureMessage, {
					cause: event.error,
				}),
			);
		this.#onMessageError = () =>
			this.terminate(
				new WorkerRpcError(
					"worker-protocol",
					"worker response could not be cloned",
				),
			);
		worker.addEventListener("message", this.#onMessage);
		worker.addEventListener("error", this.#onError);
		worker.addEventListener("messageerror", this.#onMessageError);
		signal?.addEventListener("abort", this.#onOwnerAbort, { once: true });
	}

	get disposed() {
		return this.#worker === undefined;
	}

	request(message, options) {
		if (this.#worker === undefined) {
			return Promise.reject(
				new WorkerRpcError("disposed", this.#disposedMessage),
			);
		}
		if (this.#pending.size > 0 && !options.allowConcurrent) {
			return Promise.reject(
				new WorkerRpcError("worker-protocol", this.#pendingMessage),
			);
		}
		const key = message.id ?? NO_ID;
		if (this.#pending.has(key)) {
			return Promise.reject(
				new WorkerRpcError("worker-protocol", "duplicate worker request id"),
			);
		}

		return new Promise((resolve, reject) => {
			const timer = setTimeout(
				() =>
					this.#finish(
						key,
						reject,
						new WorkerRpcError(
							"timeout",
							`worker exceeded ${this.#timeoutMs} ms`,
						),
						true,
					),
				this.#timeoutMs,
			);
			this.#pending.set(key, { resolve, reject, timer, ...options });
			try {
				this.#worker.postMessage(message, options.transfer ?? []);
			} catch (error) {
				this.#finish(
					key,
					reject,
					new WorkerRpcError(
						"worker-protocol",
						options.cloneError ?? "worker request could not be cloned",
						{ cause: error },
					),
					true,
				);
			}
		});
	}

	terminate(
		reason = new WorkerRpcError("disposed", "worker has been disposed"),
	) {
		if (this.#worker === undefined) return;
		const worker = this.#worker;
		this.#worker = undefined;
		this.#signal?.removeEventListener("abort", this.#onOwnerAbort);
		worker.removeEventListener("message", this.#onMessage);
		worker.removeEventListener("error", this.#onError);
		worker.removeEventListener("messageerror", this.#onMessageError);
		worker.terminate();
		for (const [key, pending] of [...this.#pending]) {
			this.#finish(key, pending.reject, reason);
		}
	}

	#receive(message) {
		const key = message?.id ?? NO_ID;
		const pending = this.#pending.get(key);
		if (pending === undefined) return;
		if (message?.kind === pending.progressKind) {
			pending.onProgress?.(message.result);
			return;
		}
		if (message?.kind === pending.expectedKind) {
			this.#finish(
				key,
				pending.resolve,
				message.result ?? message.output,
				pending.releaseOnSettle,
			);
			return;
		}
		if (message?.kind === pending.errorKind) {
			this.#finish(
				key,
				pending.reject,
				new WorkerRpcError(
					message.error?.code ?? "worker",
					message.error?.message ?? pending.failureMessage,
					{ diagnostic: message.error?.diagnostic },
				),
				pending.releaseOnSettle,
			);
			return;
		}
		if (pending.invalidMessage !== undefined) {
			this.#finish(
				key,
				pending.reject,
				new WorkerRpcError("worker-protocol", pending.invalidMessage),
				pending.releaseOnSettle,
			);
		}
	}

	#finish(key, callback, value, release = false) {
		const pending = this.#pending.get(key);
		if (pending === undefined) return;
		clearTimeout(pending.timer);
		this.#pending.delete(key);
		if (release) this.terminate();
		callback(value);
	}
}

export class WorkerRpcError extends Error {
	constructor(code, message, options = {}) {
		super(message, { cause: options.cause });
		this.name = "WorkerCompileError";
		this.code = code;
		if (options.diagnostic !== undefined) this.diagnostic = options.diagnostic;
	}
}

const NO_ID = Symbol("worker request without an id");

function abortReason(signal) {
	return (
		signal.reason ?? new DOMException("The operation was aborted", "AbortError")
	);
}
