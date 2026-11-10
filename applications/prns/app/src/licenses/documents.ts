export type NoticeDocument = { title: string } & (
  { asset: () => number } | { bundleFile: string }
);

export const nativeSources: NoticeDocument = {
  title: "Native libraries and corresponding source",
  asset: () => require("../../assets/licenses/native-sources.txt"),
};

export const documents: readonly NoticeDocument[] = [nativeSources];
