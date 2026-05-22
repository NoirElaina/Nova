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

export async function listWorkspaceDirectory(path = ''): Promise<WorkspaceDirectoryListing> {
  return invoke<WorkspaceDirectoryListing>('workspace_list_directory', {
    path,
  });
}

export async function readWorkspaceTextFile(path: string): Promise<WorkspaceFileContent> {
  return invoke<WorkspaceFileContent>('workspace_read_text_file', {
    path,
  });
}

export async function setWorkspaceRoot(path: string): Promise<WorkspaceDirectoryListing> {
  return invoke<WorkspaceDirectoryListing>('workspace_set_root', {
    path,
  });
}
