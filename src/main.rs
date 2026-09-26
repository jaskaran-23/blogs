use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::NaiveDate;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_yaml::Value;
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use tera::{Context, Tera};

use crate::models::{
    Document, HeadingView, Post, PostView, SidebarView, SiteConfig, SiteView, TagView,
};

mod models;

fn main() {
    let root = std::env::current_dir().expect("get current directory");
    let full = std::env::args().any(|a| a == "--full");
    let config = load_config(&root);
    if full {
        build_site(&root, &config);
    } else {
        build_stale(&root, &config);
    }
}

/// Build the whole site. Rebuild every page and remove orphaned output files
/// whose source markdown no longer exists.
fn build_site(root: &Path, config: &SiteConfig) -> Vec<Post> {
    let posts_dir = root.join(&config.posts_dir);
    let public_dir = root.join(&config.output_dir);

    let mut posts = read_posts(&posts_dir);
    if posts.is_empty() {
        eprintln!("No posts found in {}", posts_dir.display());
        std::process::exit(1);
    }
    posts.sort_by_key(|p| std::cmp::Reverse(p.date));

    fs::create_dir_all(&public_dir).expect("create output dir");

    // Load the templates.
    let templates_dir = root.join(&config.templates_dir);
    let mut tera = Tera::default();
    tera.load_from_glob(&format!("{}/**/*.html", templates_dir.display()))
        .expect("load templates");

    // Copy the static assets (including the stylesheet) to the output.
    copy_static(root, config);

    // Build each post page into the post/ subdirectory.
    let post_dir = public_dir.join("post");
    fs::create_dir_all(&post_dir).expect("create post dir");
    for post in &posts {
        let doc = document_for_post(config, post, &posts);
        let html = render(&tera, "post.html", &doc);
        let path = post_dir.join(format!("{}.html", post.slug));
        fs::write(&path, html).expect("write post page");
    }

    // Build the index page.
    let doc = document_for_index(config, &posts);
    let html = render(&tera, "index.html", &doc);
    fs::write(public_dir.join("index.html"), html).expect("write index.html");

    // Build the archive page.
    let doc = document_for_archive(config, &posts);
    let html = render(&tera, "archive.html", &doc);
    fs::write(public_dir.join("archive.html"), html).expect("write archive.html");

    // Build the simple posts list page.
    let doc = document_for_posts(config, &posts);
    let html = render(&tera, "posts.html", &doc);
    fs::write(public_dir.join("posts.html"), html).expect("write posts.html");

    // Build the about page.
    let doc = document_for_about(root, config, &posts);
    let html = render(&tera, "about.html", &doc);
    fs::write(public_dir.join("about.html"), html).expect("write about.html");

    // Build the tag pages and the tag index.
    let tags = collect_tags(&posts);
    let tags_dir = public_dir.join("tags");
    fs::create_dir_all(&tags_dir).expect("create tags dir");
    for (tag, tag_posts) in &tags {
        let doc = document_for_tag(config, tag, tag_posts, &posts);
        let html = render(&tera, "tag.html", &doc);
        let path = tags_dir.join(format!("{}.html", slugify(tag)));
        fs::write(&path, html).expect("write tag page");
    }
    let doc = document_for_tag_index(config, &tags);
    let html = render(&tera, "tags.html", &doc);
    fs::write(tags_dir.join("index.html"), html).expect("write tag index");

    // Write the SEO files (robots.txt and sitemap.xml).
    write_seo_files(config, &posts, &tags, &public_dir);

    // Remove old root-level post pages (they now live in post/).
    // Aggregate pages (index, archive, posts, about) and the stylesheet are kept.
    let aggregate = ["index", "archive", "posts", "about"];
    if let Ok(entries) = fs::read_dir(&public_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if aggregate.contains(&stem.as_str()) {
                continue;
            }
            // Any other root-level html is an old post page; remove it.
            fs::remove_file(&path).expect("remove old post page");
            println!("Removed old post page {}", path.display());
        }
    }

    // Remove orphaned post pages in post/ whose source markdown no longer exists.
    let valid_slugs: std::collections::HashSet<String> =
        posts.iter().map(|p| p.slug.clone()).collect();
    if let Ok(entries) = fs::read_dir(&post_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !valid_slugs.contains(&stem) {
                fs::remove_file(&path).expect("remove orphaned post page");
                println!("Removed orphaned post page {}", path.display());
            }
        }
    }

    // Remove orphaned tag pages whose tag no longer exists.
    let valid_tags: std::collections::HashSet<String> =
        tags.iter().map(|(tag, _)| slugify(tag)).collect();
    if let Ok(entries) = fs::read_dir(&tags_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if stem == "index" {
                continue;
            }
            if !valid_tags.contains(&stem) {
                fs::remove_file(&path).expect("remove orphaned tag page");
                println!("Removed orphaned tag page {}", path.display());
            }
        }
    }

    println!(
        "Built {} posts and {} tags into {}",
        posts.len(),
        tags.len(),
        public_dir.display()
    );
    posts
}

