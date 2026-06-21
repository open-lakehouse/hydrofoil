import { useQuery } from "@tanstack/react-query";

import { schemaDetailQuery } from "@/lib/uc/queries";

import { DetailStates } from "./DetailStates";
import { Meta, MetaGrid } from "./Meta";

export function SchemaDetail({ fullName }: { fullName: string }) {
  const {
    data: schema,
    isLoading,
    error,
  } = useQuery(schemaDetailQuery(fullName));
  if (!schema) return <DetailStates isLoading={isLoading} error={error} />;

  return (
    <MetaGrid>
      <Meta label="Owner" value={schema.owner} />
      <Meta label="Catalog" value={schema.catalog_name} />
      <Meta label="Storage root" value={schema.storage_root} wide mono />
      <Meta label="Storage location" value={schema.storage_location} wide mono />
      <Meta label="Comment" value={schema.comment} wide />
    </MetaGrid>
  );
}
