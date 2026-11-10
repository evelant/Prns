import { nativeSources } from "./documents";
import type { NoticeDocument } from "./documents";

export const documents: readonly NoticeDocument[] = [
  { title: "JavaScript libraries", asset: () => require("../../assets/licenses/npm-ios.txt") },
  nativeSources,
  { title: "iOS libraries", bundleFile: "prns-cocoapods-notices.txt" },
];