/// Build only the pages whose source is newer than their output.
fn build_stale(root: &Path, config: &SiteConfig) -> Vec<Post> {
    let posts_dir = root.join(&config.posts_dir);
    let public_dir = root.join(&config.output_dir);

    let mut posts = read_posts(&posts_dir);
    posts.sort_by_key(|p| std::cmp::Reverse(p.date));

    fs::create_dir_all(&public_dir).expect("create output dir");

    // Copy static assets (cheap, idempotent).
    copy_static(root, config);

    let templates_dir = root.join(&config.templates_dir);
    let mut tera = Tera::default();
    tera.load_from_glob(&format!("{}/**/*.html", templates_dir.display()))
        .expect("load templates");

    let mut changed_any = false;

    // Check if config.toml changed since the last build.
    let config_path = root.join("config.toml");
    let index_path = public_dir.join("index.html");
    let config_changed = is_stale(&config_path, &index_path);
    if config_changed {
        changed_any = true;
    }

    let post_dir = public_dir.join("post");
    fs::create_dir_all(&post_dir).expect("create post dir");

    for post in &posts {
        let src = posts_dir.join(format!("{}.md", post.slug));
        let out = post_dir.join(format!("{}.html", post.slug));
        if config_changed || is_stale(&src, &out) {
            let doc = document_for_post(config, post, &posts);
            let html = render(&tera, "post.html", &doc);
            fs::write(&out, html).expect("write post page");
            changed_any = true;
        }
    }

    let index_path = public_dir.join("index.html");
    let index_stale = changed_any
        || !index_path.exists()
        || posts.iter().any(|p| {
            let src = posts_dir.join(format!("{}.md", p.slug));
            let out = post_dir.join(format!("{}.html", p.slug));
            is_stale(&src, &out)
        });
    if index_stale {
        let doc = document_for_index(config, &posts);
        let html = render(&tera, "index.html", &doc);
        fs::write(&index_path, html).expect("write index.html");
    }

    let archive_path = public_dir.join("archive.html");
    if index_stale || !archive_path.exists() {
        let doc = document_for_archive(config, &posts);
        let html = render(&tera, "archive.html", &doc);
        fs::write(&archive_path, html).expect("write archive.html");
    }

    let posts_path = public_dir.join("posts.html");
    if index_stale || !posts_path.exists() {
        let doc = document_for_posts(config, &posts);
        let html = render(&tera, "posts.html", &doc);
        fs::write(&posts_path, html).expect("write posts.html");
    }

    // Build the about page if stale or missing.
    let about_src = posts_dir.join("about.md");
    let about_path = public_dir.join("about.html");
    if config_changed || is_stale(&about_src, &about_path) || !about_path.exists() {
        let doc = document_for_about(root, config, &posts);
        let html = render(&tera, "about.html", &doc);
        fs::write(&about_path, html).expect("write about.html");
    }
    let tags = collect_tags(&posts);
    let tags_dir = public_dir.join("tags");
    fs::create_dir_all(&tags_dir).expect("create tags dir");
    let mut tag_stale = changed_any;
    for (tag, _tag_posts) in &tags {
        let path = tags_dir.join(format!("{}.html", slugify(tag)));
        if !path.exists() {
            tag_stale = true;
        }
    }
    if tag_stale || !tags_dir.join("index.html").exists() {
        for (tag, tag_posts) in &tags {
            let doc = document_for_tag(config, tag, tag_posts, &posts);
            let html = render(&tera, "tag.html", &doc);
            let path = tags_dir.join(format!("{}.html", slugify(tag)));
            fs::write(&path, html).expect("write tag page");
        }
        let doc = document_for_tag_index(config, &tags);
        let html = render(&tera, "tags.html", &doc);
        fs::write(tags_dir.join("index.html"), html).expect("write tag index");
    }

    // Write the SEO files (robots.txt, sitemap.xml, and feed.xml) when the
    // config or any post changed, or when they are missing.
    let robots_path = public_dir.join("robots.txt");
    let sitemap_path = public_dir.join("sitemap.xml");
    let feed_path = public_dir.join("feed.xml");
    if changed_any || !robots_path.exists() || !sitemap_path.exists() || !feed_path.exists() {
        write_seo_files(config, &posts, &tags, &public_dir);
    }

    println!(
        "Checked {} posts and {} tags in {}",
        posts.len(),
        tags.len(),
        public_dir.display()
    );
    posts
}

/// True if the source file is newer than the output file, or the output is missing.
fn is_stale(src: &Path, out: &Path) -> bool {
    let Ok(src_meta) = fs::metadata(src) else {
        return false;
    };
    let Ok(out_meta) = fs::metadata(out) else {
        return true;
    };
    let Ok(src_mtime) = src_meta.modified() else {
        return false;
    };
    let Ok(out_mtime) = out_meta.modified() else {
        return true;
    };
    src_mtime > out_mtime
}

/// Copy every file in the static directory to the output directory,
/// preserving the directory structure.
fn copy_static(root: &Path, config: &SiteConfig) {
    let static_dir = root.join(&config.static_dir);
    let public_dir = root.join(&config.output_dir);
    if !static_dir.exists() {
        return;
    }
    fs::create_dir_all(&public_dir).expect("create output dir");
    copy_dir_recursive(&static_dir, &public_dir);
}

/// Recursively copy the contents of `src` into `dest`, creating directories
/// as needed.
fn copy_dir_recursive(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).expect("create dest dir");
    for entry in fs::read_dir(src).expect("read static dir") {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let name = path.file_name().unwrap_or_default();
        let target = dest.join(name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target);
        } else if path.is_file() {
            fs::copy(&path, &target).expect("copy static asset");
        }
    }
}

