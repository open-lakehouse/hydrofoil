export interface UcServer {
  id: string;
  name: string;
  url: string;
}

export interface SettingsState {
  servers: UcServer[];
}
