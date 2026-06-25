import { invoke } from '@tauri-apps/api/core';

export type WorkspaceEntryKind = 'directory' | 'file';

export type WorkspaceEntry = {
  name: string;
  path: string;
  relativePath: string;
  kind: WorkspaceEntryKind;
  extension?: string | null;
  size?: number | null;
  modified?: number | null;
};

export type WorkspaceDirectoryListing = {
  root: string;
  path: string;
  relativePath: string;
  entries: WorkspaceEntry[];
};

export type WorkspaceFileContent = {
  path: string;
  relativePath: string;
  content: string;
  size: number;
};

export async function listWorkspaceDirectory(
  conversationId: string | null,
  path = '',
): Promise<WorkspaceDirectoryListing> {
  return invoke<WorkspaceDirectoryListing>('workspace_list_directory', {
    conversationId,
    path,
  });
}

export async function readWorkspaceTextFile(
  conversationId: string | null,
  path: string,
): Promise<WorkspaceFileContent> {
  return invoke<WorkspaceFileContent>('workspace_read_text_file', {
    conversationId,
    path,
  });
}

export async function setDefaultWorkspaceRoot(path: string): Promise<string> {
  return invoke<string>('set_default_workspace_root', { path });
}