/// Load `config.toml` from the project root. Missing file uses defaults.
fn load_config(root: &Path) -> SiteConfig {
    let path = root.join("config.toml");
    let Ok(text) = fs::read_to_string(&path) else {
        return SiteConfig::default();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return SiteConfig::default();
    };
    let mut config = SiteConfig::default();
    if let Some(title) = value.get("title").and_then(|v| v.as_str()) {
        config.title = title.to_string();
    }
    if let Some(tagline) = value.get("tagline").and_then(|v| v.as_str()) {
        config.tagline = tagline.to_string();
    }
    if let Some(header) = value.get("header") {
        if let Some(header_title) = header.get("title").and_then(|v| v.as_str()) {
            config.header_title = header_title.to_string();
        }
        if let Some(header_tagline) = header.get("tagline").and_then(|v| v.as_str()) {
            config.header_tagline = header_tagline.to_string();
        }
    }
    if let Some(base_url) = value.get("base_url").and_then(|v| v.as_str()) {
        config.base_url = base_url.trim_end_matches('/').to_string();
    }
    if let Some(og_image) = value.get("og_image").and_then(|v| v.as_str()) {
        config.og_image = og_image.to_string();
    }
    if let Some(twitter_handle) = value.get("twitter_handle").and_then(|v| v.as_str()) {
        config.twitter_handle = twitter_handle.to_string();
    }
    if let Some(posts_dir) = value.get("posts_dir").and_then(|v| v.as_str()) {
        config.posts_dir = posts_dir.to_string();
    }
    if let Some(output_dir) = value.get("output_dir").and_then(|v| v.as_str()) {
        config.output_dir = output_dir.to_string();
    }
    if let Some(templates_dir) = value.get("templates_dir").and_then(|v| v.as_str()) {
        config.templates_dir = templates_dir.to_string();
    }
    if let Some(static_dir) = value.get("static_dir").and_then(|v| v.as_str()) {
        config.static_dir = static_dir.to_string();
    }
    if let Some(css_file) = value.get("css_file").and_then(|v| v.as_str()) {
        config.css_file = css_file.to_string();
    }
    config
}

/// Read every `*.md` file in the posts directory and parse it.
fn read_posts(dir: &Path) -> Vec<Post> {
    let mut posts = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("read posts dir")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        // The about page is a special page, not a regular post.
        if path.file_stem().and_then(|s| s.to_str()) == Some("about") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read post file");
        match parse_post(&path, &text) {
            Ok(post) => posts.push(post),
            Err(err) => eprintln!("Skipping {}: {}", path.display(), err),
        }
    }
    posts
}

/// Parse frontmatter and body from a markdown file into a `Post`.
fn parse_post(path: &Path, text: &str) -> Result<Post, String> {
    let (frontmatter, body) = split_frontmatter(text)?;
    let meta: Value =
        serde_yaml::from_str(frontmatter).map_err(|e| format!("invalid frontmatter: {e}"))?;

    let title = get_str(&meta, "title").ok_or("missing title")?;
    let date_str = get_str(&meta, "date").ok_or("missing date")?;
    let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map_err(|e| format!("invalid date {date_str:?}: {e}"))?;
    let summary = get_str(&meta, "summary").unwrap_or_default();
    let tags = get_tags(&meta);

    let slug = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("post")
        .to_string();

    let body_html = render_markdown(body, "../");
    let body_html = anchorize_headings(&body_html);
    let body_html = externalize_links(&body_html);
    let headings = extract_headings(body);

    Ok(Post {
        title,
        date,
        tags,
        summary,
        slug,
        body_html,
        headings,
    })
}

