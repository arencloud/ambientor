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
