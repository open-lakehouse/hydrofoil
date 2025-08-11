import { create } from "@bufbuild/protobuf";
import {
  KitchenStation,
  SiteSetup,
  SiteSetupSchema,
} from "../gen/caspers/core/v1/models_pb";

export function getData(): SiteSetup[] {
  let siteSetup = create(SiteSetupSchema, {
    info: {
      name: "London",
      latitude: 51.518898098201326,
      longitude: -0.13381370382489707,
    },
    kitchens: [
      {
        info: {
          name: "kitchen-1",
        },
        stations: [
          {
            name: "station-1",
            stationType: KitchenStation.WORKSTATION,
          },
        ],
      },
    ],
  });

  return [siteSetup];
}
