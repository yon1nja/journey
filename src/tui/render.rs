use super::*;

pub(super) fn render(frame: &mut Frame<'_>, app: &mut JourneyApp) {
    app.prune_toasts();

    let area = frame.area();

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, root[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(root[1]);
    render_journey_list(frame, app, body[0]);
    render_details_pane(frame, app, body[1], app.screen == Screen::Details);

    if app.screen == Screen::Actions {
        let action_count = app.shortcuts.actions().len().max(1) as u16 + 2; // +2 for border
        let height = action_count.min(body[0].height);
        let action_area = Rect {
            x: body[0].x,
            y: body[0].y + body[0].height - height,
            width: body[0].width,
            height,
        };
        render_actions(frame, app, action_area);
    }

    render_footer(frame, app, root[2]);
    render_toasts(frame, app, area);
    render_notification_history(frame, app, area);
    render_dialog(frame, app, area);
}

pub(super) fn render_header(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    let repo_hint = git::discover_repo(&app.cwd)
        .ok()
        .and_then(|repo| {
            repo.root
                .file_name()
                .map(|name| format!("git: {}, {}", name.to_string_lossy(), repo.branch))
        })
        .unwrap_or_else(|| "no git repo detected".to_string());

    let search_indicator = if app.screen == Screen::Search {
        vec![
            Span::styled("search: ", accent().add_modifier(Modifier::BOLD)),
            Span::styled(&app.query, Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]
    } else if !app.query.is_empty() {
        vec![
            Span::styled("query: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.query, Style::default().fg(Color::DarkGray)),
        ]
    } else {
        vec![]
    };

    let mut second_line = vec![
        Span::styled("filter: ", dim()),
        Span::styled(app.filter.label(), accent()),
    ];
    if !search_indicator.is_empty() {
        second_line.push(Span::raw("  "));
        second_line.extend(search_indicator);
    }

    let lines = vec![
        Line::from(vec![
            Span::styled("Journey", accent().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(repo_hint, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(second_line),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

pub(super) fn render_journey_list(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    let rows = if app.filtered.is_empty() {
        vec![Row::new([
            Cell::from("No matching Journeys").style(dim()),
            Cell::from(""),
            Cell::from(""),
        ])]
    } else {
        app.filtered
            .iter()
            .map(|idx| journey_row(&app.rows[*idx]))
            .collect()
    };

    let block = Block::default()
        .title(" Journeys ")
        .borders(Borders::ALL)
        .border_style(match app.screen {
            Screen::List | Screen::Search => accent(),
            Screen::Details | Screen::Actions => Style::default().fg(Color::DarkGray),
        });
    let header = Row::new([
        Cell::from("Journey"),
        Cell::from("Status"),
        Cell::from("Repos"),
    ])
    .style(dim())
    .bottom_margin(1);
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(54),
            Constraint::Length(11),
            Constraint::Min(10),
        ],
    )
    .block(block)
    .header(header)
    .column_spacing(2)
    .row_highlight_style(selected_row())
    .highlight_symbol(selection_stroke())
    .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

pub(super) fn journey_row(entry: &IndexEntry) -> Row<'static> {
    let repos = if entry.repos.is_empty() {
        "no repos".to_string()
    } else if entry.repos.len() == 1 {
        entry.repos[0].clone()
    } else {
        format!("{} repos", entry.repos.len())
    };

    Row::new([
        Cell::from(entry.title.clone()).style(Style::default().fg(Color::White)),
        Cell::from(entry.status.to_string()).style(status_style(entry.status)),
        Cell::from(repos).style(dim()),
    ])
}

pub(super) fn render_details_pane(
    frame: &mut Frame<'_>,
    app: &mut JourneyApp,
    area: Rect,
    focused: bool,
) {
    let has_terminal_tabs = !app.workspace.tabs_for_selected().is_empty();
    let right_pane_tab = app.workspace.right_pane_tab();
    let is_terminal_active = right_pane_tab > 0 && app.workspace.active_tab_is_terminal();

    // Determine border highlight: accent when focused, or when terminal is focused
    let is_highlighted = focused
        || (is_terminal_active && app.workspace.focus == workspace::Focus::Terminal);

    let block = Block::default()
        .title(if !has_terminal_tabs {
            " Details "
        } else {
            "" // tab bar replaces title
        })
        .borders(Borders::ALL)
        .border_style(if is_highlighted {
            accent()
        } else {
            Style::default()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // If there are terminal tabs, render the tab bar and split the inner area
    if has_terminal_tabs {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        workspace::render_right_pane_tab_bar(frame, chunks[0], &app.workspace);

        if is_terminal_active {
            // Render terminal grid
            if let Some(terminal) = app.workspace.active_terminal() {
                workspace::render_term_grid(frame, chunks[1], terminal);
            }
            return;
        }

        // Details tab with tab bar: render details content below the tab bar
        render_details_content(frame, app, chunks[1], focused);
    } else {
        // No terminal tabs: render details content in the full inner area
        render_details_content(frame, app, inner, focused);
    }
}

fn render_details_content(
    frame: &mut Frame<'_>,
    app: &mut JourneyApp,
    area: Rect,
    focused: bool,
) {
    let meta = metadata_lines(app);

    let journey_path = app
        .selected_entry()
        .map(|e| storage::journey_dir(&app.home, &e.id));
    let tabs = journey_path
        .as_deref()
        .map(discover_doc_tabs)
        .unwrap_or_default();

    if tabs.is_empty() {
        let mut content_area = area;
        let mut content_height = wrapped_line_count(&meta, content_area.width);
        let viewport_height = usize::from(content_area.height);
        let is_scrollable = content_height > viewport_height;
        if is_scrollable && content_area.width > 1 {
            content_area.width = content_area.width.saturating_sub(1);
            content_height = wrapped_line_count(&meta, content_area.width);
        }
        app.details_viewport_height = usize::from(content_area.height).max(1);
        let max_scroll = content_height.saturating_sub(app.details_viewport_height);
        app.details_scroll = app.details_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(Text::from(meta))
                .wrap(Wrap { trim: false })
                .scroll((scroll_offset(app.details_scroll), 0)),
            content_area,
        );
        render_details_scrollbar(
            frame,
            area,
            app.details_scroll,
            app.details_viewport_height,
            max_scroll,
            focused,
        );
        return;
    }

    let meta_height = wrapped_line_count(&meta, area.width) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(meta_height + 1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Text::from(meta)).wrap(Wrap { trim: false }),
        chunks[0],
    );

    let tab_line = tab_bar_line(&tabs, app.doc_tab_index, usize::from(chunks[1].width));
    frame.render_widget(Paragraph::new(Text::from(vec![tab_line])), chunks[1]);

    let sep = "\u{2500}".repeat(usize::from(chunks[2].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(sep, dim()))),
        chunks[2],
    );

    let doc_lines = doc_content_lines(app, &tabs);
    let mut doc_area = chunks[3];
    let mut content_height = wrapped_line_count(&doc_lines, doc_area.width);
    let viewport_height = usize::from(doc_area.height);
    let is_scrollable = content_height > viewport_height;
    if is_scrollable && doc_area.width > 1 {
        doc_area.width = doc_area.width.saturating_sub(1);
        content_height = wrapped_line_count(&doc_lines, doc_area.width);
    }

    app.details_viewport_height = usize::from(doc_area.height).max(1);
    let max_scroll = content_height.saturating_sub(app.details_viewport_height);
    app.details_scroll = app.details_scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(doc_lines))
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset(app.details_scroll), 0)),
        doc_area,
    );

    render_details_scrollbar(
        frame,
        chunks[3],
        app.details_scroll,
        app.details_viewport_height,
        max_scroll,
        focused,
    );
}

pub(super) fn render_details_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll: usize,
    viewport_height: usize,
    max_scroll: usize,
    focused: bool,
) {
    if max_scroll > 0 && area.width > 0 && area.height > 0 {
        let scrollbar_area = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y,
            width: 1,
            height: area.height,
        };
        let mut scrollbar_state = ScrollbarState::new(details_scrollbar_content_length(max_scroll))
            .position(scroll)
            .viewport_content_length(viewport_height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .thumb_style(if focused { accent() } else { dim() }),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

pub(super) fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum()
}

pub(super) fn scroll_offset(offset: usize) -> u16 {
    offset.min(usize::from(u16::MAX)) as u16
}

pub(super) fn details_scrollbar_content_length(max_scroll: usize) -> usize {
    max_scroll.saturating_add(1)
}

pub(super) fn render_actions(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    let items = app
        .shortcuts
        .actions()
        .iter()
        .map(|action| {
            let mut spans = Vec::new();
            if let Some(shortcut) = app.shortcuts.binding_for_action(*action) {
                spans.push(Span::styled(format!("[{}] ", shortcut.display()), dim()));
            }
            spans.extend([
                Span::styled(action.label(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(action.description(), dim()),
            ]);
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No actions configured",
            dim(),
        )))]
    } else {
        items
    };
    let block = Block::default()
        .title(" Actions ")
        .borders(Borders::ALL)
        .border_style(accent());
    let list = List::new(items)
        .block(block)
        .highlight_style(selected())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut app.action_state);
}

