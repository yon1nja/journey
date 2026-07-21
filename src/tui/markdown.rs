use super::*;

const CODE_THEME: &str = "base16-ocean.dark";

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
pub(super) struct DocTab {
    pub(super) label: String,
    pub(super) path: PathBuf,
}

pub(super) fn discover_doc_tabs(journey_path: &Path) -> Vec<DocTab> {
    let mut tabs = Vec::new();

    let readme_path = journey_path.join(storage::README_FILE);
    if readme_path.is_file() {
        tabs.push(DocTab {
            label: "README".to_string(),
            path: readme_path,
        });
    }

    let docs_dir = journey_path.join(storage::DOCS_DIR);
    if let Ok(entries) = fs::read_dir(&docs_dir) {
        let mut doc_files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .collect();
        doc_files.sort_by_key(|e| e.file_name());

        for entry in doc_files {
            let path = entry.path();
            let label = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            tabs.push(DocTab { label, path });
        }
    }

    tabs
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct DocCacheKey {
    pub(super) path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Default)]
pub(super) struct DocRenderCache {
    key: Option<DocCacheKey>,
    lines: Vec<Line<'static>>,
}

impl DocRenderCache {
    pub(super) fn lines(&mut self, path: &Path) -> Vec<Line<'static>> {
        let Ok(metadata) = fs::metadata(path) else {
            return vec![Line::from(Span::styled("unavailable", dim()))];
        };

        let key = DocCacheKey {
            path: path.to_path_buf(),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        };

        if self.key.as_ref() != Some(&key) {
            self.lines = render_doc_file(path);
            self.key = Some(key);
        }

        self.lines.clone()
    }
}

pub(super) fn render_doc_file(path: &Path) -> Vec<Line<'static>> {
    match fs::read_to_string(path) {
        Ok(content) if content.trim().is_empty() => {
            vec![Line::from(Span::styled("(empty)", dim()))]
        }
        Ok(content) => render_markdown_lines(&content),
        Err(err) => vec![Line::from(Span::styled(
            format!("unavailable: {err}"),
            dim(),
        ))],
    }
}

pub(super) fn render_markdown_lines(content: &str) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::default();
    let parser = Parser::new_ext(
        content,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
    );

    for (event, range) in parser.into_offset_iter() {
        renderer.preserve_source_gap(content, range.start);
        renderer.handle(event);
        renderer.mark_source_end(range.end);
    }

    renderer.preserve_source_gap(content, content.len());
    renderer.finish()
}

pub(super) fn source_blank_lines(gap: &str) -> usize {
    if !gap.trim().is_empty() {
        return 0;
    }

    gap.bytes().filter(|byte| *byte == b'\n').count()
}

#[derive(Default)]
struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_stack: Vec<ListMarker>,
    current_item_prefix: Option<String>,
    blockquote_depth: usize,
    code_block: Option<MarkdownCodeBlock>,
    pending_link_destinations: Vec<String>,
    source_offset: usize,
}

impl MarkdownRenderer {
    fn handle(&mut self, event: MarkdownEvent<'_>) {
        match event {
            MarkdownEvent::Start(tag) => self.start_tag(tag),
            MarkdownEvent::End(tag) => self.end_tag(tag),
            MarkdownEvent::Text(text)
            | MarkdownEvent::Html(text)
            | MarkdownEvent::InlineHtml(text) => {
                self.push_text(text.as_ref());
            }
            MarkdownEvent::Code(code) | MarkdownEvent::InlineMath(code) => {
                self.push_span(Span::styled(code.to_string(), inline_code()));
            }
            MarkdownEvent::DisplayMath(math) => {
                self.flush_current();
                self.push_span(Span::styled(format!("    {math}"), code_block_style()));
                self.flush_current();
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => self.flush_current(),
            MarkdownEvent::Rule => {
                self.flush_current();
                self.lines
                    .push(Line::from(Span::styled("----------------", dim())));
            }
            MarkdownEvent::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(Span::styled(marker, dim()));
            }
            MarkdownEvent::FootnoteReference(reference) => {
                self.push_span(Span::styled(format!("[{reference}]"), dim()));
            }
        }
    }

    fn preserve_source_gap(&mut self, source: &str, next_start: usize) {
        if self.code_block.is_some() || next_start <= self.source_offset {
            return;
        }

        let gap = &source[self.source_offset..next_start];
        let blank_lines = source_blank_lines(gap);
        if blank_lines == 0 {
            return;
        }

        self.flush_current();
        for _ in 0..blank_lines {
            self.lines.push(Line::from(""));
        }
    }

