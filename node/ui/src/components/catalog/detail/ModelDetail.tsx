import { useQuery } from "@tanstack/react-query";

import { modelDetailQuery } from "@/lib/uc/queries";

import { DetailStates } from "./DetailStates";
import { Meta, MetaGrid } from "./Meta";

export function ModelDetail({ fullName }: { fullName: string }) {
  const {
    data: model,
    isLoading,
    error,
  } = useQuery(modelDetailQuery(fullName));
  if (!model) return <DetailStates isLoading={isLoading} error={error} />;

  return (
    <MetaGrid>
      <Meta label="Owner" value={model.owner} />
      <Meta label="Storage location" value={model.storage_location} wide mono />
      <Meta label="Comment" value={model.comment} wide />
    </MetaGrid>
  );
}
