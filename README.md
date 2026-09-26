# sblog

A tiny static blog generator written in Rust. Write posts in Markdown with
YAML frontmatter. sblog renders them into plain HTML and CSS. Serve the
output with any static host.

## Features

- **Markdown posts** with YAML frontmatter (`title`, `date`, `tags`, `summary`)
- **Tera templates** for full control over page layout
- **Syntax highlighting** for code blocks (via syntect)
- **Figures** — centered images with captions, and mermaid diagrams rendered to SVG at build time
- **Tag pages** generated automatically from post frontmatter
- **Open Graph & Twitter Card** meta tags
- **robots.txt & sitemap.xml** generated automatically for search engines and agents
- **RSS 2.0 feed** (`feed.xml`) generated automatically for subscribers
- **Reading time** estimated per post
- **Full rebuild** with orphan cleanup
- **Static asset copying** — CSS, images, and more

---

## Quickstart

### 1. Prerequisites

- Rust toolchain (edition 2024)
- Cargo

### 2. Install the binary

Install the latest release from crates.io:

```bash
cargo install sblog
```

The binary is `sblog`. It is available on your `PATH` after install.

### 3. Set up your project structure

Create a directory for your blog with this layout:

```
my-blog/
├── config.toml          # Site configuration
├── posts/               # Markdown posts
│   └── hello-world.md
├── static/              # CSS, images, assets
│   ├── style.css
│   └── images/
├── templates/           # Tera HTML templates
│   ├── base.html
│   ├── index.html
│   ├── post.html
│   ├── archive.html
│   ├── posts.html
│   ├── tag.html
│   ├── tags.html
│   └── about.html
└── public/              # Generated output (created by the build)
```

