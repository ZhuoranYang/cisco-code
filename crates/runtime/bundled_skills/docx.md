---
name: docx
description: Generate Word documents from markdown content
user-invocable: true
---

Create a Word document (.docx) from user-provided content or markdown.

Follow these steps:

1. Gather the user's content, structure, and any options (title, author, table of contents, reference template).
2. Write a temporary markdown file with the content. Add a YAML metadata block if title/author are specified:
   ```
   ---
   title: "Document Title"
   author: "Author Name"
   ---
   ```
3. Attempt conversion with pandoc:
   ```
   pandoc input.md -o output.docx
   ```
   - Add `--toc` if table of contents is requested.
   - Add `--reference-doc=template.docx` if the user provides a reference template.
4. If pandoc is not installed, fall back: generate a standalone HTML file from the markdown and inform the user they can open it in Word or install pandoc (`brew install pandoc` / `apt install pandoc`).
5. Output the .docx (or .html fallback) to the user's current working directory, or to a path they specify.
6. Confirm the output file path and size when done.
