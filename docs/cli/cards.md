# Cards

Cards belong to a list. They are the primary work items in Planka. Cards have tasks (checklists), comments, labels, assignees, and attachments.

## Commands

### List cards in a list or across a board

```bash
plnk card list --list <listId>
plnk card list --board <boardId>
plnk cards --list <listId>                  # alias
plnk cards --board <boardId>                # alias
```

Filter by one or more board-scoped labels with repeated `--label` flags. Repeated labels use AND semantics.

```bash
plnk card list --list <listId> --label <labelId>
plnk card list --board <boardId> --label "Urgent"
plnk card list --board <boardId> --label "Urgent" --label "Backend"
```

### Get a card by ID

```bash
plnk card get <cardId>
plnk card get 1234 --output json
plnk card get 1234 --full                   # include all fields
```

### Get many cards by exact ID

```bash
plnk card get-many --id <cardId> --id <cardId>
plnk card get-many --id 1234 --id 5678 --output json
plnk card get-many --id 1234 --id 5678 --concurrency 1
plnk card get-many --id 1234 --id 9999 --allow-missing --output json
```

Notes:
- Exact ID fetch only — no title matching or search fallback.
- Output order matches the input `--id` order.
- Default concurrency is `4`; valid range is `1..=16`.
- `--allow-missing` downgrades per-card `404` misses into metadata, but auth/server/transport failures still fail the command.
- JSON `meta` includes `requestedCount`, `foundCount`, `missingCount`, `missingIds`, `concurrency`, and `allowMissing`.

### Find cards by title, label, member, or assignee

Must be scoped to a list, board, or project. Title matching uses three-tier matching: exact > case-insensitive > substring.

```bash
# Search within a list (fastest, fewest API calls)
plnk card find --list <listId> --title "Fix auth"
plnk card find --list <listId> --label "Urgent"

# Search within a board (searches all lists)
plnk card find --board <boardId> --title "Fix auth"
plnk card find --board <boardId> --label "Urgent"
plnk card find --board <boardId> --label "Urgent" --label "Backend" --title "Fix auth"

# Cards assigned to a user via card membership
plnk card find --board <boardId> --member jane

# Cards with at least one task assigned to a user
plnk card find --board <boardId> --assignee jane

# Both at once: jane is a member AND has a task assigned
plnk card find --board <boardId> --member jane --assignee jane

# Either relation: jane is a member OR has a task assigned
# (matches Planka's web UI "Members" button)
plnk card find --board <boardId> --assignee-or-member jane

# Search within a project (searches all boards and lists)
plnk card find --project <projectId> --title "auth"
```

Notes:
- At least one of `--title`, `--label`, `--member`, `--assignee`, or
  `--assignee-or-member` is required.
- `--label`, `--member`, `--assignee`, and `--assignee-or-member` are only
  supported with `--list` or `--board` scopes.
- Label names are resolved within the owning board using exact > case-insensitive > substring matching.
- If a label query matches more than one board label, the CLI tells you it matched multiple labels and prints the candidates.
- If no board label matches, the CLI tells you no label matched in the current board scope.
- If a name is ambiguous, use the label ID instead (`plnk label list --board <boardId>`).
- `--member`, `--assignee`, and `--assignee-or-member` accept a user ID,
  username, or display name. Use `plnk user list` to inspect candidates.
- Repeating any of the three flags is OR within that flag. Combining `--member`
  with `--assignee` is AND between them. See the next section.
- `--assignee-or-member` cannot be combined with `--member` or `--assignee`.
  It is shorthand for both at once.

#### Member versus assignee

These are separate filters on purpose. Planka's web UI has a single "Members"
button that shows a card if you are a member **or** if one of its tasks is
assigned to you. `plnk` splits that into two flags so you can ask for either
half precisely:

| Flag | Matches |
|------|---------|
| `--member` | The user is attached to the card itself (card membership) |
| `--assignee` | At least one task on the card is assigned to the user |
| `--assignee-or-member` | Either of the two above |

When `--member` and `--assignee` are given together, both must hold: the card
needs a matching member **and** a matching task assignee.

