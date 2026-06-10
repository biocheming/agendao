---
name: folder-inspector
description: Structured folder purpose analysis with concise evidence.
---

# Folder Inspector Skill

文档基线：v2026.6.10（更新日期：2026-06-10）

When user asks what a folder does, follow this workflow:

1. Run `ls` for top-level files.
2. Pick 2-5 representative files and run `read`.
3. Summarize:
- Main purpose
- Key files and roles
- Next actionable steps

Additional guidance (v2026.6.10):

- Prefer representative small / medium files first; avoid reading multiple large binaries in one turn.
- If a binary file is needed, consume attachment metadata instead of inlining full payload text.
- If the answer depends on a real user choice, prefer a structured question flow over burying the question in a long prose block.

Output constraints:

- Keep summary concise.
- Avoid dumping full file contents.
- Prefer bullet points with file references.
