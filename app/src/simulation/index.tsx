import {
  DrawerBody,
  DrawerHeader,
  DrawerHeaderTitle,
  InlineDrawer,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { useState } from "react";
import { SimulationProps, SimulationProvider } from "./context";
import { CaspersTree } from "./site";
import { getData } from "./data";

const useStyles = makeStyles({
  root: {
    display: "flex",
    height: "100%",
    width: "100%",
    userSelect: "auto",
  },

  container: {
    position: "relative",
  },

  drawer: {
    width: "320px",
    borderRightColor: tokens.colorNeutralForeground4,
    borderRightWidth: "1px",
    borderRightStyle: "solid",
    height: "100%",
  },

  content: {
    flex: "1",
  },
});

function Simulation() {
  const styles = useStyles();
  const [state, setState] = useState<SimulationProps>({
    sites: getData(),
    vendors: [],
  });

  return (
    <SimulationProvider value={{ props: state, update: setState }}>
      <div className={styles.root}>
        <div className={styles.container}>
          <InlineDrawer open className={styles.drawer}>
            <DrawerHeader>
              <DrawerHeaderTitle>Caspers Universe</DrawerHeaderTitle>
            </DrawerHeader>
            <DrawerBody>
              <CaspersTree />
            </DrawerBody>
          </InlineDrawer>
        </div>
        <div className={styles.content}>
          <div />
        </div>
      </div>
    </SimulationProvider>
  );
}

export default Simulation;
