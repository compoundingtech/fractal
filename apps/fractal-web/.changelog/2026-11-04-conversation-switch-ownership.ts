import type { ChangelogEntrySource } from "../src/changelog/contract.ts";

export default {
  title: "Reliable conversation switching",
  summary:
    "Switching conversations now releases the previous conversation’s live follow promptly so the selected conversation can load without an unavailable error.",
  kind: "fix",
} satisfies ChangelogEntrySource;
