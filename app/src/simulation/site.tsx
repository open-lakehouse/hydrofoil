import { Tree, TreeItem, TreeItemLayout } from "@fluentui/react-components";
import {
  SiteSetup,
  Station,
  KitchenSetup,
} from "../gen/caspers/core/v1/models_pb";
import { useSimulation } from "./context";

function StationNode({ station }: { station: Station }) {
  return (
    <TreeItem itemType="leaf">
      <TreeItemLayout>{station.name}</TreeItemLayout>
    </TreeItem>
  );
}

function KitchenNode({ kitchen }: { kitchen: KitchenSetup }) {
  return (
    <TreeItem itemType="branch">
      <TreeItemLayout>{kitchen.info?.name}</TreeItemLayout>
      <Tree>
        {kitchen.stations.map((station) => (
          <StationNode station={station} />
        ))}
      </Tree>
    </TreeItem>
  );
}

function SiteNode({ site }: { site: SiteSetup }) {
  return (
    <TreeItem itemType="branch">
      <TreeItemLayout>{site.info?.name}</TreeItemLayout>
      <Tree>
        {site.kitchens.map((kitchen) => (
          <KitchenNode kitchen={kitchen} />
        ))}
      </Tree>
    </TreeItem>
  );
}

export function CaspersTree() {
  const { props } = useSimulation();

  return (
    <Tree>
      {props.sites.map((site) => (
        <SiteNode site={site} />
      ))}
    </Tree>
  );
}