pub(super) fn render_footer(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    let help = match app.screen {
        Screen::List => {
            let mut parts = vec![
                format!("{} search", app.shortcuts.focus_search_display()),
                format!("{} details", app.shortcuts.focus_details_display()),
                format!("{} actions", app.shortcuts.open_actions_display()),
                format!("{} new", app.shortcuts.new_journey.display()),
            ];
            let has_terminal = app
                .selected_id()
                .map(|id| app.workspace.has_tabs(&id))
                .unwrap_or(false);
            if has_terminal {
                parts.push("t tab  x close  [] switch  ^H terminal".to_string());
            }
            parts.push("^N notifications".to_string());
            parts.push(format!("{} quit", app.shortcuts.quit_display()));
            parts.join("  ")
        }
        Screen::Search => format!(
            "{} clear/exit  {} go to list",
            app.shortcuts.back_display(),
            app.shortcuts.confirm_display()
        ),
        Screen::Details => {
            if app.workspace.focus == workspace::Focus::Terminal
                && app.workspace.active_tab_is_terminal()
            {
                "^H sidebar  ^Q quit".to_string()
            } else if !app.workspace.tabs_for_selected().is_empty() {
                format!(
                    "[] tabs  t new  x close  Tab/S-Tab docs  {}/{} scroll  {} back",
                    app.shortcuts.nav_up_display(),
                    app.shortcuts.nav_down_display(),
                    app.shortcuts.back_display()
                )
            } else {
                format!(
                    "Tab/S-Tab switch doc  {}/{} scroll  ^N notifications  {} back",
                    app.shortcuts.nav_up_display(),
                    app.shortcuts.nav_down_display(),
                    app.shortcuts.back_display()
                )
            }
        }
        Screen::Actions => format!(
            "{}/{} navigate  {} run  ^N notifications  {} back",
            app.shortcuts.nav_up_display(),
            app.shortcuts.nav_down_display(),
            app.shortcuts.confirm_display(),
            app.shortcuts.back_display()
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(help, dim())))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

pub(super) fn render_toasts(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    if app.toasts.is_empty() {
        return;
    }

    let max_width = (area.width * 30 / 100).clamp(20, 40);
    let mut y = area.y + 1;

    for toast in app.toasts.iter().rev().take(3) {
        if y + 1 >= area.y + area.height {
            break;
        }
        let style = match toast.level {
            NotificationLevel::Success => Style::default().fg(Color::Black).bg(Color::Green),
            NotificationLevel::Error => Style::default().fg(Color::White).bg(Color::Red),
        };
        let msg = if toast.message.len() > max_width as usize - 2 {
            format!(" {}… ", &toast.message[..max_width as usize - 3])
        } else {
            format!(" {} ", &toast.message)
        };
        let width = msg.len().min(max_width as usize) as u16;
        let x = area.x + area.width - width - 1;
        let toast_area = Rect::new(x, y, width, 1);
        frame.render_widget(Clear, toast_area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, style))),
            toast_area,
        );
        y += 2;
    }
}