Copy the [`templates/`](https://github.com/santanusinha/sblog/tree/master/templates) and
[`static/`](https://github.com/santanusinha/sblog/tree/master/static) directories from the
[GitHub repository](https://github.com/santanusinha/sblog), or write your own. Copy
[`config.toml`](https://github.com/santanusinha/sblog/blob/master/config.toml) and edit it to
match your site.

### 4. Configure `config.toml`

```toml
title = "My Blog"
tagline = "A tiny static blog"
base_url = "https://example.com"
og_image = "https://example.com/og-image.png"
twitter_handle = "myhandle"

# Paths (relative to the project root).
posts_dir = "posts"
output_dir = "public"
templates_dir = "templates"
static_dir = "static"
css_file = "static/style.css"

[header]
title = "My Blog"
tagline = "A tiny static blog"
```

| Key | Description | Default |
|-----|-------------|---------|
| `title` | Site title, used in `<title>` and footer | `sblog` |
| `tagline` | Short site description | `A tiny static blog` |
| `base_url` | Absolute URL for social meta tags and sitemap | *(empty)* |
| `og_image` | Absolute URL of the Open Graph image | *(empty)* |
| `twitter_handle` | Twitter handle (without `@`) for card meta | *(empty)* |
| `posts_dir` | Directory holding Markdown posts | `posts` |
| `output_dir` | Directory where generated pages are written | `public` |
| `templates_dir` | Directory holding Tera templates | `templates` |
| `static_dir` | Directory holding static assets | `static` |
| `css_file` | Path to the stylesheet (relative to static dir) | `static/style.css` |
| `[header].title` | Title shown in the site masthead | `sblog` |
| `[header].tagline` | Tagline shown in the site masthead | `A tiny static blog` |

### 5. Write your first post

Create `posts/hello-world.md`:

```markdown
---
title: Hello, world
date: 2026-08-15
tags: [meta, rust]
summary: My first post on this blog.
---

This is my first post.

## A section

Some **bold** text and a code block:

```rust
fn main() {
    println!("Hello, world!");
}
```
```

**Frontmatter fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `title` | Yes | Post title, shown in cards, lists, and the browser tab |
| `date` | Yes | Post date in `YYYY-MM-DD` format. Controls sort order (newest first) |
| `tags` | No | YAML list of tags. Generates tag pages automatically |
| `summary` | No | Short description shown on cards and list pages |

The **filename** becomes the URL slug. `hello-world.md` → `/post/hello-world.html`.

### 6. Build the site

```bash
# Full rebuild (all posts, removes orphaned output)
sblog --full

# Incremental build (only changed posts)
sblog
```

Output goes to `public/`.

### 7. Serve locally

```bash
python3 -m http.server 8123 --directory public
```

Open `http://localhost:8123/`.

### 8. Deploy

Upload the `public/` directory to any static host:

- Nginx
- GitHub Pages
- Netlify
- Vercel
- Cloudflare Pages
- Any S3-compatible bucket

---

## Markdown Support

sblog uses `pulldown-cmark` with these extensions enabled:

| Feature | Syntax | Example |
|---------|--------|---------|
| Tables | Pipe-delimited rows | `\| A \| B \|` |
| Footnotes | `[^1]` references | `Text[^1]` |
| Strikethrough | `~~text~~` | `~~done~~` |
| Task lists | `- [ ]` / `- [x]` | `- [x] done` |
| Smart punctuation | Auto-converted quotes/dashes | `"quotes"` → `"quotes"` |
| Fenced code blocks | Triple backticks with language | ` ```rust ` |
| Syntax highlighting | Via syntect, `syn-` prefixed classes | ` ```python ` |

### Images and diagrams

sblog wraps images in `<figure>` elements and renders mermaid diagrams to
SVG at build time. No JavaScript runs on the generated pages.

**Images.** A paragraph that holds a single image becomes a centered
figure. The optional link title becomes the caption:

```markdown
![Alt text for the image](images/photo.png "Figure 1: The system layout")
```

- With a title: centered figure with a caption below the image.
- Without a title: centered figure with no caption.
- Image paths are relative to the `static/` directory.

**Mermaid diagrams.** A fenced code block with the `mermaid` info string
renders to inline SVG during the build. The words after `mermaid` in the
info string become the caption:

````markdown
```mermaid Figure 2: The request path
flowchart LR
    A[Markdown post] --> B[HTML page]
```
````

- The renderer uses `mermaid-rs-renderer` at build time. The output is
  plain SVG inside the HTML.
- Diagram colors follow the site theme (light background, dark text,
  accent blue).
- Supported diagram types include flowchart, sequence, class, state,
  ER, gantt, and pie.
- A block with invalid mermaid syntax still builds. The renderer emits
  a partial SVG, and the build does not fail.

See `posts/figures-demo.md` in the repository for a working example.

---

## Template System

Templates use the **Tera** templating engine (Jinja2/Django-style syntax).
All templates extend `base.html` and override its blocks.

### Template Files

| File | Page | Purpose |
|------|------|---------|
| `base.html` | All pages | Layout shell: header, nav, footer, blocks |
| `index.html` | `/` | Home page: featured card + card grid + Topics sidebar |
| `post.html` | `/post/<slug>.html` | Individual post with prev/next nav and sidebar |
| `archive.html` | `/archive.html` | Full post list grouped by date |
| `posts.html` | `/posts.html` | Simple post list (title + date only) |
| `tag.html` | `/tags/<tag>.html` | Posts filtered by a single tag |
| `tags.html` | `/tags/index.html` | Index of all tags with counts |
| `about.html` | `/about.html` | About page from `posts/about.md` |

### Blocks in `base.html`

| Block | Purpose | Default |
|-------|---------|---------|
| `title` | Browser tab title | `{{ site.title }}` |
| `description` | `<meta name="description">` | `{{ site.tagline }}` |
| `social_meta` | Open Graph / Twitter meta tags | *(empty)* |
| `content` | Main page body | *(empty)* |

---

## Template Context Variables

The following variables are available in every template. They are injected
by the Rust generator at render time.

### Global Variables (all pages)

| Variable | Type | Description |
|----------|------|-------------|
| `site.title` | string | Site title from config |
| `site.tagline` | string | Site tagline from config |
| `site.header_title` | string | Header title from config |
| `site.header_tagline` | string | Header tagline from config |
| `title` | string | Page title for the browser tab |
| `description` | string | Page description for meta tags |
| `social_meta` | string (HTML) | Pre-rendered Open Graph / Twitter meta tags |
| `header_title` | string | Header title shown in masthead |
| `header_tagline` | string | Header tagline shown in masthead |
| `year` | string | Year for the footer copyright |
| `css_href` | string | Path to the stylesheet (relative to current page) |
| `home_href` | string | Path to the home page (relative) |
| `archive_href` | string | Path to the archive page (relative) |
| `about_href` | string | Path to the about page (relative) |
| `feed_href` | string | Path to the RSS feed (relative) |
| `active_nav` | string | Active nav item: `latest`, `archive`, or `about` |
| `active_nav` | string | Active nav item: `latest`, `archive`, or `about` |

### Post Object (`post`)

Available on the **post page** and **about page**.

| Field | Type | Description |
|-------|------|-------------|
| `post.title` | string | Post title |
| `post.summary` | string | Post summary from frontmatter |
| `post.href` | string | Relative link to the post |
| `post.date_long` | string | Long date, e.g. `August 15, 2026` |
| `post.date_short` | string | Short date, e.g. `Aug 15, 2026` |
| `post.read_time` | string | Estimated reading time, e.g. `3 min read` |
| `post.tags` | array | List of `TagView` objects |
| `post.body_html` | string (HTML) | Rendered Markdown body (use `\| safe`) |

### Posts List (`posts`)

Available on the **index**, **archive**, **posts**, and **tag** pages.

An array of `PostView` objects. Each item has the same fields as `post`
above, except `body_html` is omitted on list pages.

### Tag Object (`tag`)

Available on the **tag page**.

| Field | Type | Description |
|-------|------|-------------|
| `tag.name` | string | Tag name |
| `tag.href` | string | Relative link to the tag page |
| `tag.count` | integer | Number of posts with this tag |

### Tags List (`tags`)

Available on the **index**, **archive**, and **tags** pages.

An array of `TagView` objects with `name`, `href`, and `count`.

### Prev / Next Navigation

Available on the **post page**.

| Variable | Type | Description |
|----------|------|-------------|
| `prev_post` | PostView \| null | The newer post (previous in sort order) |
| `next_post` | PostView \| null | The older post (next in sort order) |

### Sidebar (`sidebar`)

Available on the **index**, **post**, and **about** pages.

| Field | Type | Description |
|-------|------|-------------|
| `sidebar.older_count` | integer | Number of essays listed |
| `sidebar.essays` | array | Up to 5 recent posts (PostView objects) |
| `sidebar.tags` | array | All tags with counts (TagView objects) |
| `sidebar.archive_href` | string | Relative link to the archive page |

---

## Template Elements by Logical Grouping

### 1. Layout & Navigation

| Element | Template | Variable(s) | Description |
|---------|----------|-------------|-------------|
| Site header | `base.html` | `site.header_title`, `site.header_tagline` | Masthead with title and tagline |
| Nav links | `base.html` | `home_href`, `archive_href`, `about_href`, `active_nav` | Header navigation (Home, Archive, About) |
| Footer | `base.html` | `year`, `site.title`, `home_href`, `archive_href`, `about_href` | Footer with brand, links, copyright |
| Main content block | `base.html` | `content` | The block each page overrides |
| Social meta block | `base.html` | `social_meta` | Open Graph / Twitter meta tags |
| RSS feed link | `base.html` | `feed_href` | `<link rel="alternate">` for the RSS feed |
| Meta description block | `base.html` | `description` | SEO description |
| Social meta block | `base.html` | `social_meta` | Open Graph / Twitter meta tags |

### 2. Home Page (index.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Featured card | `posts[0]` | The latest post as a large hero card |
| Card grid | `posts` (index 1+) | Older posts as a responsive grid of cards |
| Topics sidebar | `tags`, `tags_href` | All tags with post counts |
| All posts link | `archive_href` | Link to the full archive (shown when >10 posts) |

### 3. Post Page (post.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Post header | `post.title`, `post.date_long`, `post.read_time`, `post.summary` | Title, date, reading time, subtitle |
| Article body | `post.body_html` | Rendered Markdown content |
| Prev/Next nav | `prev_post`, `next_post` | Newer / older post links |
| Older sidebar | `sidebar.essays`, `sidebar.archive_href` | Recent posts list |
| Tags sidebar | `post.tags`, `tags_href` | Post tags as chips |

### 4. Archive Page (archive.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Page header | `posts \| length` | Title and post count |
| Post list | `posts` | Full list with date, title, summary, tags |
| Tags sidebar | `tags` | All tags with counts |

### 5. Posts List Page (posts.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Page header | `posts \| length` | Title and article count |
| Post list | `posts` | Simple list: date + title only |

### 6. Tag Page (tag.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Page header | `tag.name`, `tag.count` | Tag name and post count |
| Post list | `posts` | Posts with this tag: date, title, summary, tags |

### 7. Tags Index Page (tags.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Page header | `tags \| length` | Title and tag count |
| Tag list | `tags` | All tags with names and post counts |

### 8. About Page (about.html)

| Element | Variable(s) | Description |
|---------|-------------|-------------|
| Portrait (mobile) | `images/profile.png` | Profile image shown above content on mobile |
| About content | `post.body_html` | Rendered Markdown from `posts/about.md` |
| Portrait (sidebar) | `images/profile.png` | Profile image in the sidebar on desktop |
| Elsewhere links | Hardcoded URLs | GitHub, LinkedIn, Twitter links |
| Recent sidebar | `sidebar.essays`, `sidebar.archive_href` | Recent posts list |

---

## Build Modes

### Full build (`--full`)

```bash
sblog --full
```

- Rebuilds **every** page
- Removes orphaned post pages (source deleted)
- Removes orphaned tag pages (tag removed from all posts)
- Removes old root-level post pages

### Incremental build (default)

```bash
sblog
```
- Regenerates `robots.txt`, `sitemap.xml`, and `feed.xml` when the config or any post changed
- Rebuilds only posts whose source is newer than their output
- Rebuilds aggregate pages (index, archive, posts) when any post changed
- Regenerates `robots.txt` and `sitemap.xml` when the config or any post changed
- Copies static assets every time (idempotent)

---

## Output Structure

```
public/
├── index.html          # Home page
├── archive.html        # All posts
├── posts.html          # Simple post list
├── about.html          # About page
├── post/               # Individual post pages
│   ├── hello-world.html
│   ├── my-other-post.html
│   └── ...
├── tags/               # Tag pages
│   ├── index.html      # Tag index
│   ├── meta.html
├── robots.txt           # Crawler rules (generated)
├── sitemap.xml          # URL list for search engines (generated)
├── feed.xml             # RSS 2.0 feed (generated)
└── images/             # Static images (copied from static/)
├── robots.txt           # Crawler rules (generated)
├── sitemap.xml          # URL list for search engines (generated)
└── images/             # Static images (copied from static/)
```

---

## License

MIT
