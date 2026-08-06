import {
	buildPage,
	captureUserState,
	insertAt,
	restoreUserState,
	stageInsertions,
	updatePage,
} from "./html-patch-dom.js";
import { simulatePatch, validateSnapshot } from "./html-patch-model.js";
import { HtmlResourceRegistry } from "./html-patch-resources.js";
import {
	freshMetrics,
	HtmlPatchError,
	modelPage,
	now,
	required,
	resolveLimits,
} from "./html-patch-shared.js";

export { HtmlResourceRegistry } from "./html-patch-resources.js";
export { HtmlPatchError } from "./html-patch-shared.js";

/** Mounts typed render snapshots and applies schema-1 patches in place. */
export class HtmlPatchMount {
	#root;
	#document;
	#limits;
	#resources;
	#state = null;
	#nodes = new Map();
	#pageContent = new Map();
	#needsResync = false;
	#disposed = false;
	#metrics = freshMetrics();

	constructor(root, options = {}) {
		if (!root || typeof root.replaceChildren !== "function") {
			throw new TypeError("HTML patch root must be a DOM-like element");
		}
		this.#root = root;
		this.#document =
			options.document ?? root.ownerDocument ?? globalThis.document;
		if (!this.#document?.createElement || !this.#document?.createElementNS) {
			throw new TypeError("HTML patch mount requires DOM constructors");
		}
		this.#limits = resolveLimits(options.limits);
		this.#resources =
			options.resources ??
			new HtmlResourceRegistry({
				document: this.#document,
				verify: options.verifyResource,
				FontFace: options.FontFace,
				maxBytes: this.#limits.maxResourceBytes,
			});
	}

	get revision() {
		return this.#state?.revision ?? 0;
	}

	get digest() {
		return this.#state?.digest ?? null;
	}

	get needsResync() {
		return this.#needsResync;
	}

	get metrics() {
		return Object.freeze({
			...this.#metrics,
			resources: this.#resources.metrics,
		});
	}