pub(super) fn render_notification_history(frame: &mut Frame<'_>, app: &JourneyApp, area: Rect) {
    if !app.show_notification_history {
        return;
    }

    let area = centered_rect(60, 60, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Notifications ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.notification_history.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("No notifications yet", dim()))),
            inner,
        );
        return;
    }

    let visible_height = inner.height as usize;
    let items: Vec<ListItem> = app
        .notification_history
        .iter()
        .rev()
        .skip(app.notification_scroll)
        .take(visible_height)
        .map(|notification| {
            let elapsed = relative_time(notification.created_at.elapsed());
            let style = match notification.level {
                NotificationLevel::Success => Style::default().fg(Color::Green),
                NotificationLevel::Error => Style::default().fg(Color::Red),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{elapsed:>8}"), dim()),
                Span::raw("  "),
                Span::styled(notification.message.clone(), style),
            ]))
        })
        .collect();

    let help = Line::from(Span::styled("j/k scroll  Esc close  Ctrl+n close", dim()));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    frame.render_widget(List::new(items), chunks[0]);
    frame.render_widget(Paragraph::new(help), chunks[1]);
}

pub(super) fn relative_time(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 5 {
        "just now".into()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else {
        format!("{}m ago", secs / 60)
    }
}

pub(super) fn render_dialog(frame: &mut Frame<'_>, app: &mut JourneyApp, area: Rect) {
    match &mut app.dialog {
        Dialog::None => {}
        Dialog::NewTitle { input } => {
            render_input_dialog(
                frame,
                area,
                "New Journey",
                "Title",
                input,
                "Enter next  Esc cancel",
            );
        }
        Dialog::NewDescription { title, input } => render_input_dialog(
            frame,
            area,
            &format!("New Journey: {title}"),
            "Description",
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::WorktreeBranch {
            input,
            default_branch,
            ..
        } => render_input_dialog(
            frame,
            area,
            "New Branch + Worktree",
            &format!("Branch (default: {default_branch})"),
            input,
            "Enter next  Esc cancel",
        ),
        Dialog::WorktreePath {
            input,
            default_path,
            ..
        } => render_input_dialog(
            frame,
            area,
            "New Branch + Worktree",
            &format!("Path (default: {})", default_path.display()),
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::ExistingBranch {
            branches,
            query,
            selected,
            state,
            ..
        } => render_branch_selection_dialog(frame, area, branches, query, *selected, state),
        Dialog::ExistingWorktreePath {
            input,
            default_path,
            ..
        } => render_input_dialog(
            frame,
            area,
            "Existing Branch + Worktree",
            &format!("Path (default: {})", default_path.display()),
            input,
            "Enter create  Esc cancel",
        ),
        Dialog::ExistingWorktreeLinkConfirm {
            branch, worktree, ..
        } => render_confirm_dialog(
            frame,
            area,
            " Link Existing Worktree ",
            vec![
                Line::from(Span::styled(
                    "A worktree for this branch already exists at:",
                    dim(),
                )),
                Line::from(Span::styled(worktree.display().to_string(), accent())),
                Line::from(""),
                Line::from(vec![
                    Span::raw("Branch: "),
                    Span::styled(branch.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Would you like to link this worktree instead of creating a new one?",
                    dim(),
                )),
                Line::from(Span::styled("Enter/y link  n/Esc cancel", dim())),
            ],
        ),
        Dialog::Unlink { repos, state, .. } => {
            render_repo_selection_dialog(frame, area, " Unlink Repo ", repos, state);
        }
        Dialog::DeleteWorktree { repos, state, .. } => {
            render_repo_selection_dialog(frame, area, " Delete Worktree ", repos, state);
        }
        Dialog::Capture { input, .. } => render_input_dialog(
            frame,
            area,
            "Capture Thought",
            "Text",
            input,
            "Enter capture  Esc cancel",
        ),
        Dialog::DoneConfirm {
            journey_id,
            repo_count,
        } => render_confirm_dialog(
            frame,
            area,
            " Done ",
            vec![
                Line::from(vec![
                    Span::raw("Archive "),
                    Span::styled(journey_id.clone(), accent()),
                    Span::raw(format!(" and remove {repo_count} worktrees?")),
                ]),
                Line::from(Span::styled(
                    "Git will refuse dirty or main worktrees.",
                    dim(),
                )),
                Line::from(""),
                Line::from(Span::styled("Enter confirm  Esc cancel", dim())),
            ],
        ),
    }
}

pub(super) fn render_branch_selection_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    branches: &[String],
    query: &str,
    selected_index: usize,
    state: &mut ListState,
) {
    let area = centered_rect(74, 72, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Existing Branch + Worktree ")
        .borders(Borders::ALL)
        .border_style(accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    let query_line = if query.is_empty() {
        Line::from(" ")
    } else {
        Line::from(Span::styled(
            query.to_string(),
            Style::default().fg(Color::White),
        ))
    };
    frame.render_widget(
        Paragraph::new(query_line).block(
            Block::default()
                .title(" Filter ")
                .borders(Borders::ALL)
                .border_style(dim()),
        ),
        chunks[0],
    );

    let filtered = branch_filter_indices(branches, query);
    let items = if filtered.is_empty() {
        state.select(None);
        vec![ListItem::new(Line::from(Span::styled(
            "No matching branches",
            dim(),
        )))]
    } else {
        sync_list_state(state, selected_index, filtered.len());
        filtered
            .iter()
            .map(|branch_idx| {
                let branch = &branches[*branch_idx];
                ListItem::new(Line::from(Span::styled(
                    branch.clone(),
                    Style::default().fg(Color::White),
                )))
            })
            .collect::<Vec<_>>()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Branches {}/{} ", filtered.len(), branches.len()))
                .borders(Borders::ALL)
                .border_style(dim()),
        )
        .highlight_style(selected())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter select  Backspace edit  Esc cancel",
            dim(),
        ))),
        chunks[2],
    );
}

pub(super) fn render_repo_selection_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    repos: &[RepoRef],
    state: &mut ListState,
) {
    let area = centered_rect(68, 60, area);
    frame.render_widget(Clear, area);
    let items = repos
        .iter()
        .map(|repo| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    repo.name.clone(),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(repo.worktree.display().to_string(), dim())),
            ])
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(accent()),
        )
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Red))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, state);
}

