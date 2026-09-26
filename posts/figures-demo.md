---
title: Figures and Diagrams
date: 2026-09-26
tags: [meta, markdown]
summary: How images and mermaid diagrams render inside posts.
---

Posts can carry visual blocks. A visual block sits in the center of the
article. Text flows above and below it. A title shows below the visual.

## Images

An image with a title renders as a centered figure. The caption comes
from the markdown link title.

![A three-tier architecture sketch](images/demo-architecture.png "Figure 1: A simple three-tier architecture")

An image without a title renders centered, with no caption.

![A three-tier architecture sketch](images/demo-architecture.png)

## Mermaid diagrams

A fenced block tagged `mermaid` renders to SVG at build time. The words
after `mermaid` in the info string become the caption. Diagrams follow
the site theme.

```mermaid Figure 2: The request path through the generator
flowchart LR
    A[Markdown post] --> B[pulldown-cmark]
    B --> C{mermaid block?}
    C -- yes --> D[Render SVG]
    C -- no --> E[Highlight code]
    D --> F[HTML page]
    E --> F
```

```mermaid Figure 3: A sequence of a build
sequenceDiagram
    participant U as Author
    participant S as sblog
    participant B as Browser
    U->>S: sblog --full
    S->>S: Render diagrams to SVG
    S->>B: Static HTML
    B->>B: No JavaScript runs
```

## Text around figures

Text above and below a figure flows normally. The figure stays centered
between the paragraphs. Long captions wrap inside the figure width.
