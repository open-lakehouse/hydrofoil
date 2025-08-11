import { createContext, Dispatch, SetStateAction, useContext } from "react";
import { SiteSetup, VendorSetup } from "../gen/caspers/core/v1/models_pb";

export type SimulationProps = {
  sites: SiteSetup[];
  vendors: VendorSetup[];
};

export type SimulationState = {
  props: SimulationProps;
  update: Dispatch<SetStateAction<SimulationProps>>;
};

export const SimulationContext = createContext<SimulationState>({
  props: {
    sites: [],
    vendors: [],
  },
  update: () => {},
});
export const useSimulation = () => useContext(SimulationContext);
export const SimulationProvider = SimulationContext.Provider;