pub(super) fn render_confirm_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
) {
    let area = centered_rect(64, 32, area);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn render_input_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    label: &str,
    input: &str,
    help: &str,
) {
    let area = centered_rect(64, 32, area);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(label.to_string(), dim())),
        Line::from(Span::styled(
            if input.is_empty() {
                " ".to_string()
            } else {
                input.to_string()
            },
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(help.to_string(), dim())),
    ];
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(accent());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn metadata_lines(app: &JourneyApp) -> Vec<Line<'static>> {
    let Some(entry) = app.selected_entry() else {
        return vec![
            Line::from(Span::styled("No Journeys", accent())),
            Line::from(""),
            Line::from(Span::styled("Create one with Ctrl-N.", dim())),
        ];
    };

    let journey_path = storage::journey_dir(&app.home, &entry.id);
    let mut lines = vec![Line::from(Span::styled(
        entry.title.clone(),
        accent().add_modifier(Modifier::BOLD),
    ))];
    if let Some(description) = &entry.description {
        lines.push(label_value("description:", description.clone()));
    }
    lines.push(label_value("id:", entry.id.clone()));
    lines.push(Line::from(vec![
        Span::styled("status:", dim()),
        Span::raw(" "),
        Span::styled(entry.status.to_string(), status_style(entry.status)),
    ]));
    lines.push(label_value("updated:", entry.updated.clone()));
    lines.push(label_value("path:", journey_path.display().to_string()));

    let event_count = events::read_events(&journey_path)
        .map(|events| events.len().to_string())
        .unwrap_or_else(|_| "unavailable".to_string());
    lines.push(label_value("events:", event_count));
    lines.push(Line::from(""));
    lines.extend(repo_lines(&app.home, &entry.id));
    lines
}

