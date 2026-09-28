# Linear and Jira kanban boards: what to copy

Question: how do the Linear and Jira boards look and behave, and which ideas fit a native desktop board over local markdown tickets?

Sources are official docs (linear.app/docs, support.atlassian.com) unless marked "third party". "Unverified" marks a detail I could not confirm in a primary source.

## 1. Board layout

### Linear

- A board is a layout of any issue view, not a separate object. `Cmd/Ctrl+B` toggles list and board. List and board share one ordering.
- Default grouping is Status. Other groupings: assignee, project, priority, cycle, label, label group, parent issue, team, SLA status. "No grouping" also exists.
- Status columns always follow workflow order (first to last), unlike some list orderings.
- Column header: status icon, status name, count, a `+` button (creates an issue already in that status), and a `...` menu (Hide column).
- The count next to each group can toggle between issue count and total estimate. Click the number to switch.
- Hidden columns appear as the last column on the board. You can still drag cards into a hidden column without unhiding it.
- "Show empty groups" is a display option. Turn it off to drop empty columns.
- Swimlanes = "sub-grouping". They render as rows on a board. Each lane can collapse (`T` toggles). The group header stays sticky while you scroll.
- Dragging a card into another group or lane sets that property ("adopts the properties of that grouping").
- Horizontal scroll: shift+wheel, trackpad, or click-and-drag on empty board space.
- No WIP limits and no per-column background color. Color comes from the status icon only.

### Jira

- Columns map to one or more workflow statuses. Unmapped statuses sit in a side panel in settings. Default kanban columns: Backlog, Selected for Development, In Progress, Done.
- Column bar color follows the status category: first column grey (new), middle columns blue (in progress), last column green (done).
- Column constraints (WIP limits): set a min and/or max per column. Count mode is "work item count" or "count excluding subtasks". The limit shows in the header ("Max: 2"). Header turns red when max is exceeded and yellow when min is not met. Limits are visual only and do not block moves. They count cards hidden by quick filters too.
- Swimlane methods: Queries (one JQL per lane, default "Expedite" = `priority = Blocker`, plus a catch-all "Everything Else" that cannot be deleted), Stories (parent per lane, subtasks inside), Assignees (unassigned above or below), Epics (parent per lane, rest below), Spaces, or none.
- Board shortcut `-` toggles all swimlanes.