	/** Initial mount or explicit resynchronization. */
	async mountSnapshot(snapshot) {
		this.#assertLive();
		const state = validateSnapshot(snapshot, this.#limits);
		const retained = new Set(
			state.resources.map((resource) => resource.identity),
		);
		const releases = (this.#state?.resources ?? [])
			.map((resource) => resource.identity)
			.filter((identity) => !retained.has(identity));
		const retainedIdentities = state.resources.map(
			(resource) => resource.identity,
		);
		const nodes = new Map();
		const pageContent = new Map();
		const fragment = this.#document.createDocumentFragment();
		for (const page of state.pages) {
			const built = buildPage(this.#document, page, nodes, pageContent);
			fragment.append(built);
		}
		const lease = await this.#resources.stage(
			state.resources,
			releases,
			retainedIdentities,
		);
		try {
			this.#root.replaceChildren(fragment);
			await lease.commit(releases, retainedIdentities);
		} catch (error) {
			await lease.rollback();
			throw new HtmlPatchError(
				"snapshot-apply",
				"snapshot could not be mounted",
				{
					cause: error,
				},
			);
		}
		this.#nodes = nodes;
		this.#pageContent = pageContent;
		this.#state = state;
		this.#needsResync = false;
		this.#metrics.snapshots += 1;
		return this.acknowledgement();
	}

	async applyPatch(patch) {
		this.#assertLive();
		if (!this.#state)
			throw new HtmlPatchError("missing-base", "mount a snapshot first");
		if (this.#needsResync) {
			throw new HtmlPatchError(
				"resync-required",
				"mount requires a full resynchronization",
			);
		}
		if (
			patch?.sessionId === this.#state.sessionId &&
			patch?.targetRevision === this.#state.revision &&
			patch?.afterDigest === this.#state.digest
		) {
			this.#metrics.duplicates += 1;
			return this.acknowledgement();
		}
		let candidate;
		let staged;
		let lease;
		try {
			candidate = simulatePatch(this.#state, patch, this.#limits);
			const retainedIdentities = candidate.resources.map(
				(resource) => resource.identity,
			);
			staged = stageInsertions(
				this.#document,
				patch.operations,
				candidate.pages,
			);
			lease = await this.#resources.stage(
				patch.resourceAdditions,
				patch.resourceReleases,
				retainedIdentities,
			);
		} catch (error) {
			this.#needsResync = true;
			this.#metrics.resyncs += 1;
			throw error;
		}
		const retainedIdentities = candidate.resources.map(
			(resource) => resource.identity,
		);
		const releases = patch.resourceReleases;
		const preserved = captureUserState(this.#document, this.#root, this.#nodes);
		const started = now();
		try {
			for (const operation of patch.operations) this.#apply(operation, staged);
			restoreUserState(preserved, this.#nodes, this.#root);
			await lease.commit(releases, retainedIdentities);
		} catch (error) {
			await lease.rollback();
			this.#restoreValidatedState();
			this.#needsResync = true;
			this.#metrics.resyncs += 1;
			throw new HtmlPatchError("apply-failed", "patch publication failed", {
				cause: error,
			});
		}
		this.#state = candidate;
		this.#metrics.patches += 1;
		this.#metrics.operations += patch.operations.length;
		this.#metrics.applyMilliseconds += now() - started;
		return this.acknowledgement();
	}

	acknowledgement() {
		this.#assertLive();
		if (!this.#state) return null;
		return Object.freeze({
			kind: "ack",
			schemaVersion: 1,
			sessionId: this.#state.sessionId,
			revision: this.#state.revision,
			digest: this.#state.digest,
		});
	}

	nodeForKey(key) {
		return this.#nodes.get(key) ?? null;
	}

	async dispose() {
		if (this.#disposed) return;
		this.#disposed = true;
		await this.#resources.dispose();
		this.#root.replaceChildren();
		this.#nodes.clear();
		this.#pageContent.clear();
		this.#state = null;
	}

	#assertLive() {
		if (this.#disposed)
			throw new HtmlPatchError("disposed", "HTML patch mount is disposed");
	}

	#restoreValidatedState() {
		const nodes = new Map();
		const pageContent = new Map();
		const fragment = this.#document.createDocumentFragment();
		for (const page of this.#state.pages) {
			fragment.append(buildPage(this.#document, page, nodes, pageContent));
		}
		this.#root.replaceChildren(fragment);
		this.#nodes = nodes;
		this.#pageContent = pageContent;
	}

	#apply(operation, staged) {
		switch (operation.kind) {
			case "remove-node": {
				const node = required(this.#nodes, operation.key);
				node.remove();
				this.#nodes.delete(operation.key);
				this.#metrics.removed += 1;
				break;
			}
			case "remove-page": {
				const model = modelPage(this.#state.pages, operation.key);
				const page = required(this.#nodes, operation.key);
				for (const node of model.nodes) this.#nodes.delete(node.key);
				page.remove();
				this.#nodes.delete(operation.key);
				this.#pageContent.delete(operation.key);
				this.#metrics.removed += model.nodes.length + 1;
				break;
			}
			case "insert-page": {
				const record = staged.get(operation.page.key);
				insertAt(this.#root, record.element, operation.index);
				for (const [key, node] of record.nodes) this.#nodes.set(key, node);
				this.#pageContent.set(operation.page.key, record.content);
				this.#metrics.inserted += operation.page.nodes.length + 1;
				break;
			}
			case "move-page": {
				insertAt(
					this.#root,
					required(this.#nodes, operation.key),
					operation.index,
				);
				this.#metrics.moved += 1;
				break;
			}
			case "insert-node": {
				const content = required(this.#pageContent, operation.page);
				const node = staged.get(operation.node.key).element;
				insertAt(content, node, operation.index);
				this.#nodes.set(operation.node.key, node);
				this.#metrics.inserted += 1;
				break;
			}
			case "move-node": {
				const content = required(this.#pageContent, operation.page);
				insertAt(
					content,
					required(this.#nodes, operation.key),
					operation.index,
				);
				this.#metrics.moved += 1;
				break;
			}
			case "update-page":
				updatePage(
					required(this.#nodes, operation.page.key),
					operation.page,
					required(this.#pageContent, operation.page.key),
				);
				this.#metrics.updated += 1;
				break;
			case "update-node": {
				const old = required(this.#nodes, operation.node.key);
				const replacement = staged.get(operation.node.key).element;
				old.replaceWith(replacement);
				this.#nodes.set(operation.node.key, replacement);
				this.#metrics.updated += 1;
				break;
			}
			default:
				throw new HtmlPatchError(
					"unknown-operation",
					"unknown patch operation",
				);
		}
	}
}
