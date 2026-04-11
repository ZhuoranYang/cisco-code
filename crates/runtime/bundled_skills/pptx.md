---
name: pptx
description: Generate PowerPoint presentations from structured content
user-invocable: true
---

Generate a .pptx presentation from the user's content or outline.

Steps:
1. Gather the user's content, outline, or topic. Clarify slide count and structure if unclear.
2. Create a markdown file using `---` as slide separators and `#` for slide titles. Example:

```
---
title: "Presentation Title"
author: "Author Name"
date: "2025-01-01"
---

# Slide Title

- Bullet one
- Bullet two

---

# Another Slide

Content here
```

3. Convert to .pptx using pandoc (preferred):
   ```
   pandoc slides.md -t pptx -o output.pptx
   ```
   If a template is specified, add `--reference-doc=template.pptx`.

4. If pandoc is not available, generate a Python script using the `python-pptx` library as fallback. The script should create slides programmatically with proper layouts, titles, and bullet content. Run it with `python3`.

5. Handle these options when provided:
   - Theme/template: pass as `--reference-doc` (pandoc) or apply in python-pptx
   - Slide dimensions: set via python-pptx `prs.slide_width`/`prs.slide_height`; pandoc uses template dimensions
   - Title slide metadata: title, subtitle, author, date

6. Save the .pptx to the user's working directory or their specified path. Clean up intermediate files.
