import { nativeSources } from "./documents";
import type { NoticeDocument } from "./documents";

export const documents: readonly NoticeDocument[] = [
  { title: "JavaScript libraries", asset: () => require("../../assets/licenses/npm-android.txt") },
  nativeSources,
  { title: "Android libraries", asset: () => require("../../assets/licenses/android-maven.txt") },
];