pub(super) fn tab_bar_line(tabs: &[DocTab], selected: usize, width: usize) -> Line<'static> {
    let sep = " \u{2502} ";
    // Build label positions so we can scroll to keep the selected tab visible
    let mut positions: Vec<(usize, usize)> = Vec::new(); // (start, end) for each tab
    let mut pos = 1usize; // leading space
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            pos += sep.chars().count();
        }
        let start = pos;
        pos += tab.label.len();
        positions.push((start, pos));
    }
    let total_width = pos + 1; // trailing space

    // Determine horizontal scroll offset to keep selected tab visible
    let scroll = if total_width <= width {
        0
    } else if let Some(&(_start, end)) = positions.get(selected) {
        // Ensure the selected tab fits in the viewport
        if end > width {
            // Scroll right so the selected tab's end is at the right edge
            (end.saturating_sub(width)).min(total_width.saturating_sub(width))
        } else {
            0
        }
    } else {
        0
    };

    let mut spans = Vec::new();
    let mut col = 0usize;

    let push_visible =
        |text: String, style: Style, col: &mut usize, spans: &mut Vec<Span<'static>>| {
            let len = text.len();
            let start = *col;
            let end = start + len;
            *col = end;

            if end <= scroll || start >= scroll + width {
                return; // entirely outside viewport
            }

            let visible_start = scroll.saturating_sub(start);
            let visible_end = if end > scroll + width {
                scroll + width - start
            } else {
                len
            };
            let visible: String = text
                .chars()
                .skip(visible_start)
                .take(visible_end - visible_start)
                .collect();
            if !visible.is_empty() {
                spans.push(Span::styled(visible, style));
            }
        };

    push_visible(" ".to_string(), Style::default(), &mut col, &mut spans);
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            push_visible(sep.to_string(), dim(), &mut col, &mut spans);
        }
        let style = if i == selected {
            accent().add_modifier(Modifier::BOLD)
        } else {
            dim()
        };
        push_visible(tab.label.clone(), style, &mut col, &mut spans);
    }
    push_visible(" ".to_string(), Style::default(), &mut col, &mut spans);

    // Show scroll indicators
    if scroll > 0 {
        spans.insert(0, Span::styled("\u{25c2}", dim()));
    }
    if scroll + width < total_width {
        spans.push(Span::styled("\u{25b8}", dim()));
    }

    Line::from(spans)
}

