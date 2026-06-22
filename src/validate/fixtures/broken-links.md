---
type: Fixture
title: Broken Links Fixture
description: A fixture with valid frontmatter, one broken relative link, one valid link, and one external URL.
---

# Broken Links Fixture

This file has valid OKF frontmatter. It contains:

- A valid relative link: [good fixture](good.md)
- An external URL (must not be flagged): [Anthropic](https://anthropic.com)
- A pure anchor (must not be flagged): [Section](#section)
- A broken relative link (must be flagged): [Missing File](nonexistent-file.md)

## Section

Content here.
