import { SimulationProps } from "./context";

export type SimulationOptions = {
  storagePath: string;
  snapshotInterval: number;
  timeIncrement: number;
};

export function runSimulation(
  props: SimulationProps,
  options: SimulationOptions,
) {
  // Implementation of the simulation
}
