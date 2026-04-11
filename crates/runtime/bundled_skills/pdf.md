---
name: pdf
description: Generate PDF documents from markdown content
user-invocable: true
---

Generate a PDF document from the user's content or request. Support common options: title, author, date, page size (letter/A4), and margins.

Steps:
1. Gather the user's content, topic, or instructions for the PDF
2. Create a markdown file with YAML frontmatter for metadata:
   ```yaml
   ---
   title: "Document Title"
   author: "Author Name"
   date: "2024-01-01"
   geometry: "margin=1in"
   papersize: letter
   ---
   ```
   Adjust `geometry` and `papersize` per user preferences (e.g. `a4paper`, `margin=2cm`).
3. Write the full markdown content to a `.md` file in the target output directory
4. Check if `pandoc` is available by running `which pandoc`
5. If pandoc is available:
   - Convert with: `pandoc input.md -o output.pdf --pdf-engine=xelatex` (or `pdflatex`/`tectonic` as fallback engines)
   - If the LaTeX engine is missing, try: `pandoc input.md -o output.pdf --pdf-engine=weasyprint` or `pandoc input.md -t html5 -o output.pdf`
6. If pandoc is not available:
   - Create an HTML file with equivalent styling (use CSS for margins, page size via `@page`)
   - Inform the user: "Pandoc is not installed. An HTML file has been created at `<path>`. Open it in a browser and print/save as PDF."
   - Suggest installing pandoc: `brew install pandoc` (macOS) or `apt install pandoc` (Linux)
7. Output the PDF (or fallback HTML) to the current working directory unless the user specifies a different path
8. Report the output file path to the user
