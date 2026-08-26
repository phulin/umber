import { validateResource } from "./html-patch-model.js";
import {
	DEFAULT_LIMITS,
	HtmlPatchError,
	sameBytes,
	verifyAhash64,
} from "./html-patch-shared.js";

/** Content-addressed FontFace/object ownership behind an acknowledgement barrier. */
export class HtmlResourceRegistry {
	#document;
	#verify;
	#FontFace;
	#maxBytes;
	#maxResourceBytes;
	#maxChurnBytes;
	#churnBytes = 0;
	#entries = new Map();
	#disposed = false;

	constructor(options = {}) {
		this.#document = options.document ?? globalThis.document;
		this.#verify = options.verify ?? verifyAhash64;
		this.#FontFace = options.FontFace ?? globalThis.FontFace;
		this.#maxBytes = options.maxBytes ?? DEFAULT_LIMITS.maxResourceBytes;
		this.#maxResourceBytes = options.maxResourceBytes ?? 64 * 1024 * 1024;
		this.#maxChurnBytes = options.maxChurnBytes ?? 1024 * 1024 * 1024;
	}

	get metrics() {
		let bytes = 0;
		for (const entry of this.#entries.values()) bytes += entry.bytes.byteLength;
		return Object.freeze({
			count: this.#entries.size,
			bytes,
			churnBytes: this.#churnBytes,
		});
	}

	async stage(additions, releases = [], retained = []) {
		if (this.#disposed)
			throw new HtmlPatchError("disposed", "resource registry is disposed");
		validateReleaseSet(this.#entries, releases, retained);
		let projected = this.metrics.bytes;
		const staged = new Map();
		for (const resource of additions) {
			validateResource(resource);
			if (resource.bytes.byteLength > this.#maxResourceBytes) {
				throw new HtmlPatchError(
					"resource-size",
					"individual resource budget exceeded",
				);
			}
			const existing =
				this.#entries.get(resource.identity) ?? staged.get(resource.identity);
			if (existing) {
				if (!sameBytes(existing.bytes, resource.bytes)) {
					throw new HtmlPatchError(
						"resource-conflict",
						"resource identity has conflicting bytes",
					);
				}
				continue;
			}
			projected += resource.bytes.byteLength;
			if (projected > this.#maxBytes) {
				throw new HtmlPatchError(
					"resource-budget",
					"resident resource budget exceeded",
				);
			}
			if (!(await this.#verify(resource.identity, resource.bytes))) {
				throw new HtmlPatchError(
					"resource-digest",
					"resource digest does not match bytes",
				);
			}
			if (this.#churnBytes + resource.bytes.byteLength > this.#maxChurnBytes) {
				throw new HtmlPatchError(
					"resource-churn",
					"cumulative resource churn exceeded",
				);
			}
			this.#churnBytes += resource.bytes.byteLength;
			const entry = { ...resource, bytes: resource.bytes.slice(), face: null };
			if (resource.kind === "font" && this.#FontFace) {
				entry.face = new this.#FontFace(
					resource.family,
					entry.bytes,
					resource.descriptors ?? {},
				);
				await entry.face.load();
			}
			staged.set(resource.identity, entry);
		}
		let settled = false;
		return {
			commit: async (releases, retained) => {
				if (settled)
					throw new HtmlPatchError(
						"resource-lease",
						"resource lease already settled",
					);
				validateReleaseSet(this.#entries, releases, retained);
				settled = true;
				const installed = [];
				try {
					for (const entry of staged.values()) {
						this.#entries.set(entry.identity, entry);
						if (entry.face) this.#document?.fonts?.add(entry.face);
						installed.push(entry.identity);
					}
					for (const identity of releases) this.#remove(identity);
				} catch (error) {
					for (const identity of installed.reverse()) this.#remove(identity);
					throw error;
				}
			},
			rollback: async () => {
				settled = true;
			},
		};
	}

	async dispose() {
		if (this.#disposed) return;
		this.#disposed = true;
		for (const identity of [...this.#entries.keys()]) this.#remove(identity);
	}

	#remove(identity) {
		const entry = this.#entries.get(identity);
		if (!entry)
			throw new HtmlPatchError(
				"unknown-resource",
				"cannot release an unknown resource",
			);
		if (entry.face) this.#document?.fonts?.delete(entry.face);
		this.#entries.delete(identity);
	}
}

function validateReleaseSet(entries, releases, retained) {
	const live = new Set(retained);
	const uniqueReleases = new Set(releases);
	if (uniqueReleases.size !== releases.length) {
		throw new HtmlPatchError(
			"duplicate-release",
			"resource released more than once",
		);
	}
	for (const identity of releases) {
		if (live.has(identity)) {
			throw new HtmlPatchError(
				"live-resource-release",
				"cannot release a live resource",
			);
		}
		if (!entries.has(identity)) {
			throw new HtmlPatchError(
				"unknown-resource",
				"cannot release an unknown resource",
			);
		}
	}
}