Sources: [Linear board layout](https://linear.app/docs/board-layout), [Linear display options](https://linear.app/docs/display-options), [Jira configure columns](https://support.atlassian.com/jira-software-cloud/docs/configure-columns/), [Jira configure swimlanes](https://support.atlassian.com/jira-software-cloud/docs/configure-swimlanes/), [Atlassian community on WIP visuals](https://community.atlassian.com/forums/Jira-questions/Applying-WIP-limits-on-Jira-columns/qaq-p/2724923).

## 2. Card anatomy and display options

### Linear card

- Cards never show the description. If a card has too many properties, some do not fit. Use peek (`Space`) or open the issue for the rest.
- Typical order (from the product, not a doc): top row is ID on the left and assignee avatar on the right. Next is the status icon plus title (title wraps up to two lines). The bottom row holds property chips: priority icon, labels (colored dot + name), project, cycle, estimate, due date, sub-issue progress, links/PR icon. Unverified as a fixed spec; it is what the app renders.
- Display properties that toggle on and off: ID, status, assignee, priority, SLA, project, due date, milestone, cycle, release, estimate, labels, links, customers, customer revenue, time in status, created date, updated date, pull requests and commits, Sentry issues.
- Due date chip: calendar icon, red if due today or overdue, orange if due within a week, grey otherwise. Hover shows the date and days left or past.
- Blocked state: in the issue sidebar, "Blocked by" shows an orange flag and "Blocks" a red flag. On cards and rows a blocked icon appears when the relation exists (unverified as a display option name).
- Other display options: grouping, sub-grouping, ordering (Manual, Status, Priority, Last created, Last updated, Due date, Link count; reversible except Manual), show sub-issues on/off, show empty groups. Manual order is the board default and is shared by everyone.
- Display options save per person. "Set as default" saves them as the view default for the workspace. "Reset to default" reverts. `Shift+V` opens the panel.
- Priority: No priority, Urgent, High, Medium, Low. No custom priorities, by design. No-priority sorts last. Icon: three rising bars filled to the level; Urgent is a filled square with "!" (third party).

### Jira card

- Three stacked layers: summary always on top, then up to three custom fields, then details (work type icon, priority, assignee avatar, estimate). The work item key shows at the bottom of the card.
- Toggleable fields (view settings): work type, work item key, epic (colored lozenge), linked work items, priority, assignee, card cover, due date and labels (team-managed), status and version (backlog).
- Card color: a thin strip on the left edge. Choose one method per board: work type (Task blue, Sub-task light blue, Bug red, Story purple), priority, assignee, or JQL queries (first match wins, no match = grey).
- "Days in column": dots on the card for 1, 2, 3, 5, 8, 12, 20+ days. Cumulative if a card returns to a column. Good cheap aging signal.
- Development icons (branch, commit, PR) show on cards when keys appear in git work. Not shown on boards over 100 items.
- Flag: marks an impediment. The flagged card gets a highlighted (yellow/orange) background and a flag icon (third party).

Sources: [Linear display options](https://linear.app/docs/display-options), [Linear board FAQ](https://linear.app/docs/board-layout), [Linear due dates](https://linear.app/docs/due-dates), [Linear priority](https://linear.app/docs/priority), [Linear issue relations](https://linear.app/docs/issue-relations), [Jira customize cards](https://support.atlassian.com/jira-software-cloud/docs/customize-cards/), [Jira customize board view](https://support.atlassian.com/jira-software-cloud/docs/customize-your-view-of-the-board-and-backlog/), [Jira development info](https://support.atlassian.com/jira-software-cloud/docs/enable-code/), [priority icon shape, third party](https://cephalochromoscope.net/b3d81cc8-90bd-46db-8e5b-429d132b510b), [flag, third party](https://estudy247.com/courses/jira/lessons/jira-scrum-board/).

## 3. Workflow state categories and icons (Linear)

Categories are fixed and ordered. Teams add, rename, recolor and reorder statuses only inside a category. Every category must keep one status.

| Category (`type`) | Default status | Icon shape | Default color |
| --- | --- | --- | --- |
| triage | Triage | (inbox-style, optional category) | n/a |
| backlog | Backlog | dashed circle | `#bec2c8` |
| unstarted | Todo | empty circle (solid ring) | `#e2e2e2` |
| started | In Progress | ring with a filled pie sector (half) | `#f2c94c` |
| started | In Review (common addition) | ring with larger sector | `#0f783c` or `#f2994a` seen; not a default |
| completed | Done | filled circle with check | `#5e6ad2` |
| canceled | Canceled | filled circle with cross | `#95a2b3` |
| duplicate | Duplicate (system, auto) | canceled-style icon | `#95a2b3` (unverified) |

- The started icon fills more as the state moves right inside the Started category. This gives a "progress pie" read across the board (observed behaviour, unverified in docs).
- Default status for new issues is the first Backlog status. You can pick another Backlog or Todo status.
- Linear's own team uses: Backlog (Icebox, Backlog), Unstarted (Todo), Started (In Progress, In Review, Ready to Merge), Completed (Done), Canceled (Canceled, Could not reproduce, Won't Fix), Duplicate.
- Auto-close closes stale issues after a set time. Auto-archive archives closed issues.

Sources: [Linear issue status](https://linear.app/docs/configuring-workflows), [Terraform provider defaults (backlog, completed, canceled colors)](https://registry.terraform.io/providers/terraform-community-providers/linear/latest/docs/resources/team), [third-party state table incl. #e2e2e2, #f2c94c](https://terminalskills.io/use-cases/build-engineering-workflow-with-linear), [WorkflowState types](https://docs.rs/linear-api/latest/linear_api/workspace/struct.WorkflowState.html), [icon shapes, third party](https://fp.dev/docs/extensions/status/), [Linear-style StateIcon, third party](https://cephalochromoscope.net/b3d81cc8-90bd-46db-8e5b-429d132b510b).

## 4. Issue detail view

### Linear

- Three depths: peek, side panel (`Cmd+I` opens the details sidebar), full issue view.
- Peek: `Space` toggles, hold `Space` for a temporary preview. `Up`/`Down` walk adjacent issues while peek stays open. `Esc` closes. Shows description, assignee, status, priority, cycle, labels, estimate, created and updated dates.
- Full view, main column: breadcrumb (team > parent) then title and description, both inline-editable and autosaved. Description history is restorable. Below: sub-issues list with progress and `+ Add sub-issues`, then the activity feed.
- Activity feed: history events (status changes, assignments, labels) interleaved with comments. Comments support threads, resolve, reactions, attachments. `Cmd+Enter` posts. Unsent comments persist as drafts.
- Properties sidebar (right): status, priority, assignee, labels, project (+ milestone), cycle, estimate (`Shift+E`), due date (`Shift+D`), parent. Relations section: Blocked by (orange flag), Blocks (red flag), Related, Duplicate of. When a blocker resolves, the relation moves to Related. Links and PR/branch attachments show here too.
- Mentions of another issue in text auto-create a Related relation.
- Relation keys: `M` then `B` blocked by, `M` then `X` blocking, `M` then `R` related.
- Sub-issues inherit team, priority and project from the parent, not labels. Optional automations: close parent when all sub-issues are done; close sub-issues when parent is done.
- Copy actions: `Cmd+.` copy ID, `Cmd+Shift+,` copy URL, `Cmd+Shift+.` copy git branch name. Linking a PR moves the status automatically.

### Jira

- Opening a card from the board shows the work item in a detail view (modal or panel, depending on the board version). The key sits in a breadcrumb with the parent (epic).
- Status is a button near the top. It opens a list of workflow transitions, not a free list of statuses.
- Main column ("description fields"): summary, description, attachments, child work items (with a progress bar), linked work items (typed links like "blocks", "is blocked by", "relates to", "duplicates").
- Right column ("context fields"): Details group (assignee, reporter, labels, priority, sprint, story points, parent...). Users can pin fields. Empty fields hide under "Show N more fields". Created, Updated and Resolved dates sit at the bottom.
- Development panel: counts of branches, commits, PRs, builds, deployments. Buttons for Create branch (copies a `git checkout -b KEY-...` command), Create commit, Create pull request.
- Activity: tabs All, Comments, History, Work log. A 2026 redesign moves some tabs behind a "More" menu and unifies history; users complained about the extra click.

Sources: [Linear peek](https://linear.app/docs/peek), [Linear edit issues](https://linear.app/docs/editing-issues), [Linear comments](https://linear.app/docs/comment-on-issues), [Linear issue relations](https://linear.app/docs/issue-relations), [Linear sub-issues](https://linear.app/docs/parent-and-sub-issues), [Linear GitHub](https://linear.app/docs/github), [Linear shortcuts, third party](https://keycombiner.com/collections/linear/), [Jira update details](https://support.atlassian.com/jira-software-cloud/docs/update-a-work-items-details/), [Jira activity types](https://support.atlassian.com/jira-software-cloud/docs/what-are-the-different-types-of-activity-on-an-issue/), [Jira development info](https://support.atlassian.com/jira-software-cloud/docs/view-development-information-for-an-issue/), [Jira new work item view](https://community.atlassian.com/forums/Jira-articles/A-clearer-more-focused-Jira-work-item-view-is-coming/ba-p/3262002), [Activity tabs complaint](https://jira.atlassian.com/browse/JIRAAUTOSERVER-1217).

## 5. Interactions

### Linear

- Focus vs selection: hover or `J`/`K`/arrows moves focus. `X` selects, `Shift+X` / `Shift+click` / `Shift+Up/Down` extends. `Cmd+A` selects all visible. Actions apply to focused or selected cards.
- Single-key actions on focus or selection: `S` status, `A` assign, `I` assign to me, `P` priority, `L` labels, `Shift+E` estimate, `Shift+D` due date, `C` create, `F` filter, `Cmd+K` command menu (context-aware, shows each command's shortcut), `G` then a key to navigate.
- Status change via `S` puts the card at the top of the target column. Mouse drag puts it where you drop it.
- `Alt+Shift+Up/Down` moves a card to the top or bottom of its column.
- `Enter` opens the issue. `Space` peeks. `Esc` closes or clears.
- Filters (`F`): typed quick search ("High", a username, a label). Operators: is / is not / is either of / includes any, all, none / before / after. Advanced filters nest AND/OR groups. Main filters live in the URL. Natural-language "filter with AI" exists.
- Right-click opens a context menu with the same actions. Undo with `Cmd+Z`.

### Jira

- Drag and drop between columns runs a workflow transition. A transition can open a screen that asks for fields.
- Board keys: `J`/`K` next/previous card, `N`/`P` next/previous column, `O` open, `A` assign, `I` assign to me, `M` comment, `C` create, `/` search, `.` or `Cmd+K` command palette, `-` toggle swimlanes, `?` shortcut help.
- Quick filters: buttons above the board, each one a JQL query. Defaults: "Only My Issues" (`assignee = currentUser()`) and "Recently Updated" (`updatedDate >= -1d`). Several can stack.
- Board search box plus avatar filter row (click an avatar to show only that person's cards).
- Density: cards stay one size. The backlog has a compact option. Linear likewise has no density toggle on boards; density comes from which properties you show.

Sources: [Linear select issues](https://linear.app/docs/select-issues), [Linear assigning](https://linear.app/docs/assigning-issues), [Linear filters](https://linear.app/docs/filters), [Linear board layout](https://linear.app/docs/board-layout), [Descript's Linear guide](https://www.linear.app/now/descript-internal-guide-for-using-linear), [Jira keyboard shortcuts](https://support.atlassian.com/jira-service-management-cloud/docs/overview-of-jira-cloud-keyboard-shortcuts/), [Jira command palette](https://support.atlassian.com/jira-software-cloud/docs/what-is-the-command-palette/), [Jira quick filters (Server doc, same defaults)](https://confluence.atlassian.com/jirasoftwareserver0821/configuring-quick-filters-1114801496.html).

## 6. Takeaways for a local-markdown board

1. Store the status category, not only the status name. Fix the category order (backlog, unstarted, started, completed, canceled). Derive icon shape from the category and color from the status.
2. Columns = statuses in workflow order. Put icon, name, count and `+` in the header. `+` writes a new file with that status in front matter.
3. Offer "hide column" (hidden columns collapse into a strip at the right, still droppable) and "show empty groups".
4. Group by any front-matter field; sub-group as collapsible swimlanes. Dropping a card writes the group values back to the file.
5. Card = ID + avatar row, icon + title, then a chip row. Let each chip toggle in a display-options panel. Never show the body on the card.
6. Add a peek panel on `Space` with `Up`/`Down` walking. `Enter` opens the full view. `Esc` backs out.
7. Full view: title and markdown body in the main column, properties in a right sidebar, relations (blocked by / blocks / related / duplicate) and sub-issue progress there too.
8. Activity for local files = git log of the file (history) plus a comments section in the markdown. Merge them in one feed, with a filter like Jira's All / Comments / History.
9. Keyboard first: `J`/`K`/arrows, `H`/`L` or `N`/`P` across columns, `X` select, `S`/`P`/`A`/`L` pickers, `Cmd+K` command menu that lists shortcuts, `Alt+Shift+Up/Down` to top or bottom.
10. Keep manual order in a front-matter rank field (fractional index) so a move rewrites one file.
11. Borrow Jira's visual-only WIP limits (red over max, yellow under min) and "days in column" dots. Compute days from git history of the status field.
12. Due-date chip colors: red for overdue or today, orange within 7 days, grey otherwise.
13. Show a blocked marker on the card when any "blocked by" target is not in a completed or canceled category.
14. Copy ID, copy file path and copy branch name (`id-slug-title`) as first-class commands.
