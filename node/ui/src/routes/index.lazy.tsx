import { createLazyRoute, Link } from "@tanstack/react-router";
import { Database, FlaskConical, NotebookPen } from "lucide-react";

export const Route = createLazyRoute("/")({
  component: HomePage,
});

function HomePage() {
  return (
    <div className="mx-auto max-w-3xl p-8">
      <h1 className="text-2xl font-semibold tracking-tight">Open Lakehouse</h1>
      <p className="mt-2 text-muted-foreground">
        A single pane of glass over the services running in your lakehouse.
      </p>

      <div className="mt-8 grid gap-4 sm:grid-cols-2">
        <HomeCard
          to="/catalog"
          icon={<Database className="h-5 w-5" />}
          title="Unity Catalog"
          body="Browse catalogs, schemas, and tables."
        />
        <HomeCard
          to="/services/$serviceId"
          params={{ serviceId: "mlflow" }}
          icon={<FlaskConical className="h-5 w-5" />}
          title="MLflow"
          body="Experiment tracking and model registry."
        />
        <HomeCard
          to="/services/$serviceId"
          params={{ serviceId: "marimo" }}
          icon={<NotebookPen className="h-5 w-5" />}
          title="Marimo"
          body="Reactive Python notebooks."
        />
      </div>
    </div>
  );
}

function HomeCard({
  to,
  params,
  icon,
  title,
  body,
}: {
  to: string;
  params?: Record<string, string>;
  icon: React.ReactNode;
  title: string;
  body: string;
}) {
  return (
    <Link
      to={to}
      params={params}
      className="rounded-lg border bg-card p-4 transition-colors hover:border-primary hover:bg-accent"
    >
      <div className="flex items-center gap-2 font-medium">
        {icon}
        {title}
      </div>
      <p className="mt-1 text-sm text-muted-foreground">{body}</p>
    </Link>
  );
}
