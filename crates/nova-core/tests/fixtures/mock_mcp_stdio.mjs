import readline from 'node:readline';

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

function send(msg) {
  process.stdout.write(`${JSON.stringify(msg)}\n`);
}

rl.on('line', (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let req;
  try {
    req = JSON.parse(trimmed);
  } catch {
    return;
  }

  // Notifications have no id
  if (req.id === undefined || req.id === null) {
    return;
  }

  switch (req.method) {
    case 'initialize':
      send({
        jsonrpc: '2.0',
        id: req.id,
        result: {
          protocolVersion: '2024-11-05',
          capabilities: { tools: {}, resources: {} },
          serverInfo: { name: 'mock-mcp-stdio', version: '0.1.0' },
        },
      });
      break;
    case 'tools/list':
      send({
        jsonrpc: '2.0',
        id: req.id,
        result: {
          tools: [
            {
              name: 'mock_echo',
              description: 'Echoes input from stdio MCP mock',
              inputSchema: {
                type: 'object',
                properties: { text: { type: 'string' } },
                required: ['text'],
              },
            },
          ],
        },
      });
      break;
    case 'tools/call':
      send({
        jsonrpc: '2.0',
        id: req.id,
        result: {
          content: [
            {
              type: 'text',
              text: `stdio:${req.params?.arguments?.text ?? ''}`,
            },
          ],
        },
      });
      break;
    case 'resources/list':
      send({
        jsonrpc: '2.0',
        id: req.id,
        result: {
          resources: [
            {
              uri: 'mock://stdio/readme',
              name: 'stdio-readme',
              mimeType: 'text/plain',
            },
          ],
        },
      });
      break;
    case 'resources/read':
      send({
        jsonrpc: '2.0',
        id: req.id,
        result: {
          contents: [{ uri: req.params?.uri ?? '', text: 'stdio-resource-body' }],
        },
      });
      break;
    default:
      send({
        jsonrpc: '2.0',
        id: req.id,
        error: { code: -32601, message: `unknown method: ${req.method}` },
      });
      break;
  }
});