pub(super) fn doc_content_lines(app: &mut JourneyApp, tabs: &[DocTab]) -> Vec<Line<'static>> {
    if tabs.is_empty() {
        return Vec::new();
    }
    let idx = app.doc_tab_index.min(tabs.len().saturating_sub(1));
    app.doc_render_cache.lines(&tabs[idx].path)
}

pub(super) fn repo_lines(home: &Path, id: &str) -> Vec<Line<'static>> {
    let journey_path = storage::journey_dir(home, id);
    let Ok(journey) = storage::load_journey(&journey_path) else {
        return vec![Line::from(Span::styled("repos: unavailable", dim()))];
    };
    if journey.repos.is_empty() {
        return vec![Line::from(vec![
            Span::styled("repos:", dim()),
            Span::raw(" none"),
        ])];
    }

    let mut lines = vec![Line::from(Span::styled("repos:", dim()))];
    for repo in journey.repos {
        lines.push(Line::from(vec![
            Span::raw("- "),
            Span::styled(repo.name, accent()),
            Span::raw("  "),
            Span::styled(repo.branch, dim()),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(repo.worktree.display().to_string(), dim()),
        ]));
    }
    lines
}

pub(super) fn label_value(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(label.to_string(), dim()),
        Span::raw(" "),
        Span::raw(value),
    ])
}

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

pub(super) fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub(super) fn heading_style(level: HeadingLevel) -> Style {
    let style = accent().add_modifier(Modifier::BOLD);
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => style.add_modifier(Modifier::UNDERLINED),
        _ => style,
    }
}

pub(super) fn blockquote_style() -> Style {
    dim().add_modifier(Modifier::ITALIC)
}

pub(super) fn inline_code() -> Style {
    Style::default().fg(Color::Yellow)
}

pub(super) fn code_block_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub(super) fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub(super) fn selected() -> Style {
    Style::default().fg(ACCENT_TEXT).bg(ACCENT)
}

