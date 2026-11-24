import { Link } from "react-router";
import { ChefHat, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSession } from "@/lib/session";
import { WaxBand } from "./brand";
import { CopyButton } from "./secret-dialog";

/**
 * What a person who is on no team yet sees where a pot would be (Board, Pass, Questions, the home page):
 * their handle to give to an admin, a way to hire a cook meanwhile and, for admins, a way to start a pot.
 */
export function NoPot({ title = "You are not in a pot yet" }: { title?: string }) {
  const me = useSession();
  return (
    <div className="mx-auto max-w-xl px-4 py-10 sm:px-5 sm:py-16">
      <div className="overflow-hidden rounded-2xl border border-border bg-card">
        <WaxBand height={52} />
        <div className="space-y-5 px-5 py-7 text-center sm:px-8">
          <div>
            <h1 className="text-[22px] leading-tight sm:text-2xl">{me.is_admin ? "No pot yet" : title}</h1>
            <p className="mx-auto mt-1.5 max-w-md text-[14px] text-muted-foreground">
              {me.is_admin
                ? "Start the first pot, then tasks can go in and cooks can scoop them."
                : "Pots belong to teams. Ask an admin to add you to a team, and give them your handle. Once you are in, you can add your own cooks to it yourself."}
            </p>
          </div>
          <div className="mx-auto flex max-w-sm min-w-0 flex-col items-stretch gap-2 rounded-xl border border-border bg-muted/50 p-3 sm:flex-row sm:items-center">
            <div className="min-w-0 flex-1 text-left">
              <p className="label-caps">Your handle</p>
              <p className="truncate font-mono text-[15px] font-bold" title={`@${me.name}`}>
                {me.name}
              </p>
            </div>
            <CopyButton text={me.name} label="Copy handle" className="max-sm:h-11" />
          </div>
          <div className="flex flex-col items-stretch justify-center gap-2 sm:flex-row">
            {me.is_admin && (
              <Button asChild className="max-sm:h-11">
                <Link to="/admin/pots?new=1">
                  <Plus /> Start a pot
                </Link>
              </Button>
            )}
            <Button asChild variant={me.is_admin ? "outline" : "default"} className="max-sm:h-11">
              <Link to="/cooks">
                <ChefHat /> {me.is_admin ? "Hire a cook" : "Hire a cook meanwhile"}
              </Link>
            </Button>
          </div>
          {!me.is_admin && <p className="text-[12.5px] text-muted-foreground">A cook is an agent you run yourself. It works on its own until you join a team; then you can add it to your team.</p>}
        </div>
      </div>
    </div>
  );
}
