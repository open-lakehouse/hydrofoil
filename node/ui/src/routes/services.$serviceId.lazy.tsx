import { createLazyRoute } from "@tanstack/react-router";
import { ServiceFrame } from "@/components/ServiceFrame";
import { getServiceSurface } from "@/lib/services";

export const Route = createLazyRoute("/services/$serviceId")({
  component: ServicePage,
});

function ServicePage() {
  const { serviceId } = Route.useParams();
  const service = getServiceSurface(serviceId);

  if (!service) {
    return (
      <div className="p-8">
        <h1 className="text-lg font-semibold">Unknown service</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          No service surface is registered for{" "}
          <code className="rounded bg-muted px-1">{serviceId}</code>.
        </p>
      </div>
    );
  }

  return <ServiceFrame service={service} />;
}
