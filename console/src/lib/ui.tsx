import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";

interface Ui {
  paletteOpen: boolean;
  setPaletteOpen: (v: boolean) => void;
  menuOpen: boolean;
  setMenuOpen: (v: boolean) => void;
  newTask: { open: boolean; pot?: string | undefined; menu?: string | undefined };
  openNewTask: (pot?: string, menu?: string) => void;
  closeNewTask: () => void;
}

const Ctx = createContext<Ui | null>(null);

export function UiProvider({ children }: { children: ReactNode }) {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [newTask, setNewTask] = useState<Ui["newTask"]>({ open: false });
  const openNewTask = useCallback((pot?: string, menu?: string) => setNewTask({ open: true, pot, menu }), []);
  const closeNewTask = useCallback(() => setNewTask((s) => ({ ...s, open: false })), []);
  const value = useMemo(
    () => ({ paletteOpen, setPaletteOpen, menuOpen, setMenuOpen, newTask, openNewTask, closeNewTask }),
    [paletteOpen, menuOpen, newTask, openNewTask, closeNewTask],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useUi(): Ui {
  const v = useContext(Ctx);
  if (!v) throw new Error("useUi outside UiProvider");
  return v;
}
