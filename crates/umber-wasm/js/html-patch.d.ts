export interface HtmlPatchAcknowledgement {
	readonly kind: "ack";
	readonly schemaVersion: 1;
	readonly sessionId: string;
	readonly revision: number;
	readonly digest: string;
}

export interface HtmlRenderResource {
	readonly identity: string;
	readonly kind: "font";
	readonly family: string;
	readonly bytes: Uint8Array;
	readonly provenance: string;
}

export interface HtmlRenderNode {
	readonly key: string;
	readonly kind:
		| "box"
		| "rule"
		| "text"
		| "special"
		| "math-start"
		| "math-glyph"
		| "math-rule"
		| "math-end";
	readonly xSp?: number;
	readonly ySp?: number;
	readonly widthSp?: number;
	readonly heightSp?: number;
	readonly baselineSp?: number;
	readonly depthSp?: number;
	readonly text?: string;
	readonly family?: string;
	readonly fontSizeSp?: number;
	readonly positionsSp?: readonly number[];
	readonly direction?: "ltr" | "rtl";
	readonly script?: string;
	readonly language?: string;
	readonly features?: readonly {
		readonly tag: string;
		readonly value: number;
	}[];
	readonly variations?: readonly {
		readonly tag: string;
		readonly value: number;
	}[];
	readonly accessibilityLine?: number;
	readonly boxId?: number;
	readonly boxKind?: "hbox" | "vbox";
	readonly glyphId?: number;
	readonly ssty?: number;
	readonly fontInstance?: string;
	readonly drawing?: "text" | "outline";
	readonly path?: string;
	readonly unitsPerEm?: number;
	readonly mathId?: number;
	readonly color?: string;
	readonly link?: string;
	readonly class?: string;
	readonly payloadHex?: string;
	readonly action?:
		| "color-push"
		| "color-pop"
		| "link-start"
		| "link-end"
		| "destination"
		| "inert";
	readonly actionValue?: string;
}

export interface HtmlRenderPage {
	readonly key: string;
	readonly ordinal: number;
	readonly widthSp: number;
	readonly heightSp: number;
	readonly originXSp: number;
	readonly originYSp: number;
	readonly mag: number;
	readonly nodes: readonly HtmlRenderNode[];
}

export interface HtmlRenderSnapshot {
	readonly kind: "snapshot";
	readonly schemaVersion: 1;
	readonly sessionId: string;
	readonly revision: number;
	readonly digest: string;
	readonly title: string;
	readonly language: string;
	readonly resources: readonly HtmlRenderResource[];
	readonly pages: readonly HtmlRenderPage[];
}

export type HtmlPatchOperation =
	| {
			readonly kind: "remove-node";
			readonly page: string;
			readonly key: string;
	  }
	| { readonly kind: "remove-page"; readonly key: string }
	| {
			readonly kind: "insert-page";
			readonly index: number;
			readonly page: HtmlRenderPage;
	  }
	| { readonly kind: "move-page"; readonly key: string; readonly index: number }
	| {
			readonly kind: "insert-node";
			readonly page: string;
			readonly index: number;
			readonly node: HtmlRenderNode;
	  }
	| {
			readonly kind: "move-node";
			readonly page: string;
			readonly key: string;
			readonly index: number;
	  }
	| {
			readonly kind: "update-page";
			readonly page: Omit<HtmlRenderPage, "nodes">;
	  }
	| {
			readonly kind: "update-node";
			readonly page: string;
			readonly node: HtmlRenderNode;
	  };

export interface HtmlRenderPatch {
	readonly kind: "patch";
	readonly schemaVersion: 1;
	readonly sessionId: string;
	readonly baseRevision: number;
	readonly targetRevision: number;
	readonly beforeDigest: string;
	readonly afterDigest: string;
	readonly title?: string;
	readonly language?: string;
	readonly resourceAdditions: readonly HtmlRenderResource[];
	readonly resourceReleases: readonly string[];
	readonly operations: readonly HtmlPatchOperation[];
}

export type HtmlRenderUpdate = HtmlRenderSnapshot | HtmlRenderPatch;

export interface HtmlPatchMountOptions {
	document?: Document;
	limits?: Partial<{
		maxPages: number;
		maxNodes: number;
		maxOperations: number;
		maxResources: number;
		maxStrings: number;
		maxStringBytes: number;
		maxTotalStringBytes: number;
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
	mountSnapshot(
		snapshot: HtmlRenderSnapshot,
	): Promise<HtmlPatchAcknowledgement>;
	applyPatch(patch: HtmlRenderPatch): Promise<HtmlPatchAcknowledgement>;
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
		maxResourceBytes?: number;
		maxChurnBytes?: number;
	});
	readonly metrics: Readonly<{
		count: number;
		bytes: number;
		churnBytes: number;
	}>;
	stage(additions: readonly HtmlRenderResource[]): Promise<{
		commit(
			releases: readonly string[],
			retained: readonly string[],
		): Promise<void>;
		rollback(): Promise<void>;
	}>;
	dispose(): Promise<void>;
}
