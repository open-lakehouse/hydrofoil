import { Link } from "@tanstack/react-router";
import { ThemeToggle } from "./ThemeToggle";

export function TopBar() {
  return (
    <header className="sticky top-0 z-50 flex h-12 items-center justify-between border-b bg-background/80 px-4 backdrop-blur-sm">
      <Link to="/" className="text-sm font-semibold tracking-tight">
        Open Lakehouse
      </Link>
      <ThemeToggle />
    </header>
  );
}
