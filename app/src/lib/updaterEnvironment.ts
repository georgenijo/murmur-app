import { invoke } from '@tauri-apps/api/core';

export interface UpdateInstallEnvironment {
  appTranslocated: boolean;
}

export async function getUpdateInstallEnvironment(): Promise<UpdateInstallEnvironment> {
  return await invoke('get_update_install_environment');
}
