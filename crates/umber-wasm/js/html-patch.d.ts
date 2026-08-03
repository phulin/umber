export interface HtmlPatchAcknowledgement {
	readonly kind: "ack";
	readonly schemaVersion: 1;
	readonly sessionId: string;
	readonly revision: number;
	readonly digest: string;
}

export interface HtmlPatchMountOptions {
	document?: Document;
	limits?: Partial<{
		maxPages: number;
		maxNodes: number;
		maxOperations: number;
		maxStringBytes: number;
		maxResourceBytes: number;
	}>;
	resources?: HtmlResourceRegistry;
	verifyResource?: (
		identity: string,
		bytes: Uint8Array,
	) => boolean | Promise<boolean>;
	FontFace?: typeof FontFace;
}

export class HtmlPatchError extends Error {
	readonly code: string;
}

export class HtmlPatchMount {
	constructor(root: HTMLElement, options?: HtmlPatchMountOptions);
	readonly revision: number;
	readonly digest: string | null;
	readonly needsResync: boolean;
	readonly metrics: Readonly<Record<string, unknown>>;
	mountSnapshot(snapshot: unknown): Promise<HtmlPatchAcknowledgement>;
	applyPatch(patch: unknown): Promise<HtmlPatchAcknowledgement>;
	acknowledgement(): HtmlPatchAcknowledgement | null;
	nodeForKey(key: string): Node | null;
	dispose(): Promise<void>;
}

export class HtmlResourceRegistry {
	constructor(options?: {
		document?: Document;
		verify?: (
			identity: string,
			bytes: Uint8Array,
		) => boolean | Promise<boolean>;
		FontFace?: typeof FontFace;
		maxBytes?: number;
	});
	readonly metrics: Readonly<{ count: number; bytes: number }>;
	stage(additions: readonly unknown[]): Promise<{
		commit(
			releases: readonly string[],
			retained: readonly string[],
		): Promise<void>;
		rollback(): Promise<void>;
	}>;
	dispose(): Promise<void>;
}