/// Extract the headings from a markdown body for the outline.
///
/// The outline lists every heading in document order. It uses the heading
/// text and level. The post title comes from frontmatter, not from the
/// body, so H1 headings in the body are valid section headings.
fn extract_headings(text: &str) -> Vec<HeadingView> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(text, options);
    let mut headings = Vec::new();
    let mut current: Option<(u8, String)> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some((level as u8, String::new()));
            }
            Event::Text(t) if current.is_some() => {
                if let Some((_, text)) = &mut current {
                    text.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = current.take() {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        let id = slugify_heading(&trimmed);
                        headings.push(HeadingView {
                            text: trimmed,
                            level,
                            id,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    headings
}

/// Turn heading text into an anchor ID. Lowercase, replace spaces with
/// hyphens, strip non-alphanumeric characters.
fn slugify_heading(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c.is_whitespace() || c == '-' || c == '_' {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push_str("heading");
    }
    out
}

/// Post-process HTML to add anchor IDs to heading tags (<h1> through <h6>).
/// The ID is generated from the heading text using slugify_heading.
/// This makes the outline sidebar items linkable to specific sections.
fn anchorize_headings(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(pos) = rest.find("<h") {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];

        let bytes = rest.as_bytes();
        let is_heading =
            bytes.len() > 3 && bytes[2] >= b'1' && bytes[2] <= b'6' && bytes[3] == b'>';
        if !is_heading {
            out.push_str(&rest[..1]);
            rest = &rest[1..];
            continue;
        }

        let level = bytes[2] as char;
        rest = &rest[4..];

        let close_tag = format!("</h{level}>");
        if let Some(end) = rest.find(&close_tag) {
            let inner = &rest[..end];
            // Strip inline HTML tags from inner text for clean ID.
            let text = inner
                .replace("<em>", "")
                .replace("</em>", "")
                .replace("<strong>", "")
                .replace("</strong>", "")
                .replace("<code>", "")
                .replace("</code>", "")
                .replace("<a href", "")
                .replace("</a>", "");
            let text_only = text
                .replace('<', " <")
                .split('>')
                .filter(|s| !s.starts_with(' ') && !s.starts_with('/'))
                .collect::<Vec<_>>()
                .join(" ");
            let id = slugify_heading(text_only.trim());
            out.push_str("<h");
            out.push(level);
            out.push_str(" id=\"");
            out.push_str(&id);
            out.push_str("\">");
            out.push_str(inner);
            out.push_str(&close_tag);
            rest = &rest[end + close_tag.len()..];
        } else {
            out.push_str(&rest[..4]);
            rest = &rest[4..];
        }
    }
    out.push_str(rest);
    out
}

/// Split a file into its YAML frontmatter and markdown body.
fn split_frontmatter(text: &str) -> Result<(&str, &str), String> {
    let rest = text
        .strip_prefix("---")
        .ok_or("missing frontmatter delimiter")?;
    let end = rest
        .find("\n---")
        .ok_or("missing closing frontmatter delimiter")?;
    let frontmatter = &rest[..end];
    let body = &rest[end + 4..];
    Ok((frontmatter, body))
}

/// Get a string field from the frontmatter mapping.
fn get_str(meta: &Value, key: &str) -> Option<String> {
    meta.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Get the tags list from the frontmatter.
fn get_tags(meta: &Value) -> Vec<String> {
    meta.get("tags")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Render markdown to HTML with syntax highlighting for fenced code blocks.
///
/// Images render as centered `<figure>` blocks with an optional caption
/// from the markdown link title. Fenced blocks tagged `mermaid` render to
/// inline SVG at build time, with the rest of the info string as caption.
/// `depth` prefixes image sources, e.g. `"../"` for pages under `/post/`.
fn render_markdown(text: &str, depth: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(text, options);
    let mut html = String::new();
    let mut code_lang: Option<String> = None;
    let mut code_lines: Vec<String> = Vec::new();
    let mut image: Option<ImageInfo> = None;
    let mut mermaid_cache: HashMap<String, String> = HashMap::new();
    // Holds the HTML of a paragraph until it closes. If the paragraph
    // holds only an image, its HTML is a figure and must not be wrapped
    // in `<p>`, which would be invalid HTML.
    let mut paragraph: Option<String> = None;

    let syntax_set = SyntaxSet::load_defaults_newlines();

    for event in parser {
        match event {
            Event::Start(Tag::Paragraph) => {
                paragraph = Some(String::new());
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(buf) = paragraph.take() {
                    // A lone figure renders as-is. Anything else keeps the
                    // default `<p>` wrapper.
                    if buf.starts_with("<figure") || buf.starts_with("<p class=") {
                        html.push_str(&buf);
                    } else {
                        html.push_str("<p>");
                        html.push_str(&buf);
                        html.push_str("</p>\n");
                    }
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                code_lang = Some(lang.to_string());
                code_lines.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                code_lang = None;
                code_lines.clear();
            }
            Event::Text(t) if code_lang.is_some() => {
                code_lines.push(t.to_string());
            }
            Event::End(TagEnd::CodeBlock) => {
                let info = code_lang.take().unwrap_or_default();
                let code = code_lines.join("");
                let (lang, title) = split_info_string(&info);
                if lang == "mermaid" {
                    let figure = render_mermaid_figure(&code, &title, &mut mermaid_cache);
                    push_to_target(&mut html, &mut paragraph, &figure);
                } else {
                    let code_html = highlight_code(&code, &lang, &syntax_set);
                    push_to_target(&mut html, &mut paragraph, &code_html);
                }
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                image = Some(ImageInfo {
                    src: dest_url.to_string(),
                    title: title.to_string(),
                    alt: String::new(),
                });
            }
            Event::Text(t) if image.is_some() => {
                if let Some(img) = image.as_mut() {
                    img.alt.push_str(&t);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(img) = image.take() {
                    let figure = render_image_figure(&img, depth);
                    push_to_target(&mut html, &mut paragraph, &figure);
                }
            }
            other => {
                let mut buf = String::new();
                pulldown_cmark::html::push_html(&mut buf, std::iter::once(other));
                push_to_target(&mut html, &mut paragraph, &buf);
            }
        }
    }

    html
}

/// Push rendered HTML either into the open paragraph buffer or straight
/// into the page HTML, depending on whether a paragraph is open.
fn push_to_target(html: &mut String, paragraph: &mut Option<String>, piece: &str) {
    match paragraph {
        Some(buf) => buf.push_str(piece),
        None => html.push_str(piece),
    }
}

/// An image collected from markdown events, shaped for `render_image_figure`.
struct ImageInfo {
    src: String,
    title: String,
    alt: String,
}

/// Render an image as a centered figure with an optional caption.
///
/// The markdown link title becomes the caption. Without a title, the image
/// renders alone in a plain centered paragraph. `depth` prefixes the source
/// so the URL resolves from the page that holds the figure.
fn render_image_figure(img: &ImageInfo, depth: &str) -> String {
    // Absolute URLs and root-relative paths pass through unchanged.
    let src = if img.src.starts_with("http://")
        || img.src.starts_with("https://")
        || img.src.starts_with('/')
    {
        img.src.clone()
    } else {
        format!("{depth}{}", img.src)
    };
    let src = escape_html(&src);
    let alt = escape_html(&img.alt);
    if img.title.is_empty() {
        return format!("<p class=\"figure-plain\"><img src=\"{src}\" alt=\"{alt}\"></p>\n");
    }
    let title = escape_html(&img.title);
    format!(
        "<figure class=\"figure\">\n  \
         <img src=\"{src}\" alt=\"{alt}\">\n  \
         <figcaption>{title}</figcaption>\n\
         </figure>\n"
    )
}
/// Render mermaid source to an inline SVG figure with an optional caption.
///
/// Renders are cached by source. A failed render keeps the source as a
/// plain code block so nothing is lost, and the caller prints a warning.
fn render_mermaid_figure(code: &str, title: &str, cache: &mut HashMap<String, String>) -> String {
    let source = code.trim();
    let svg = if let Some(cached) = cache.get(source) {
        cached.clone()
    } else {
        let theme = mermaid_theme();
        let opts = mermaid_rs_renderer::RenderOptions {
            theme,
            ..Default::default()
        };
        match mermaid_rs_renderer::render_with_options(source, opts) {
            Ok(svg) => {
                cache.insert(source.to_string(), svg.clone());
                svg
            }
            Err(e) => {
                eprintln!("sblog: warning: mermaid render failed: {e}");
                return highlight_code(code, "mermaid", &SyntaxSet::load_defaults_newlines());
            }
        }
    };

    if title.is_empty() {
        format!("<figure class=\"figure\">\n{svg}</figure>\n")
    } else {
        let caption = escape_html(title);
        format!(
            "<figure class=\"figure\">\n{svg}  \
             <figcaption>{caption}</figcaption>\n\
             </figure>\n"
        )
    }
}

/// Build the mermaid theme from the site palette so diagrams match the docs.
fn mermaid_theme() -> mermaid_rs_renderer::Theme {
    let mut theme = mermaid_rs_renderer::Theme::modern();
    theme.primary_color = "#e8f1fb".to_string();
    theme.primary_text_color = "#1a1a1a".to_string();
    theme.primary_border_color = "#0066cc".to_string();
    theme.line_color = "#444444".to_string();
    theme.secondary_color = "#f7f7f7".to_string();
    theme.tertiary_color = "#f7f7f7".to_string();
    theme.background = "#ffffff".to_string();
    theme.font_family = "system-ui, -apple-system, 'Segoe UI', Arial, sans-serif".to_string();
    theme
}

/// Split a fenced code info string into the language and the title.
///
/// The first token is the language. The rest, trimmed, is the title.
/// `mermaid The request flow` yields `("mermaid", "The request flow")`.
fn split_info_string(info: &str) -> (String, String) {
    let mut parts = info.splitn(2, char::is_whitespace);
    let lang = parts.next().unwrap_or_default().to_string();
    let title = parts.next().unwrap_or_default().trim().to_string();
    (lang, title)
}

/// Escape HTML special characters in text destined for element content or
/// attribute values.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Highlight a code block with syntect and wrap it in a `<pre>`.
fn highlight_code(code: &str, lang: &str, ss: &SyntaxSet) -> String {
    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut generator = ClassedHTMLGenerator::new_with_class_style(
        syntax,
        ss,
        ClassStyle::SpacedPrefixed { prefix: "syn-" },
    );
    for line in LinesWithEndings::from(code) {
        let _ = generator.parse_html_for_line_which_includes_newline(line);
    }
    let highlighted = generator.finalize();
    let lang_class = if lang.is_empty() {
        String::new()
    } else {
        format!(" language-{}", lang)
    };
    format!(
        "<pre><code class=\"code{}\">{}</code></pre>\n",
        lang_class, highlighted
    )
}

/// Render a Tera template with a document context.
fn render(tera: &Tera, template: &str, doc: &Document) -> String {
    let mut context = Context::new();
    context.insert("site", &SiteView::from(doc));
    context.insert("doc", doc);
    // Expose the document fields at the top level so the templates can use
    // them directly (for example `post`, `posts`, `tags`, `sidebar`).
    context.insert("title", &doc.title);
    context.insert("description", &doc.description);
    context.insert("social_meta", &doc.social_meta);
    context.insert("header_title", &doc.header_title);
    context.insert("header_tagline", &doc.header_tagline);
    context.insert("year", &doc.year);
    context.insert("css_href", &doc.css_href);
    context.insert("home_href", &doc.home_href);
    context.insert("archive_href", &doc.archive_href);
    context.insert("tags_href", &doc.tags_href);
    context.insert("about_href", &doc.about_href);
    context.insert("feed_href", &doc.feed_href);
    context.insert("active_nav", &doc.active_nav);
    context.insert("posts", &doc.posts);
    context.insert("post", &doc.post);
    context.insert("prev_post", &doc.prev_post);
    context.insert("next_post", &doc.next_post);
    context.insert("sidebar", &doc.sidebar);
    context.insert("tags", &doc.tags);
    context.insert("tag", &doc.tag);
    tera.render(template, &context)
        .unwrap_or_else(|e| panic!("failed to render {template}: {e}"))
}

/// Build the document for the index page. The homepage shows the latest
/// post as the featured card, a grid of older posts, and a Topics sidebar.
fn document_for_index(config: &SiteConfig, posts: &[Post]) -> Document {
    let social_meta = render_social_meta(config, "", &config.tagline, "/index.html");
    let latest = posts.first();
    let current = latest.map(|p| post_view(p, "post/", "tags/"));
    let sidebar = sidebar_view(config, posts, "post/", "tags/", "", "posts.html");
    // The home page lists up to 10 posts: the first is the featured card
    // and the rest fill the card grid.
    let home_posts = posts
        .iter()
        .take(10)
        .map(|p| post_view(p, "post/", "tags/"))
        .collect::<Vec<_>>();
    let tags = collect_tags(posts);
    let tag_views = tags
        .iter()
        .map(|(tag, tag_posts)| TagView {
            name: tag.clone(),
            href: format!("tags/{}.html", slugify(tag)),
            count: tag_posts.len(),
        })
        .collect::<Vec<_>>();
    Document {
        title: config.title.clone(),
        site_title: config.title.clone(),
        description: config.tagline.clone(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: posts
            .first()
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, ""),
        home_href: "index.html".to_string(),
        archive_href: "archive.html".to_string(),
        tags_href: "tags/index.html".to_string(),
        about_href: "about.html".to_string(),
        feed_href: "feed.xml".to_string(),
        active_nav: "latest".to_string(),
        posts: home_posts,
        post: current,
        prev_post: None,
        next_post: None,
        sidebar,
        tags: tag_views,
        tag: None,
    }
}

/// Build the document for the archive page listing every post.
fn document_for_archive(config: &SiteConfig, posts: &[Post]) -> Document {
    let social_meta = render_social_meta(
        config,
        "Archive",
        "Every post on this site.",
        "/archive.html",
    );
    let post_views = posts
        .iter()
        .map(|p| post_view(p, "post/", "tags/"))
        .collect::<Vec<_>>();
    let tags = collect_tags(posts);
    let tag_views = tags
        .iter()
        .map(|(tag, tag_posts)| TagView {
            name: tag.clone(),
            href: format!("tags/{}.html", slugify(tag)),
            count: tag_posts.len(),
        })
        .collect::<Vec<_>>();
    Document {
        title: format!("Archive — {}", config.title),
        site_title: config.title.clone(),
        description: "Every post on this site.".to_string(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: posts
            .first()
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, ""),
        home_href: "index.html".to_string(),
        archive_href: "archive.html".to_string(),
        tags_href: "tags/index.html".to_string(),
        about_href: "about.html".to_string(),
        feed_href: "feed.xml".to_string(),
        active_nav: "archive".to_string(),
        posts: post_views,
        post: None,
        prev_post: None,
        next_post: None,
        sidebar: SidebarView {
            older_count: 0,
            essays: Vec::new(),
            tags: Vec::new(),
            archive_href: "archive.html".to_string(),
        },
        tags: tag_views,
        tag: None,
    }
}

/// Build the document for the simple posts list page.
fn document_for_posts(config: &SiteConfig, posts: &[Post]) -> Document {
    let social_meta = render_social_meta(
        config,
        "All posts",
        "Every article on this site.",
        "/posts.html",
    );
    let post_views = posts
        .iter()
        .map(|p| post_view(p, "post/", "tags/"))
        .collect::<Vec<_>>();
    Document {
        title: format!("All posts — {}", config.title),
        site_title: config.title.clone(),
        description: "Every article on this site.".to_string(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: posts
            .first()
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, ""),
        home_href: "index.html".to_string(),
        archive_href: "archive.html".to_string(),
        tags_href: "tags/index.html".to_string(),
        about_href: "about.html".to_string(),
        feed_href: "feed.xml".to_string(),
        active_nav: "archive".to_string(),
        posts: post_views,
        post: None,
        prev_post: None,
        next_post: None,
        sidebar: SidebarView {
            older_count: 0,
            essays: Vec::new(),
            tags: Vec::new(),
            archive_href: "archive.html".to_string(),
        },
        tags: Vec::new(),
        tag: None,
    }
}

/// Build the document for the about page.
fn document_for_about(root: &Path, config: &SiteConfig, posts: &[Post]) -> Document {
    let social_meta = render_social_meta(config, "About", "About me and this site.", "/about.html");
    let about_path = root.join(&config.posts_dir).join("about.md");
    let about_text = fs::read_to_string(&about_path).unwrap_or_default();
    let (_, body) = split_frontmatter(&about_text).unwrap_or(("", ""));
    let body_html = render_markdown(body, "");
    let body_html = anchorize_headings(&body_html);
    let body_html = externalize_links(&body_html);
    let about_view = PostView {
        title: "About Me".to_string(),
        summary: "Software architect, distributed systems engineer, and open-source hacker."
            .to_string(),
        href: "about.html".to_string(),
        date_long: String::new(),
        date_short: String::new(),
        read_time: String::new(),
        tags: Vec::new(),
        body_html,
        headings: Vec::new(),
    };
    let sidebar = sidebar_view(config, posts, "post/", "tags/", "", "posts.html");
    Document {
        title: format!("About — {}", config.title),
        site_title: config.title.clone(),
        description: "About me and this site.".to_string(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: posts
            .first()
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, ""),
        home_href: "index.html".to_string(),
        archive_href: "archive.html".to_string(),
        tags_href: "tags/index.html".to_string(),
        about_href: "about.html".to_string(),
        feed_href: "feed.xml".to_string(),
        active_nav: "about".to_string(),
        posts: Vec::new(),
        post: Some(about_view),
        prev_post: None,
        next_post: None,
        sidebar,
        tags: Vec::new(),
        tag: None,
    }
}

/// Build the document for a single post page.
fn document_for_post(config: &SiteConfig, post: &Post, all: &[Post]) -> Document {
    let social_meta = render_social_meta(
        config,
        &post.title,
        &post.summary,
        &format!("/post/{}.html", post.slug),
    );
    let current = post_view(post, "", "../tags/");
    // Posts are sorted newest-first. "Prev" is the newer post (the one
    // before this one in the list); "Next" is the older post (the one after).
    // The latest post has no newer post, so its prev link is absent.
    let idx = all.iter().position(|p| p.slug == post.slug);
    let prev_post = idx
        .and_then(|i| i.checked_sub(1))
        .map(|i| post_view(&all[i], "", "../tags/"));
    let next_post = idx
        .and_then(|i| all.get(i + 1))
        .map(|p| post_view(p, "", "../tags/"));
    let sidebar = sidebar_view(config, all, "", "../tags/", &post.slug, "../posts.html");
    Document {
        title: format!("{} — {}", post.title, config.title),
        site_title: config.title.clone(),
        description: post.summary.clone(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: post.date.format("%Y").to_string(),
        css_href: css_href(config, "../"),
        home_href: "../index.html".to_string(),
        archive_href: "../archive.html".to_string(),
        tags_href: "../tags/index.html".to_string(),
        about_href: "../about.html".to_string(),
        feed_href: "../feed.xml".to_string(),
        active_nav: "latest".to_string(),
        posts: Vec::new(),
        post: Some(current),
        prev_post,
        next_post,
        sidebar,
        tags: Vec::new(),
        tag: None,
    }
}

fn document_for_tag(config: &SiteConfig, tag: &str, tag_posts: &[&Post], all: &[Post]) -> Document {
    let social_meta = render_social_meta(
        config,
        &format!("Posts tagged {tag}"),
        &format!("All posts tagged \"{tag}\"."),
        &format!("/tags/{}.html", slugify(tag)),
    );
    let post_views = tag_posts
        .iter()
        .map(|p| post_view(p, "../post/", "../tags/"))
        .collect::<Vec<_>>();
    let sidebar = sidebar_view(config, all, "../post/", "../tags/", "", "../posts.html");
    Document {
        title: format!("Posts tagged {tag} — {}", config.title),
        site_title: config.title.clone(),
        description: format!("All posts tagged {tag}."),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: tag_posts
            .first()
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, "../"),
        home_href: "../index.html".to_string(),
        archive_href: "../archive.html".to_string(),
        tags_href: "index.html".to_string(),
        about_href: "../about.html".to_string(),
        feed_href: "../feed.xml".to_string(),
        active_nav: "archive".to_string(),
        posts: post_views,
        post: None,
        prev_post: None,
        next_post: None,
        sidebar,
        tags: Vec::new(),
        tag: Some(TagView {
            name: tag.to_string(),
            href: format!("{}.html", slugify(tag)),
            count: tag_posts.len(),
        }),
    }
}

/// Build the document for the tag index page.
fn document_for_tag_index(config: &SiteConfig, tags: &[(String, Vec<&Post>)]) -> Document {
    let social_meta = render_social_meta(
        config,
        "Tags",
        "Every tag on this site.",
        "/tags/index.html",
    );
    let tag_views = tags
        .iter()
        .map(|(tag, tag_posts)| TagView {
            name: tag.clone(),
            href: format!("{}.html", slugify(tag)),
            count: tag_posts.len(),
        })
        .collect::<Vec<_>>();
    Document {
        title: format!("Tags — {}", config.title),
        site_title: config.title.clone(),
        description: "Every tag on this site.".to_string(),
        social_meta,
        header_title: config.header_title.clone(),
        header_tagline: config.header_tagline.clone(),
        year: tags
            .first()
            .and_then(|(_, p)| p.first())
            .map(|p| p.date.format("%Y").to_string())
            .unwrap_or_default(),
        css_href: css_href(config, "../"),
        home_href: "../index.html".to_string(),
        archive_href: "../archive.html".to_string(),
        tags_href: "index.html".to_string(),
        about_href: "../about.html".to_string(),
        feed_href: "../feed.xml".to_string(),
        active_nav: "archive".to_string(),
        posts: Vec::new(),
        post: None,
        prev_post: None,
        next_post: None,
        sidebar: SidebarView {
            older_count: 0,
            essays: Vec::new(),
            tags: Vec::new(),
            archive_href: "../archive.html".to_string(),
        },
        tags: tag_views,
        tag: None,
    }
}

/// Build a post view for the templates.
fn post_view(post: &Post, href_prefix: &str, tag_prefix: &str) -> PostView {
    PostView {
        title: post.title.clone(),
        summary: post.summary.clone(),
        href: format!("{}{}.html", href_prefix, post.slug),
        date_long: post.date.format("%B %d, %Y").to_string(),
        date_short: post.date.format("%b %d, %Y").to_string(),
        read_time: format_read_time(&post.summary),
        tags: post
            .tags
            .iter()
            .map(|t| TagView {
                name: t.clone(),
                href: format!("{}{}.html", tag_prefix, slugify(t)),
                count: 0,
            })
            .collect(),
        body_html: post.body_html.clone(),
        headings: post.headings.clone(),
    }
}

/// Add target="_blank" and rel="noopener" to every external link in HTML.
///
/// An external link is any `<a href="http...">` that points outside the
/// site. Internal links (relative paths) are left unchanged.
fn externalize_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(pos) = rest.find("<a ") {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];

        if let Some(end) = rest.find('>') {
            let tag = &rest[..end + 1];
            if tag.contains("href=\"http") && !tag.contains("target=") {
                let base = &tag[..tag.len() - 1];
                out.push_str(&format!("{} target=\"_blank\" rel=\"noopener\">", base));
            } else {
                out.push_str(tag);
            }
            rest = &rest[end + 1..];
        } else {
            out.push_str(rest);
            break;
        }
    }
    out.push_str(rest);
    out
}

fn sidebar_view(
    _config: &SiteConfig,
    posts: &[Post],
    href_prefix: &str,
    tag_prefix: &str,
    exclude_slug: &str,
    archive_href: &str,
) -> SidebarView {
    let essays = posts
        .iter()
        .filter(|p| p.slug != exclude_slug)
        .take(5)
        .map(|p| post_view(p, href_prefix, tag_prefix))
        .collect::<Vec<_>>();
    let tags = collect_tags(posts)
        .iter()
        .map(|(tag, tag_posts)| TagView {
            name: tag.clone(),
            href: format!("{}{}.html", tag_prefix, slugify(tag)),
            count: tag_posts.len(),
        })
        .collect::<Vec<_>>();
    SidebarView {
        older_count: essays.len(),
        essays,
        tags,
        archive_href: archive_href.to_string(),
    }
}

/// The stylesheet href, relative to the current page depth.
fn css_href(config: &SiteConfig, depth: &str) -> String {
    let file = Path::new(&config.css_file)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("style.css");
    format!("{depth}{file}")
}

/// Collect every tag and the posts that carry it, sorted by tag name.
fn collect_tags(posts: &[Post]) -> Vec<(String, Vec<&Post>)> {
    let mut map: std::collections::BTreeMap<String, Vec<&Post>> = std::collections::BTreeMap::new();
    for post in posts {
        for tag in &post.tags {
            map.entry(tag.clone()).or_default().push(post);
        }
    }
    map.into_iter().collect()
}

/// Turn a tag into a safe file name.
fn slugify(tag: &str) -> String {
    let mut out = String::new();
    for c in tag.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c.is_whitespace() || c == '-' || c == '_' {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push_str("tag");
    }
    out
}

/// Write the SEO files (robots.txt, sitemap.xml, and feed.xml) to the
/// output directory.
fn write_seo_files(
    config: &SiteConfig,
    posts: &[Post],
    tags: &[(String, Vec<&Post>)],
    public_dir: &Path,
) {
    let robots = render_robots_txt(config);
    fs::write(public_dir.join("robots.txt"), robots).expect("write robots.txt");

    let sitemap = render_sitemap_xml(config, posts, tags);
    fs::write(public_dir.join("sitemap.xml"), sitemap).expect("write sitemap.xml");

    let feed = render_rss_feed(config, posts);
    fs::write(public_dir.join("feed.xml"), feed).expect("write feed.xml");
}

/// Render the robots.txt content. If base_url is set, include a sitemap
/// reference. Otherwise allow all crawlers and skip the sitemap line.
fn render_robots_txt(config: &SiteConfig) -> String {
    let mut out = String::new();
    out.push_str("User-agent: *\n");
    out.push_str("Allow: /\n");
    if !config.base_url.is_empty() {
        out.push_str(&format!("Sitemap: {}/sitemap.xml\n", config.base_url));
    }
    out
}

/// Render the sitemap.xml content. The sitemap lists every post, the tag
/// pages, and the main pages. It uses absolute URLs from base_url.
fn render_sitemap_xml(
    config: &SiteConfig,
    posts: &[Post],
    tags: &[(String, Vec<&Post>)],
) -> String {
    let base = if config.base_url.is_empty() {
        String::new()
    } else {
        config.base_url.clone()
    };

    let mut urls: Vec<String> = vec![
        "/".to_string(),
        "/archive.html".to_string(),
        "/posts.html".to_string(),
        "/about.html".to_string(),
        "/tags/index.html".to_string(),
    ];
    for post in posts {
        urls.push(format!("/post/{}.html", post.slug));
    }
    for (tag, _tag_posts) in tags {
        urls.push(format!("/tags/{}.html", slugify(tag)));
    }

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for url in &urls {
        let full = format!("{}{}", base, url);
        xml.push_str("  <url>\n");
        xml.push_str(&format!("    <loc>{}</loc>\n", escape_html(&full)));
        xml.push_str("  </url>\n");
    }
    xml.push_str("</urlset>\n");
    xml
}

/// Render the RSS 2.0 feed (feed.xml) content. The feed lists every post
/// with its title, link, description, and publication date. It uses
/// absolute URLs from base_url.
fn render_rss_feed(config: &SiteConfig, posts: &[Post]) -> String {
    let base = if config.base_url.is_empty() {
        String::new()
    } else {
        config.base_url.clone()
    };

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str("  <channel>\n");
    xml.push_str(&format!(
        "    <title>{}</title>\n",
        escape_xml(&config.title)
    ));
    xml.push_str(&format!("    <link>{}/</link>\n", escape_xml(&base)));
    xml.push_str(&format!(
        "    <description>{}</description>\n",
        escape_xml(&config.tagline)
    ));
    if !config.base_url.is_empty() {
        xml.push_str(&format!(
            "    <atom:link href=\"{}/feed.xml\" rel=\"self\" type=\"application/rss+xml\"/>\n",
            escape_xml(&base)
        ));
    }
    for post in posts {
        xml.push_str("    <item>\n");
        xml.push_str(&format!(
            "      <title>{}</title>\n",
            escape_xml(&post.title)
        ));
        xml.push_str(&format!(
            "      <link>{}/post/{}.html</link>\n",
            escape_xml(&base),
            escape_xml(&post.slug)
        ));
        xml.push_str(&format!(
            "      <guid>{}/post/{}.html</guid>\n",
            escape_xml(&base),
            escape_xml(&post.slug)
        ));
        xml.push_str(&format!(
            "      <pubDate>{}</pubDate>\n",
            post.date.format("%a, %d %b %Y 00:00:00 +0000")
        ));
        if !post.summary.is_empty() {
            xml.push_str(&format!(
                "      <description>{}</description>\n",
                escape_xml(&post.summary)
            ));
        }
        xml.push_str("    </item>\n");
    }
    xml.push_str("  </channel>\n");
    xml.push_str("</rss>\n");
    xml
}

/// Escape XML special characters in a string.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Build the Open Graph and Twitter Card meta tags for a page.
fn render_social_meta(
    config: &SiteConfig,
    page_title: &str,
    description: &str,
    url: &str,
) -> String {
    let site_title = &config.title;
    let og_title = if page_title.is_empty() {
        site_title.clone()
    } else {
        format!("{page_title} — {site_title}")
    };

    let mut meta = String::new();
    meta.push_str("<meta property=\"og:type\" content=\"website\">\n");
    meta.push_str(&format!(
        "<meta property=\"og:site_name\" content=\"{}\">\n",
        escape_html(site_title)
    ));
    meta.push_str(&format!(
        "<meta property=\"og:title\" content=\"{}\">\n",
        escape_html(&og_title)
    ));
    meta.push_str(&format!(
        "<meta property=\"og:description\" content=\"{}\">\n",
        escape_html(description)
    ));

    if !config.base_url.is_empty() {
        let full_url = format!("{}{}", config.base_url, url);
        meta.push_str(&format!(
            "<meta property=\"og:url\" content=\"{}\">\n",
            escape_html(&full_url)
        ));
        meta.push_str(&format!(
            "<meta property=\"og:image\" content=\"{}\">\n",
            escape_html(&config.og_image)
        ));
        meta.push_str("<meta name=\"twitter:card\" content=\"summary_large_image\">\n");
        meta.push_str(&format!(
            "<meta name=\"twitter:title\" content=\"{}\">\n",
            escape_html(&og_title)
        ));
        meta.push_str(&format!(
            "<meta name=\"twitter:description\" content=\"{}\">\n",
            escape_html(description)
        ));
        if !config.og_image.is_empty() {
            meta.push_str(&format!(
                "<meta name=\"twitter:image\" content=\"{}\">\n",
                escape_html(&config.og_image)
            ));
        }
        if !config.twitter_handle.is_empty() {
            meta.push_str(&format!(
                "<meta name=\"twitter:site\" content=\"@{}\">\n",
                escape_html(&config.twitter_handle)
            ));
        }
    }

    meta
}

/// Estimate the reading time from a summary string.
fn format_read_time(summary: &str) -> String {
    let words = summary.split_whitespace().count();
    let minutes = (words as f64 / 200.0).ceil().max(1.0) as u32;
    format!("{minutes} min read")
}