pub(super) fn selected_row() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub(super) fn selection_stroke() -> Text<'static> {
    Text::from(Line::from(vec![
        Span::styled("|", accent()),
        Span::raw(" "),
    ]))
}

pub(super) fn status_style(status: JourneyStatus) -> Style {
    match status {
        JourneyStatus::Active => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        JourneyStatus::Paused => Style::default().fg(Color::Yellow),
        JourneyStatus::Archived => Style::default().fg(Color::DarkGray),
        JourneyStatus::Abandoned => Style::default().fg(Color::Red),
    }
}
impl TuiAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            TuiAction::CopyCd => "Copy cd to path",
            TuiAction::OpenNvim => "Open nvim in Journey folder",
            TuiAction::OpenClaude => "Open Claude Code in Journey folder",
            TuiAction::Resume => "Resume",
            TuiAction::NewBranchWorktree => "New branch + worktree",
            TuiAction::ExistingBranchWorktree => "Existing branch + worktree",
            TuiAction::LinkCurrent => "Link current worktree",
            TuiAction::UnlinkRepo => "Unlink a repo",
            TuiAction::Capture => "Capture thought",
            TuiAction::DeleteWorktree => "Delete worktree",
            TuiAction::Done => "Done",
            TuiAction::Pause => "Pause",
            TuiAction::Archive => "Archive",
            TuiAction::Abandon => "Abandon",
        }
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            TuiAction::CopyCd => "copy cd command",
            TuiAction::OpenNvim => "cd journey + run nvim",
            TuiAction::OpenClaude => "cd journey + run claude",
            TuiAction::Resume => "mark active",
            TuiAction::NewBranchWorktree => "git worktree add -b",
            TuiAction::ExistingBranchWorktree => "select branch",
            TuiAction::LinkCurrent => "attach cwd repo",
            TuiAction::UnlinkRepo => "detach linked repo",
            TuiAction::Capture => "append docs/capture.md",
            TuiAction::DeleteWorktree => "git remove + unlink",
            TuiAction::Done => "archive + remove worktrees",
            TuiAction::Pause => "lifecycle only",
            TuiAction::Archive => "release worktrees",
            TuiAction::Abandon => "release worktrees",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::widgets::StatefulWidget;

    #[test]
    fn counts_wrapped_detail_lines() {
        let lines = vec![Line::from("abcdef"), Line::from("")];

        assert_eq!(wrapped_line_count(&lines, 3), 3);
        assert_eq!(wrapped_line_count(&lines, 0), 7);
    }

    #[test]
    fn details_scrollbar_reaches_bottom_at_max_scroll() {
        let viewport_height = 10;
        let content_height = 30;
        let max_scroll = content_height - viewport_height;
        let mut state = ScrollbarState::new(details_scrollbar_content_length(max_scroll))
            .position(max_scroll)
            .viewport_content_length(viewport_height);
        let area = Rect::new(0, 0, 1, viewport_height as u16);
        let mut buffer = Buffer::empty(area);

        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .render(area, &mut buffer, &mut state);

        assert_eq!(
            buffer
                .cell((0, viewport_height as u16 - 1))
                .expect("bottom scrollbar cell")
                .symbol(),
            "\u{2588}"
        );
    }

    #[test]
    fn relative_time_formats_durations() {
        assert_eq!(relative_time(Duration::from_secs(2)), "just now");
        assert_eq!(relative_time(Duration::from_secs(30)), "30s ago");
        assert_eq!(relative_time(Duration::from_secs(90)), "1m ago");
        assert_eq!(relative_time(Duration::from_secs(3600)), "60m ago");
    }

    #[test]
    fn notification_prune_removes_expired_toasts() {
        let now = Instant::now();
        let mut toasts: VecDeque<Notification> = VecDeque::new();
        toasts.push_back(Notification {
            message: "old".into(),
            level: NotificationLevel::Success,
            created_at: now - Duration::from_secs(5),
        });
        toasts.push_back(Notification {
            message: "new".into(),
            level: NotificationLevel::Success,
            created_at: now,
        });

        toasts.retain(|notification| notification.created_at.elapsed() < Duration::from_secs(3));

        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].message, "new");
    }
}
