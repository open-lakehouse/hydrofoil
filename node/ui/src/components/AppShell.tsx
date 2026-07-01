import {
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Link, Outlet } from "@tanstack/react-router";
import {
  Database,
  FileCode,
  type LucideIcon,
  PanelLeftClose,
  PanelLeftOpen,
  Upload,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { ingestSupported } from "@/lib/ingest/registry";
import { SERVICE_SURFACES } from "@/lib/services";

// Collapsed state is an app-wide UI preference (not per-environment), so it
// persists in localStorage rather than the env-namespaced session storage.
const STORAGE_KEY = "nav.collapsed";

function loadCollapsed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function persistCollapsed(collapsed: boolean) {
  try {
    window.localStorage.setItem(STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    // storage may be unavailable (private mode etc.)
  }
}

const navLinkBase =
  "flex items-center rounded text-sm font-medium hover:bg-accent hover:text-accent-foreground";

// A single nav link. When collapsed it shrinks to an icon-only target and wraps
// in a tooltip so the label stays discoverable.
function NavItem({
  to,
  params,
  icon: Icon,
  label,
  collapsed,
}: {
  to: string;
  params?: Record<string, string>;
  icon: LucideIcon;
  label: string;
  collapsed: boolean;
}) {
  const link = (
    <Link
      to={to}
      params={params}
      className={cn(
        navLinkBase,
        collapsed ? "justify-center px-0 py-2" : "gap-2 px-2 py-1.5",
      )}
      activeProps={{
        className: cn(
          navLinkBase,
          collapsed ? "justify-center px-0 py-2" : "gap-2 px-2 py-1.5",
          "bg-accent text-accent-foreground",
        ),
      }}
      title={collapsed ? undefined : label}
      aria-label={label}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {collapsed ? null : <span className="truncate">{label}</span>}
    </Link>
  );

  if (!collapsed) return link;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{link}</TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

// A section heading; hidden when collapsed (the grouping is implied by spacing).
function NavSection({
  label,
  collapsed,
  children,
}: {
  label: string;
  collapsed: boolean;
  children: ReactNode;
}) {
  return (
    <>
      {collapsed ? null : (
        <div className="mb-2 px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {label}
        </div>
      )}
      <ul className="space-y-1">{children}</ul>
    </>
  );
}

// The inner application: sidebar nav + routed content. The persistent header
// (Open Lakehouse + theme toggle) is owned by EnvironmentGate, which renders this
// only once an environment is active. The nav collapses to an icon-only rail.
export function AppShell() {
  const [collapsed, setCollapsed] = useState(loadCollapsed);

  const toggle = () => {
    setCollapsed((prev) => {
      const next = !prev;
      persistCollapsed(next);
      return next;
    });
  };

  return (
    <div className="flex flex-1">
      <nav
        className={cn(
          "hidden shrink-0 flex-col border-r bg-sidebar p-2 lg:flex",
          collapsed ? "w-14" : "w-56",
        )}
      >
        <button
          type="button"
          onClick={toggle}
          className={cn(
            "mb-3 flex items-center rounded p-2 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
            collapsed ? "justify-center" : "justify-end",
          )}
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? (
            <PanelLeftOpen className="h-4 w-4" />
          ) : (
            <PanelLeftClose className="h-4 w-4" />
          )}
        </button>

        <div className="mb-6">
          <NavSection label="Catalog" collapsed={collapsed}>
            <li>
              <NavItem
                to="/catalog"
                icon={Database}
                label="Unity Catalog"
                collapsed={collapsed}
              />
            </li>
            <li>
              <NavItem
                to="/editor"
                icon={FileCode}
                label="Editor"
                collapsed={collapsed}
              />
            </li>
            {/* File import is host-gated (desktop reads the local file by path);
                only show the entry when a host registered a file picker. */}
            {ingestSupported() && (
              <li>
                <NavItem
                  to="/import"
                  icon={Upload}
                  label="Import data"
                  collapsed={collapsed}
                />
              </li>
            )}
          </NavSection>
        </div>

        <NavSection label="Services" collapsed={collapsed}>
          {SERVICE_SURFACES.map((service) => (
            <li key={service.id}>
              <NavItem
                to="/services/$serviceId"
                params={{ serviceId: service.id }}
                icon={service.icon}
                label={service.label}
                collapsed={collapsed}
              />
            </li>
          ))}
        </NavSection>
      </nav>
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  );
}
