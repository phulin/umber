export interface ComputerModernFontResource {
	logicalName: string;
	container: "woff2";
	bytes: Uint8Array;
	objectSha256: string;
	provenance: string;
	legacyMapping: {
		tfmSha256: string;
		encoding: Array<string | null>;
		embeddable: true;
	};
}

export function ot1TextEncoding(): Array<string | null>;
export function loadComputerModernTextFont(
	name: string,
	tfmContentHash: string,
	options?: {
		fetchImpl?: typeof fetch;
		encoding?: Array<string | null>;
	},
): Promise<ComputerModernFontResource>;