    fn mark_source_end(&mut self, end: usize) {
        self.source_offset = self.source_offset.max(end);
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_current();
                self.push_style(heading_style(level));
            }
            Tag::BlockQuote(_) => {
                self.flush_current();
                self.blockquote_depth += 1;
                self.push_style(blockquote_style());
            }
            Tag::CodeBlock(kind) => {
                self.flush_current();
                self.code_block = Some(MarkdownCodeBlock::new(kind));
            }
            Tag::List(start) => self.list_stack.push(ListMarker::new(start)),
            Tag::Item => self.start_list_item(),
            Tag::Emphasis => self.push_style(self.current_style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(self.current_style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                self.push_style(self.current_style().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.pending_link_destinations.push(dest_url.to_string());
                self.push_style(accent().add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                let label = if title.is_empty() {
                    dest_url.to_string()
                } else {
                    format!("{title} ({dest_url})")
                };
                self.push_span(Span::styled(format!("[image: {label}]"), dim()));
                self.push_style(dim());
            }
            Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            Tag::FootnoteDefinition(name) => {
                self.flush_current();
                self.push_span(Span::styled(format!("[{name}] "), dim()));
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::FootnoteDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => self.flush_current(),
            TagEnd::Heading(_) => {
                self.flush_current();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_current();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                self.flush_current();
                if let Some(block) = self.code_block.take() {
                    self.push_code_block(block);
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_current();
                self.current_item_prefix = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(destination) = self.pending_link_destinations.pop() {
                    if !destination.is_empty() {
                        self.push_span(Span::styled(format!(" ({destination})"), dim()));
                    }
                }
            }
            TagEnd::Image => self.pop_style(),
            TagEnd::HtmlBlock | TagEnd::TableCell => {}
        }
    }

    fn start_list_item(&mut self) {
        self.flush_current();
        let depth = self.list_stack.len().saturating_sub(1);
        let indent = "  ".repeat(depth);
        let marker = self
            .list_stack
            .last_mut()
            .map(ListMarker::next)
            .unwrap_or_else(|| "- ".to_string());
        self.current_item_prefix = Some(format!("{indent}{marker}"));
    }

    fn push_text(&mut self, text: &str) {
        if let Some(block) = &mut self.code_block {
            block.content.push_str(text);
            return;
        }

        for (idx, part) in text.split('\n').enumerate() {
            if idx > 0 {
                self.flush_current();
            }
            if !part.is_empty() {
                self.push_span(Span::styled(part.to_string(), self.current_style()));
            }
        }
    }

    fn push_span(&mut self, span: Span<'static>) {
        if self.current.is_empty() {
            self.push_prefix();
        }
        self.current.push(span);
    }

    fn push_prefix(&mut self) {
        self.current.extend(self.prefix_spans());
    }

    fn prefix_spans(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for _ in 0..self.blockquote_depth {
            spans.push(Span::styled("> ", dim()));
        }
        if let Some(prefix) = &self.current_item_prefix {
            spans.push(Span::styled(prefix.clone(), dim()));
        }
        spans
    }

    fn push_code_block(&mut self, block: MarkdownCodeBlock) {
        if let Some(language) = block.language.as_deref() {
            self.push_line_with_prefix(vec![
                Span::styled("code", dim()),
                Span::styled(format!(" {language}"), dim()),
            ]);
        }

        for spans in highlight_code_lines(&block.content, block.language.as_deref()) {
            self.push_line_with_prefix(spans);
        }
    }

    fn push_line_with_prefix(&mut self, spans: Vec<Span<'static>>) {
        let mut line = self.prefix_spans();
        line.extend(spans);
        self.lines.push(Line::from(line));
    }

    fn flush_current(&mut self) {
        if !self.current.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current)));
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current();
        self.lines
    }

    fn current_style(&self) -> Style {
        self.style_stack
            .last()
            .copied()
            .unwrap_or_else(Style::default)
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(style);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }
}

struct MarkdownCodeBlock {
    language: Option<String>,
    content: String,
}

impl MarkdownCodeBlock {
    fn new(kind: CodeBlockKind<'_>) -> Self {
        let language = match kind {
            CodeBlockKind::Fenced(info) => parse_code_language(info.as_ref()),
            CodeBlockKind::Indented => None,
        };
        Self {
            language,
            content: String::new(),
        }
    }
}