`--assignee-or-member` is the union instead, and it is the flag that matches
Planka's web UI. `plnk card find --board <id> --assignee-or-member jane` returns
exactly the cards the web UI shows when Jane filters by herself.

Both filters are computed client-side from the board snapshot. The Planka API
has no endpoint that filters cards by member or assignee.

Two limits are worth knowing:
- Cards in non-finite lists are not part of the board snapshot, so they never
  appear in `--member` or `--assignee` results.
- Assignees are resolved against the full user list, not the board members, so
  a task assignee who is not a board member still resolves correctly.

Always returns a collection. Multiple results are expected, not an error.

### Get a card snapshot

```bash
plnk card snapshot <cardId> --output json
```

Returns the full `GET /api/cards/{id}` response verbatim, including `item` (the card — with fields the normal `get` discards like `commentsTotal`, `coverAttachmentId`, `listChangedAt`, `prevListId`, `stopwatch`, `type`) and `included` (tasks, taskLists, comments, attachments, cardMemberships, cardLabels, users, customFieldGroups, customFields, customFieldValues). Nothing is dropped. JSON only.

### Create a card

```bash
plnk card create --list <listId> --title "Fix auth bug"

# With description
plnk card create --list 789 --title "Fix auth" --description "OAuth flow broken on mobile"

# Description from file
plnk card create --list 789 --title "Fix auth" --description @spec.md

# Description from stdin
echo "Details here" | plnk card create --list 789 --title "Fix auth" --description -

# Control position
plnk card create --list 789 --title "Urgent fix" --position top
plnk card create --list 789 --title "Low priority" --position bottom
```

### Update a card

```bash
plnk card update <cardId> --title "Fix auth race condition"
plnk card update <cardId> --description "Updated details"
plnk card update <cardId> --description @updated-spec.md

# Pipe from clipboard (macOS)
pbpaste | plnk card update <cardId> --description -
```

At least one field must be provided.

### Move a card to a different list

```bash
plnk card move <cardId> --to-list <listId>
plnk card move 1234 --to-list 790 --position top
plnk card move 1234 --to-list 790 --position bottom
```

To move across boards, add `--to-board`:

```bash
plnk card move 1234 --to-board 456 --to-list 790
```

### Archive / unarchive a card

```bash
plnk card archive <cardId>
plnk card unarchive <cardId>
```

### Delete a card

```bash
plnk card delete <cardId>
plnk card delete 1234 --yes
```

### Card labels

Manage which labels are applied to a card. Labels must first be created on the board (see [Labels](labels.md)).

```bash
plnk card label list <cardId>
plnk card label add <cardId> <labelId>
plnk card label remove <cardId> <labelId>
```

### Card assignees

Manage which users are assigned to a card.

```bash
plnk card assignee list <cardId>
plnk card assignee add <cardId> <userId>
plnk card assignee remove <cardId> <userId>
```

### Custom field values

```bash
plnk card field list <cardId>
plnk card field set <cardId> --group <id|name> --field <id|name> --value "specs/design.html"
plnk card field clear <cardId> --group <id|name> --field <id|name>
```

`--group` and `--field` accept an ID or a name, resolving through the card's attached groups
(and through the base group when the card's group was adopted from a template). Values are
capped at 512 characters and cannot be empty; `clear` is idempotent.

See [Custom fields](custom-fields.md) for the full model, including how to define a reusable
field template on a project and adopt it onto a card.

## Examples

```bash
# Full card lifecycle
plnk card create --list 789 --title "Implement login" --description @login-spec.md
plnk card label add 1234 111                     # tag as "urgent"
plnk card assignee add 1234 88                   # assign to user
plnk card move 1234 --to-list 790                # move to "In Progress"
plnk task create --card 1234 --title "Write tests"
plnk task create --card 1234 --title "Update docs"
plnk comment create --card 1234 --text "Starting work"
plnk card move 1234 --to-list 791 --position top # move to "Done"

# Script: find all cards matching "auth" in a project, output JSON
plnk card find --project 123 --title "auth" --output json

# Script: create a card and capture its ID
ID=$(plnk card create --list 789 --title "New task" --output json | jq -r '.data.id')
echo "Created card $ID"
```
