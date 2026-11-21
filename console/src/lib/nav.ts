import { useCallback } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router";
import { lastView } from "./views";

/** "PAY-42" -> "PAY" */
export const projectOfTask = (id: string) => id.slice(0, id.lastIndexOf("-"));

/**
 * Open a task in the side sheet: on a pot route it just sets `?task=`, so the list/board behind it stays put;
 * anywhere else it jumps to that pot's last-used view with the sheet open.
 */
export function useOpenTask() {
  const navigate = useNavigate();
  const loc = useLocation();
  const [, setParams] = useSearchParams();
  return useCallback(
    (id: string) => {
      const pot = projectOfTask(id);
      const m = /^\/pots\/([^/]+)\//.exec(loc.pathname);
      if (m && decodeURIComponent(m[1]!).toUpperCase() === pot.toUpperCase()) {
        setParams(
          (p) => {
            const n = new URLSearchParams(p);
            n.set("task", id);
            return n;
          },
          { replace: false },
        );
      } else {
        void navigate(`/pots/${pot}/${lastView(pot)}?task=${encodeURIComponent(id)}`);
      }
    },
    [loc.pathname, navigate, setParams],
  );
}
