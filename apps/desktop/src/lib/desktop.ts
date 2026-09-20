import { isTauri, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import type { Connection } from './api';

export const desktop = {
  available: isTauri,
  connection: () => invoke<Connection>('server_connection'),
  chooseDirectory: () => open({ directory: true, multiple: false, title: '选择要扫描的目录' }),
  reveal: revealItemInDir,
};

export const errorText = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : '操作失败，请重试。';
