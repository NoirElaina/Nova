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

export type LspServerStatus = {
  languageId: string;
  displayName: string;
  command?: string | null;
  available: boolean;
  running: boolean;
  diagnosticCount: number;
  error?: string | null;
};

export type LspStatusResponse = {
  workspaceRoot: string;
  servers: LspServerStatus[];
};

export type LspDiagnostic = {
  uri: string;
  path: string;
  relativePath: string;
  message: string;
  severity?: number | null;
  source?: string | null;
  code?: string | null;
  line: number;
  character: number;
  endLine: number;
  endCharacter: number;
};

export type LspDiagnosticsResponse = {
  workspaceRoot: string;
  server?: string | null;
  diagnostics: LspDiagnostic[];
};

export type LspLocation = {
  uri: string;
  path: string;
  relativePath: string;
  line: number;
  character: number;
  endLine: number;
  endCharacter: number;
};

export type LspRequestResponse = {
  workspaceRoot: string;
  server: string;
  result: unknown;
  locations: LspLocation[];
};

export type LspSymbolsResponse = {
  workspaceRoot: string;
  server: string;
  result: unknown;
};

export type LspHoverResponse = {
  workspaceRoot: string;
  server: string;
  result: unknown;
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

export async function setWorkspaceRoot(
  conversationId: string | null,
  path: string,
): Promise<WorkspaceDirectoryListing> {
  return invoke<WorkspaceDirectoryListing>('workspace_set_root', {
    conversationId,
    path,
  });
}

export async function getLspStatus(
  conversationId: string | null,
): Promise<LspStatusResponse> {
  return invoke<LspStatusResponse>('lsp_status', {
    conversationId,
  });
}

export async function getLspDiagnostics(
  conversationId: string | null,
  path?: string | null,
): Promise<LspDiagnosticsResponse> {
  return invoke<LspDiagnosticsResponse>('lsp_diagnostics', {
    conversationId,
    path,
  });
}

export async function getLspDefinition(
  conversationId: string | null,
  path: string,
  line: number,
  character: number,
): Promise<LspRequestResponse> {
  return invoke<LspRequestResponse>('lsp_definition', {
    conversationId,
    path,
    line,
    character,
  });
}

export async function getLspReferences(
  conversationId: string | null,
  path: string,
  line: number,
  character: number,
  includeDeclaration = true,
): Promise<LspRequestResponse> {
  return invoke<LspRequestResponse>('lsp_references', {
    conversationId,
    path,
    line,
    character,
    includeDeclaration,
  });
}

export async function getLspSymbols(
  conversationId: string | null,
  options: { path?: string | null; query?: string | null } = {},
): Promise<LspSymbolsResponse> {
  return invoke<LspSymbolsResponse>('lsp_symbols', {
    conversationId,
    path: options.path,
    query: options.query,
  });
}

export async function getLspHover(
  conversationId: string | null,
  path: string,
  line: number,
  character: number,
): Promise<LspHoverResponse> {
  return invoke<LspHoverResponse>('lsp_hover', {
    conversationId,
    path,
    line,
    character,
  });
}
