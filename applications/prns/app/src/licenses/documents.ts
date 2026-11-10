export interface NoticeDocument {
  title: string;
  asset: () => number;
}

export const nativeSources: NoticeDocument = {
  title: "Native libraries and corresponding source",
  asset: () => require("../../assets/licenses/native-sources.txt"),
};

export const documents: readonly NoticeDocument[] = [nativeSources];
