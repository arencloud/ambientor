# Git and pull request conventions

## Commits

- Use only the repository's configured local git user (`user.name` / `user.email` from `.git/config` or local overrides).
- Never add `Co-authored-by`, agent `Signed-off-by`, or other co-author trailers to commit messages.
- Never list agents or AI tools as authors or co-authors in commit messages.
- When amending commits, strip any existing `Co-authored-by:` lines before finishing.
- Do not mention co-author cleanup in the commit message subject or body.

## Pull requests

- Keep titles and bodies limited to technical summary and test plan.
- Do not append product branding or attribution footers to PR descriptions.
- Do not add agents or AI tools as co-authors or contributors in the PR body.

## Branches

- Use descriptive names such as `feature/<topic>` or `<topic>` — do not prefix branches with IDE or tooling vendor names.

## Why `Co-authored-by: Cursor` still appears

The agent’s `git commit -m "..."` text does **not** include that trailer. **Cursor injects it** when it runs git on your machine (Commit Attribution), and may append **“Made with Cursor”** to PR bodies (PR Attribution). That is product behavior, not the commit message in the chat.

Turn it off:

1. **Cursor Settings → Agents → Attribution** — disable **Commit Attribution** and **PR Attribution**, then restart Cursor.
2. **CLI** — in `~/.cursor/cli-config.json` set `"commitAttribution": false` and `"prAttribution": false`.
3. **Repo hook (recommended)** — from the repository root:

```bash
git config core.hooksPath scripts/git-hooks
chmod +x scripts/git-hooks/prepare-commit-msg
```

`scripts/git-hooks/prepare-commit-msg` removes `Co-authored-by` / `Made-with` lines that reference the agent before each commit.

Squash-merged PRs on GitHub can still copy attribution from branch commits into the merge commit; disable attribution before pushing feature branches, or edit the squash message on merge.
