// WP MCP6: the official TypeScript SDK's client (the one Claude Desktop and
// Claude Code are built on) against dspi-mcp's loopback endpoint, over
// Streamable HTTP with the client's token, as an agent would connect. The SDK
// checks every reply against the spec's schemas, and each tool's
// structuredContent against its outputSchema.
//
//   SERVER_PORT=<port> MCP_TOKEN=<token> node sdk-client.mjs
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const url = new URL(`http://127.0.0.1:${process.env.SERVER_PORT}/mcp`);
const token = process.env.MCP_TOKEN ?? '';
let failed = 0;
let passed = 0;
function check(name, ok, detail = '') {
  if (ok) passed += 1;
  else failed += 1;
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? `: ${detail}` : ''}`);
}

const transport = new StreamableHTTPClientTransport(url, {
  requestInit: { headers: { Authorization: `Bearer ${token}` } },
});
const client = new Client({ name: 'mcp6-sdk-client', version: '1.0.0' });
try {
  await client.connect(transport);
  const info = client.getServerVersion();
  check('initialize', info?.name === 'dspi-console', JSON.stringify(info));
  check('instructions', (client.getInstructions() ?? '').includes('dspi://guide'));
  check('session id', typeof transport.sessionId === 'string' && transport.sessionId.length > 0);
  await client.ping();
  check('ping', true);

  const { tools } = await client.listTools();
  check('tools/list', tools.length >= 40, `${tools.length} tools`);
  check(
    'tool names',
    tools.every((t) => /^[A-Za-z0-9_.-]{1,128}$/.test(t.name)),
    tools.map((t) => t.name).filter((n) => !/^[A-Za-z0-9_.-]{1,128}$/.test(n)).join(','),
  );
  // Every read tool, with no arguments: a result or a tool error, never a
  // protocol failure, and structuredContent that matches its outputSchema
  // (the SDK throws otherwise).
  const reads = tools.filter((t) => t.annotations?.readOnlyHint === true);
  check('read tools are marked', reads.length >= 15, `${reads.length} read tools`);
  for (const t of reads) {
    try {
      const r = await client.callTool({ name: t.name, arguments: {} });
      const text = Array.isArray(r.content) && r.content.every((c) => c.type === 'text');
      check(`tools/call ${t.name}`, text, r.isError ? 'tool error (allowed)' : '');
    } catch (e) {
      check(`tools/call ${t.name}`, false, String(e));
    }
  }
  try {
    await client.callTool({ name: 'dspi_no_such_tool', arguments: {} });
    check('unknown tool is a protocol error', false, 'no error');
  } catch (e) {
    check('unknown tool is a protocol error', e?.code === -32602, String(e));
  }

  const { resources } = await client.listResources();
  check('resources/list', resources.some((r) => r.uri === 'dspi://guide'), `${resources.length} resources`);
  for (const r of resources) {
    try {
      const got = await client.readResource({ uri: r.uri });
      check(`resources/read ${r.uri}`, got.contents.length > 0 && got.contents.every((c) => c.uri === r.uri));
    } catch (e) {
      check(`resources/read ${r.uri}`, false, String(e));
    }
  }
  const { resourceTemplates } = await client.listResourceTemplates();
  check('resources/templates/list', resourceTemplates.length > 0, `${resourceTemplates.length} templates`);

  const { prompts } = await client.listPrompts();
  check('prompts/list', prompts.length > 0, `${prompts.length} prompts`);
  for (const p of prompts) {
    const args = Object.fromEntries((p.arguments ?? []).filter((a) => a.required).map((a) => [a.name, '0']));
    try {
      const got = await client.getPrompt({ name: p.name, arguments: args });
      check(`prompts/get ${p.name}`, got.messages.length > 0);
    } catch (e) {
      check(`prompts/get ${p.name}`, false, String(e));
    }
  }
} catch (e) {
  check('session', false, String(e?.stack ?? e));
} finally {
  // Ends the session (DELETE /mcp).
  await transport.terminateSession().catch((e) => check('DELETE /mcp', false, String(e)));
  await client.close();
}
console.log(`sdk-client: ${passed} passed, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