pub(super) fn parse_code_language(info: &str) -> Option<String> {
    let token = info
        .split_whitespace()
        .next()?
        .trim_matches(|ch| ch == '{' || ch == '}' || ch == '.');
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

pub(super) fn highlight_code_lines(code: &str, language: Option<&str>) -> Vec<Vec<Span<'static>>> {
    let Some(language) = language else {
        return plain_code_lines(code);
    };
    let Some(syntax) = syntax_for_language(language) else {
        return plain_code_lines(code);
    };
    let Some(theme) = code_theme() else {
        return plain_code_lines(code);
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    code.lines()
        .map(|line| {
            let mut spans = vec![Span::styled("    ", code_block_style())];
            match highlighter.highlight_line(line, &SYNTAX_SET) {
                Ok(ranges) => {
                    for (style, text) in ranges {
                        if !text.is_empty() {
                            spans.push(Span::styled(text.to_string(), syntax_style(style)));
                        }
                    }
                }
                Err(_) => spans.push(Span::styled(line.to_string(), code_block_style())),
            }
            spans
        })
        .collect()
}

pub(super) fn plain_code_lines(code: &str) -> Vec<Vec<Span<'static>>> {
    code.lines()
        .map(|line| vec![Span::styled(format!("    {line}"), code_block_style())])
        .collect()
}

pub(super) fn syntax_for_language(language: &str) -> Option<&'static SyntaxReference> {
    let syntax_set = &*SYNTAX_SET;
    for candidate in language_candidates(language) {
        if let Some(syntax) = syntax_set
            .find_syntax_by_token(&candidate)
            .or_else(|| syntax_set.find_syntax_by_extension(&candidate))
        {
            return Some(syntax);
        }
    }
    None
}

pub(super) fn language_candidates(language: &str) -> Vec<String> {
    let token = language.trim().trim_start_matches('.').to_ascii_lowercase();
    let alias = match token.as_str() {
        "rs" => Some("rust"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "sh" | "zsh" | "shell" => Some("bash"),
        "yml" => Some("yaml"),
        "md" => Some("markdown"),
        "rb" => Some("ruby"),
        "py" => Some("python"),
        _ => None,
    };

    let mut candidates = vec![token];
    if let Some(alias) = alias {
        candidates.push(alias.to_string());
    }
    candidates
}

pub(super) fn code_theme() -> Option<&'static Theme> {
    THEME_SET
        .themes
        .get(CODE_THEME)
        .or_else(|| THEME_SET.themes.values().next())
}

pub(super) fn syntax_style(style: SyntectStyle) -> Style {
    let mut result = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(SyntectFontStyle::BOLD) {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(SyntectFontStyle::ITALIC) {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(SyntectFontStyle::UNDERLINE) {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    result
}

struct ListMarker {
    next: Option<u64>,
}

impl ListMarker {
    fn new(next: Option<u64>) -> Self {
        Self { next }
    }

    fn next(&mut self) -> String {
        match self.next {
            Some(value) => {
                self.next = Some(value + 1);
                format!("{value}. ")
            }
            None => "- ".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_readme_markdown_as_terminal_lines() {
        let lines = render_markdown_lines(
            "# Overview\n\nA **short** note with [docs](https://example.com).\n\n- first\n- second\n\n```sh\ncargo test\n```",
        );
        let text = lines.iter().map(plain_line).collect::<Vec<_>>().join("\n");

        assert!(text.contains("Overview"));
        assert!(text.contains("A short note with docs (https://example.com)."));
        assert!(text.contains("- first"));
        assert!(text.contains("- second"));
        assert!(text.contains("code sh"));
        assert!(text.contains("    cargo test"));
    }

    #[test]
    fn renders_fenced_code_with_syntax_coloring() {
        let lines = render_markdown_lines("```rust\nfn main() {}\n```");
        let code_line = lines
            .iter()
            .find(|line| plain_line(line).contains("fn main"))
            .expect("rendered Rust code line");

        assert!(code_line
            .spans
            .iter()
            .any(|span| matches!(span.style.fg, Some(Color::Rgb(_, _, _)))));
    }

    #[test]
    fn preserves_blank_lines_between_markdown_blocks() {
        let lines =
            render_markdown_lines("Observed error:\n\n\n```js\nconsole.log('broken');\n```");
        let plain = lines.iter().map(plain_line).collect::<Vec<_>>();

        assert_eq!(
            plain,
            vec![
                "Observed error:",
                "",
                "",
                "code js",
                "    console.log('broken');"
            ]
        );
    }
    fn plain_line(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
