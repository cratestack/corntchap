import type { ReactNode } from "react";
import { Refine, type DataProvider } from "@refinedev/core";
import { StaticSession, type SessionUser } from "@/lib/session";

/** Every procedure answers an empty page: the part of a component that reads more (a station's activity) stays quiet. */
const quiet = { custom: async () => ({ data: { items: [], nextCursor: null, total: 0 } }) } as unknown as DataProvider;

const ALICE: SessionUser = { id: "00000000-0000-0000-0000-000000000001", name: "alice", display_name: "Alice", kind: "user", role: "orchestrator", csrf: null };

/**
 * What a component that takes its rows as a `source` still needs around it in Storybook: Refine with a data provider
 * that answers every read with an empty page, and a signed-in person. The rows themselves come from the story's fixture source.
 */
export function StoryData({ children, user = ALICE }: { children: ReactNode; user?: SessionUser }) {
  return (
    <Refine dataProvider={quiet} options={{ disableTelemetry: true, syncWithLocation: false, warnWhenUnsavedChanges: false }}>
      <StaticSession user={user}>{children}</StaticSession>
    </Refine>
  );
}
