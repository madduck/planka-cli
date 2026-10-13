# Names instead of IDs

By default `plnk` works with Planka's opaque IDs. The `--names` mode lets you
type names where the CLI expects IDs, and shows names in the output.

```bash
plnk --names board list --project "Platform"
```

The mode is on when either of these is true:

- the global `--names` flag is given, or
- `names = true` is set in `~/.config/plnk/config.toml`.

`--no-names` forces the mode off and overrides a config that sets
`names = true`. `--names` and `--no-names` cannot be combined.

## Input resolution

In names mode, foreign-key flags accept an ID or a name:

```bash
plnk --names list list --board "Sprint 42"
plnk --names card list --board "Sprint 42" --list "In Progress"
```

Resolution is ID-first, then the same three-tier match used by `find`:
exact case-sensitive, exact case-insensitive, then substring. A single match is
used. More than one match is an error (exit `2`) that lists the candidates, so
you can be more specific. No match is an error (exit `4`).

Card IDs are not resolved by name — `--card` and positional card IDs stay
ID-only.

## Output

Table and markdown output show the resolved name instead of the ID. JSON output
adds a sibling `*Name` field next to each foreign key, for example:

```json
{ "id": "1", "projectId": "42", "projectName": "Platform" }
```

The mode is extended resource by resource.

## Projects

`--project` accepts a project name wherever it is expected. JSON output adds
`projectName` next to `projectId`, and table/markdown output shows the name.
