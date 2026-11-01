# Control protocol

The desktop app can be driven by other programs (scripts, test harnesses, agents) over a local
control channel. Start it with a port:

```sh
trackcraft --control 7981 team.track        # or TRACKCRAFT_CONTROL_PORT=7981 trackcraft team.track
```

A port that isn't 1–65535 prints an error and exits with status 2; so does a port that can't be
bound or a key file that can't be written. The app listens on `127.0.0.1` only.

## Wire format

JSON lines over TCP: send one JSON object per line, get one reply line per request, in order.

```json
{"id": 1, "key": "…", "method": "engine.execute", "params": {"command": "issue.create", "params": {"project": "LUME", "summary": "Try it"}}}
{"id": 1, "ok": true, "result": {"key": "LUME-8", …}}
```

- `id` (any JSON value, optional) is echoed back in the reply.
- `key` is required on **every** request (see [Keys](#keys)).
- `method` is required; `params` is optional and must be an object when present.
- Errors come back as `{"id": …, "ok": false, "error": "message"}`. A failed engine command leaves
  the workspace unchanged.

Limits — each of these gets one error reply and then the app closes the connection:

| Condition | Error |
|---|---|
| Line is not JSON (including an HTTP request sent to the port) | `bad JSON: …` |
| Line is JSON but not an object | `a request is a JSON object` |
| Missing or wrong key | `unauthorized` |
| No `method`, or `params` not an object | `missing "method"` / `"params" must be an object` |
| Line longer than 16 MiB | `line too long` |
| Line not UTF-8 | `not UTF-8` |

Blank lines are ignored. Before its first authorised request a connection waits at most 30 s per
read; after that it may stay idle. At most 16 connections are served at once. A request the window
doesn't answer within 60 s gets `timeout: the app did not answer` (for example a screenshot of a
minimised window).

## Keys

Any local program, and possibly a web page, can reach a loopback port, so every request carries
the instance's key. Once the port is bound, the app writes a fresh random key (32 bytes from the
operating system, as 64 hex characters) to

```
<config dir>/trackcraft/control-key.<port>
```

and removes the file when the window closes. On Unix the file is created with mode `0600`. A stale
file left by a crashed run is replaced on the next start.

`<config dir>` is `$XDG_CONFIG_HOME` (or `~/.config`) on Linux and the BSDs, `~/Library/Application
Support` on macOS, and `%APPDATA%` on Windows. `TRACKCRAFT_CONFIG_DIR` replaces the whole
`<config dir>/trackcraft` folder (useful for isolated runs and tests). The same folder holds
`recent.json`, the recently opened workspaces (`trackcraft --recent` lists them).

```sh
cat ~/.config/trackcraft/control-key.7981                       # Linux
cat ~/Library/Application\ Support/trackcraft/control-key.7981  # macOS
```

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command` (an engine command id), `params?` | the command's result. Any id from `trackcraft-cli commands`: `issue.create`, `issue.transition`, `search.jql`, `board.view`, `page.edit`, `file.save`, `edit.undo`, … |
| `ui.navigate` | `view`: `home` \| `issues` \| `board` (+ `project`) \| `issue` (+ `key`) \| `page` / `history` (+ `id`) | `{"ok": true}` |
| `ui.search` | `jql` | runs the query in the issue list: `{"ok": true}` |
| `ui.create` | `project?` | opens the Create issue dialog: `{"ok": true}` |
| `ui.palette` | — | toggles the command palette: `{"ok": true}` |
| `ui.editPage` | `id` | opens the page in the editor: `{"ok": true}` |
| `ui.theme` | `dark`: `true` \| `false` (anything else follows the system) | `{"ok": true}` |
| `ui.screenshot` | `path`: an absolute path ending in `.png` | the window as PNG, written atomically to `path`: `{"path", "width", "height"}` (physical pixels) |
| `app.quit` | — | closes the window as the close button does (a dirty workspace with a path is saved first) |

`engine.execute` and the `ui.*` methods except `ui.screenshot` are `TrackApp::control` in
`crates/ui-egui`, the same entry point as the headless `ui_shot` example, so a script written for
one works with the other. `ui.screenshot` and `app.quit` are handled by the desktop app.

## Autosave

Independently of the control channel, the desktop app saves a workspace that has a path and
unsaved changes 30 s after it first became dirty, and again when the window closes. It saves
through the `file.save` command, so saves are atomic (temp file + rename). A workspace that was
never saved (no path) is never autosaved.

## Examples

With `trackcraft-cli send <port> <method> ['{json params}']`, which reads the key file (or
`TRACKCRAFT_CONTROL_KEY`) and prints the reply; it exits non-zero when the reply is an error:

```sh
trackcraft-cli send 7981 engine.execute '{"command": "workspace.info"}'
trackcraft-cli send 7981 engine.execute '{"command": "issue.create", "params": {"project": "LUME", "summary": "Try it"}}'
trackcraft-cli send 7981 ui.navigate '{"view": "board", "project": "LUME"}'
trackcraft-cli send 7981 ui.search '{"jql": "assignee = currentUser() ORDER BY priority"}'
trackcraft-cli send 7981 ui.screenshot '{"path": "/tmp/board.png"}'
trackcraft-cli send 7981 app.quit
```

From a shell without the CLI (`nc`, any language with sockets):

```sh
KEY=$(cat ~/.config/trackcraft/control-key.7981)
printf '{"id":1,"key":"%s","method":"ui.theme","params":{"dark":true}}\n' "$KEY" | nc -w 2 127.0.0.1 7981
```
