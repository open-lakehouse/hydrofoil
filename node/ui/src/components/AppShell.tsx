import { Link, Outlet } from "@tanstack/react-router";
import { Database, FileCode } from "lucide-react";
import { SERVICE_SURFACES } from "@/lib/services";
import { cn } from "@/lib/utils";

const navLinkClass =
  "flex items-center gap-2 rounded px-2 py-1.5 text-sm font-medium hover:bg-accent hover:text-accent-foreground";
const activeProps = {
  className: cn(navLinkClass, "bg-accent text-accent-foreground"),
};

// The inner application: sidebar nav + routed content. The persistent header
// (Open Lakehouse + theme toggle) is owned by EnvironmentGate, which renders this
// only once an environment is active.
export function AppShell() {
  return (
    <div className="flex flex-1">
      <nav className="hidden w-56 shrink-0 border-r bg-sidebar p-4 lg:block">
        <div className="mb-2 px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Catalog
        </div>
        <ul className="mb-6 space-y-1">
          <li>
            <Link
              to="/catalog"
              className={navLinkClass}
              activeProps={activeProps}
            >
              <Database className="h-4 w-4" />
              Unity Catalog
            </Link>
          </li>
          <li>
            <Link
              to="/editor"
              className={navLinkClass}
              activeProps={activeProps}
            >
              <FileCode className="h-4 w-4" />
              Editor
            </Link>
          </li>
        </ul>

        <div className="mb-2 px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Services
        </div>
        <ul className="space-y-1">
          {SERVICE_SURFACES.map((service) => {
            const Icon = service.icon;
            return (
              <li key={service.id}>
                <Link
                  to="/services/$serviceId"
                  params={{ serviceId: service.id }}
                  className={navLinkClass}
                  activeProps={activeProps}
                >
                  <Icon className="h-4 w-4" />
                  {service.label}
                </Link>
              </li>
            );
          })}
        </ul>
      </nav>
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  );
}
