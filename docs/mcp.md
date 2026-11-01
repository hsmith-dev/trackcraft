# MCP

`trackcraft-cli mcp --in team.track` (or `--sample`) serves the Model Context Protocol on
stdio, one JSON-RPC message per line. With `--in`, every successful change is saved to that file
atomically.

Tools:

- `trackcraft_commands`: the command list (`id`, `label`, `location`, `shortcut`, `params`,
  `mutates`, `disabled`).
- `trackcraft_run`: `{"command": "<id>", "params": {…}}`. Returns the command's JSON result
  as text and as `structuredContent.result`; errors come back with `isError: true`.

Example client config:

```json
{"mcpServers": {"trackcraft": {"command": "trackcraft-cli", "args": ["mcp", "--in", "/path/team.track"]}}}
```
