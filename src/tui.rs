use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::DefaultTerminal;
use std::sync::mpsc;
use std::time::Duration;
use tui_textarea::{Input, TextArea};

use crate::api::{DirectMessage, DirectThread, InstagramClient};
use crate::config::{ConfigStore, DmCache};

// ── Worker ──────────────────────────────────────────────────────────────────

pub enum WorkerCommand {
    PublishNote(String),
    FetchThreads,
    FetchMessages(String), // thread_id
    SendDM(String, String), // thread_id, text
    PollThreads,            // background poll — same as FetchThreads but tagged differently
}

pub enum WorkerEvent {
    NotePublished(Result<String>),
    ThreadsFetched(Result<Vec<DirectThread>>),
    MessagesFetched(String, Result<(Vec<DirectMessage>, String)>), // thread_id, result
    DMSent(Result<()>),
    PollResult(Result<Vec<DirectThread>>),
}

fn worker_loop(
    mut api: InstagramClient,
    _store: ConfigStore,
    cmd_rx: mpsc::Receiver<WorkerCommand>,
    evt_tx: mpsc::Sender<WorkerEvent>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            WorkerCommand::PublishNote(text) => {
                let result = api.create_note(&text);
                let _ = evt_tx.send(WorkerEvent::NotePublished(result));
            }
            WorkerCommand::FetchThreads => {
                let result = api.get_direct_threads(20);
                let _ = evt_tx.send(WorkerEvent::ThreadsFetched(result));
            }
            WorkerCommand::FetchMessages(thread_id) => {
                let result = api.get_thread_messages(&thread_id, 20);
                let _ = evt_tx.send(WorkerEvent::MessagesFetched(thread_id, result));
            }
            WorkerCommand::PollThreads => {
                let result = api.get_direct_threads(20);
                let _ = evt_tx.send(WorkerEvent::PollResult(result));
            }
            WorkerCommand::SendDM(thread_id, text) => {
                let result = api.send_dm(&thread_id, &text);
                let ok = result.is_ok();
                let _ = evt_tx.send(WorkerEvent::DMSent(result));
                // Only refresh if send succeeded
                if ok {
                    let result = api.get_thread_messages(&thread_id, 20);
                    let _ = evt_tx.send(WorkerEvent::MessagesFetched(thread_id, result));
                }
            }
        }
    }
}

// ── Screens ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Home,
    Notes,
    DMList,
    DMThread(String), // thread_id
    Login,
}

// ── App state ───────────────────────────────────────────────────────────────

pub struct App<'a> {
    screen: Screen,
    username: String,
    status: String,

    // Notes
    textarea: TextArea<'a>,

    // Login
    login_field: usize, // 0=user, 1=pass
    login_username: String,
    login_password: String,
    login_status: String,

    // DMs
    unread: std::collections::HashSet<String>, // thread_ids with new messages
    threads: Vec<DirectThread>,
    thread_list_state: ListState,
    messages: Vec<DirectMessage>,
    message_cache: std::collections::HashMap<String, (Vec<DirectMessage>, std::time::Instant)>,
    current_thread_id: String,
    current_thread_title: String,
    dm_input: String,
    dm_scroll: u16,

    // Worker
    cmd_tx: mpsc::Sender<WorkerCommand>,
}

const NOTE_CHAR_LIMIT: usize = 60;

impl<'a> App<'a> {
    fn new(
        username: String,
        cmd_tx: mpsc::Sender<WorkerCommand>,
    ) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(Block::default().borders(Borders::ALL).title(" Compose "));
        textarea.set_placeholder_text("Write a note (60 char max). Ctrl-S publishes. Esc goes back.");

        // Prefetch DMs
        let _ = cmd_tx.send(WorkerCommand::FetchThreads);

        Self {
            screen: Screen::Home,
            username,
            status: String::new(),

            textarea,

            login_field: 0,
            login_username: String::new(),
            login_password: String::new(),
            login_status: String::new(),

            unread: std::collections::HashSet::new(),
            threads: Vec::new(),
            thread_list_state: ListState::default(),
            messages: Vec::new(),
            message_cache: std::collections::HashMap::new(),
            current_thread_id: String::new(),
            current_thread_title: String::new(),
            dm_input: String::new(),
            dm_scroll: 0,

            cmd_tx,
        }
    }

    fn handle_worker_event(&mut self, evt: WorkerEvent) {
        match evt {
            WorkerEvent::NotePublished(Ok(id)) => {
                self.status = format!("published (id: {})", id);
                self.textarea.select_all();
                self.textarea.cut();
            }
            WorkerEvent::NotePublished(Err(e)) => {
                self.status = format!("error: {}", e);
            }
            WorkerEvent::ThreadsFetched(Ok(threads)) => {
                self.threads = threads;
                if self.screen == Screen::DMList {
                    self.status = format!("{} conversations", self.threads.len());
                }
                // Prefetch messages for all threads that aren't cached
                for t in &self.threads {
                    if !self.message_cache.contains_key(&t.thread_id) {
                        let _ = self.cmd_tx.send(WorkerCommand::FetchMessages(t.thread_id.clone()));
                    }
                }
            }
            WorkerEvent::ThreadsFetched(Err(e)) => {
                self.status = format!("error: {}", e);
            }
            WorkerEvent::MessagesFetched(tid, Ok((msgs, title))) => {
                self.message_cache.insert(tid.clone(), (msgs.clone(), std::time::Instant::now()));
                // Only update the visible screen if this is the active thread
                if tid == self.current_thread_id {
                    self.messages = msgs;
                    if !title.is_empty() {
                        self.current_thread_title = title;
                    }
                    self.dm_scroll = 0;
                    self.status = format!(
                        "{}  {} msgs",
                        self.current_thread_title,
                        self.messages.len()
                    );
                }
            }
            WorkerEvent::MessagesFetched(_tid, Err(e)) => {
                self.status = format!("error: {}", e);
            }
            WorkerEvent::PollResult(Ok(new_threads)) => {
                // Compare last messages to find new activity
                let old_msgs: std::collections::HashMap<String, String> = self
                    .threads
                    .iter()
                    .map(|t| (t.thread_id.clone(), t.last_message.clone()))
                    .collect();
                for t in &new_threads {
                    if let Some(old_msg) = old_msgs.get(&t.thread_id) {
                        if *old_msg != t.last_message {
                            self.unread.insert(t.thread_id.clone());
                        }
                    } else {
                        // New thread we haven't seen
                        self.unread.insert(t.thread_id.clone());
                    }
                }
                self.threads = new_threads;
            }
            WorkerEvent::PollResult(Err(_)) => {}
            WorkerEvent::DMSent(Ok(())) => {
                self.status = format!("{}  sent!", self.current_thread_title);
                self.dm_input.clear();
            }
            WorkerEvent::DMSent(Err(e)) => {
                self.status = format!("error: {}", e);
            }
        }
    }
}

// ── Draw ────────────────────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    match &app.screen {
        Screen::Home => draw_home(frame, app),
        Screen::Notes => draw_notes(frame, app),
        Screen::DMList => draw_dm_list(frame, app),
        Screen::DMThread(_) => draw_dm_thread(frame, app),
        Screen::Login => draw_login(frame, app),
    }
}

fn draw_home(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let welcome = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  inote",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  [N] notes    [D] dms    [Q] quit"),
    ]);
    frame.render_widget(welcome, chunks[0]);

    let status = Paragraph::new(format!("  @{}", app.username))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);

    let footer = Paragraph::new("  n notes | d dms | q quit")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

fn draw_notes(frame: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(2),
            Constraint::Length(3),
        ])
        .split(frame.area());

    // Textarea
    let textarea_area = centered_rect(90, chunks[0]);
    frame.render_widget(&app.textarea, textarea_area);

    // Meta line
    let text = app.textarea.lines().join("\n");
    let count = text.len();
    let remaining = NOTE_CHAR_LIMIT as i32 - count as i32;

    let color = if remaining < 0 {
        Color::Red
    } else if remaining < 10 {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let left_str = if remaining < 0 {
        format!("{} over", remaining.unsigned_abs())
    } else {
        format!("{} left", remaining)
    };

    let meta = Paragraph::new(format!(
        "  {} chars  {}  |  {}",
        count, left_str, app.status
    ))
    .style(Style::default().fg(color));
    frame.render_widget(meta, chunks[1]);

    // Footer
    let footer = Paragraph::new("  Ctrl-S publish | Ctrl-L clear | Esc back")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

fn draw_dm_list(frame: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(2),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let items: Vec<ListItem> = app
        .threads
        .iter()
        .map(|t| {
            let is_unread = app.unread.contains(&t.thread_id);
            let title = if is_unread {
                format!("* {}", t.thread_title)
            } else {
                t.thread_title.clone()
            };
            let title_style = if is_unread {
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let preview = if t.last_message.chars().count() > 60 {
                let truncated: String = t.last_message.chars().take(60).collect();
                format!("{}...", truncated)
            } else {
                t.last_message.clone()
            };
            ListItem::new(vec![
                Line::from(Span::styled(
                    title,
                    title_style,
                )),
                Line::from(Span::styled(preview, Style::default().fg(Color::DarkGray))),
            ])
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" DMs ")
                .padding(ratatui::widgets::Padding::horizontal(1)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let list_area = centered_rect(90, chunks[0]);
    frame.render_stateful_widget(list, list_area, &mut app.thread_list_state);

    // Status
    let status = Paragraph::new(format!("  {}", app.status))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);

    // Footer
    let footer = Paragraph::new("  Enter open | R refresh | Esc back")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

fn draw_dm_thread(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(3),
        ])
        .split(frame.area());

    // Messages (reversed — newest at bottom)
    let msg_lines: Vec<Line> = app
        .messages
        .iter()
        .rev()
        .flat_map(|m| {
            let style = if m.is_sender {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            vec![
                Line::from(Span::styled(
                    format!("{}:", m.user_id),
                    style.add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(&*m.text, style)),
                Line::from(""),
            ]
        })
        .collect();

    let total_lines = msg_lines.len() as u16;
    let visible = chunks[0].height.saturating_sub(2);
    let scroll = if total_lines > visible {
        total_lines - visible - app.dm_scroll
    } else {
        0
    };

    let messages = Paragraph::new(msg_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", app.current_thread_title))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    let msg_area = centered_rect(90, chunks[0]);
    frame.render_widget(messages, msg_area);

    // Reply input
    let input = Paragraph::new(format!(" {}", app.dm_input))
        .block(Block::default().borders(Borders::ALL).title(" reply "));
    let input_area = centered_rect(90, chunks[1]);
    frame.render_widget(input, input_area);

    // Status
    let status_color = if app.status.contains("sending") || app.status.contains("loading") {
        Color::Yellow
    } else if app.status.contains("sent!") {
        Color::Green
    } else if app.status.contains("error") {
        Color::Red
    } else {
        Color::DarkGray
    };
    let status = Paragraph::new(format!("  {}", app.status))
        .style(Style::default().fg(status_color));
    frame.render_widget(status, chunks[2]);

    // Footer
    let footer = Paragraph::new("  Enter send | R refresh | Esc back")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[3]);
}

fn draw_login(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(2),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let user_style = if app.login_field == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let pass_style = if app.login_field == 1 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let password_display = "*".repeat(app.login_password.len());

    let login_form = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  login",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  username: "),
            Span::styled(&app.login_username, user_style),
            if app.login_field == 0 {
                Span::styled("_", user_style)
            } else {
                Span::raw("")
            },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  password: "),
            Span::styled(&password_display, pass_style),
            if app.login_field == 1 {
                Span::styled("_", pass_style)
            } else {
                Span::raw("")
            },
        ]),
    ]);
    frame.render_widget(login_form, chunks[0]);

    let status = Paragraph::new(format!("  {}", app.login_status))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);

    let footer = Paragraph::new("  Tab switch field | Enter login | Esc quit")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

fn centered_rect(percent_x: u16, area: Rect) -> Rect {
    let margin = (100 - percent_x) / 2;
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(margin),
        ])
        .split(area)[1]
}

// ── Input handling ──────────────────────────────────────────────────────────

fn handle_input(app: &mut App, key: event::KeyEvent) -> bool {
    match &app.screen {
        Screen::Home => handle_home_input(app, key),
        Screen::Notes => handle_notes_input(app, key),
        Screen::DMList => handle_dm_list_input(app, key),
        Screen::DMThread(_) => handle_dm_thread_input(app, key),
        Screen::Login => handle_login_input(app, key),
    }
}

fn handle_home_input(app: &mut App, key: event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('n') => {
            app.screen = Screen::Notes;
            app.status.clear();
        }
        KeyCode::Char('d') => {
            app.screen = Screen::DMList;
            app.status = format!("{} conversations", app.threads.len());
            if app.threads.is_empty() {
                app.status = "loading...".to_string();
                let _ = app.cmd_tx.send(WorkerCommand::FetchThreads);
            }
            if !app.threads.is_empty() && app.thread_list_state.selected().is_none() {
                app.thread_list_state.select(Some(0));
            }
        }
        _ => {}
    }
    false
}

fn handle_notes_input(app: &mut App, key: event::KeyEvent) -> bool {
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc) => {
            app.screen = Screen::Home;
            app.status.clear();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
            let text: String = app.textarea.lines().join("\n");
            let text = text.trim().to_string();
            if text.is_empty() {
                app.status = "empty".to_string();
            } else if text.len() > NOTE_CHAR_LIMIT {
                app.status = format!("too long: {}/{}", text.len(), NOTE_CHAR_LIMIT);
            } else {
                app.status = "publishing...".to_string();
                let _ = app.cmd_tx.send(WorkerCommand::PublishNote(text));
                // Clear immediately so you can keep working
                app.textarea.select_all();
                app.textarea.cut();
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            app.textarea.select_all();
            app.textarea.cut();
            app.status.clear();
        }
        _ => {
            app.textarea.input(Input::from(key));
        }
    }
    false
}

fn handle_dm_list_input(app: &mut App, key: event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Home;
            app.status.clear();
        }
        KeyCode::Char('r') => {
            app.status = "loading...".to_string();
            let _ = app.cmd_tx.send(WorkerCommand::FetchThreads);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.thread_list_state.selected().unwrap_or(0);
            if i > 0 {
                app.thread_list_state.select(Some(i - 1));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let i = app.thread_list_state.selected().unwrap_or(0);
            if i + 1 < app.threads.len() {
                app.thread_list_state.select(Some(i + 1));
            }
        }
        KeyCode::Enter => {
            if let Some(i) = app.thread_list_state.selected() {
                if let Some(thread) = app.threads.get(i) {
                    let tid = thread.thread_id.clone();
                    app.current_thread_title = thread.thread_title.clone();
                    app.current_thread_id = tid.clone();
                    app.unread.remove(&tid);
                    app.dm_input.clear();
                    app.dm_scroll = 0;
                    app.screen = Screen::DMThread(tid.clone());

                    // Show cached messages, only refetch if stale (>30s)
                    let cache_ttl = std::time::Duration::from_secs(30);
                    if let Some((cached, fetched_at)) = app.message_cache.get(&tid) {
                        app.messages = cached.clone();
                        if fetched_at.elapsed() < cache_ttl {
                            app.status = format!(
                                "{}  {} msgs",
                                thread.thread_title,
                                cached.len()
                            );
                        } else {
                            app.status = format!(
                                "{}  {} msgs (refreshing...)",
                                thread.thread_title,
                                cached.len()
                            );
                            let _ = app.cmd_tx.send(WorkerCommand::FetchMessages(tid));
                        }
                    } else {
                        app.messages.clear();
                        app.status = format!("{}  loading...", thread.thread_title);
                        let _ = app.cmd_tx.send(WorkerCommand::FetchMessages(tid));
                    }
                }
            }
        }
        _ => {}
    }
    false
}

fn handle_dm_thread_input(app: &mut App, key: event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::DMList;
            app.status = format!("{} conversations", app.threads.len());
        }
        KeyCode::Char('r') if app.dm_input.is_empty() => {
            app.status = format!("{}  loading...", app.current_thread_title);
            let _ = app
                .cmd_tx
                .send(WorkerCommand::FetchMessages(app.current_thread_id.clone()));
        }
        KeyCode::Enter => {
            let text = app.dm_input.trim().to_string();
            if !text.is_empty() {
                // Optimistically add message to display
                app.messages.push(DirectMessage {
                    user_id: "you".to_string(),
                    text: text.clone(),
                    timestamp: String::new(),
                    is_sender: true,
                });
                app.dm_scroll = 0;
                app.status = format!("{}  sending...", app.current_thread_title);
                let _ = app.cmd_tx.send(WorkerCommand::SendDM(
                    app.current_thread_id.clone(),
                    text,
                ));
                app.dm_input.clear();
            }
        }
        KeyCode::Backspace => {
            app.dm_input.pop();
        }
        KeyCode::Char(c) => {
            app.dm_input.push(c);
        }
        KeyCode::Up => {
            app.dm_scroll = app.dm_scroll.saturating_add(1);
        }
        KeyCode::Down => {
            app.dm_scroll = app.dm_scroll.saturating_sub(1);
        }
        _ => {}
    }
    false
}

fn handle_login_input(app: &mut App, key: event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => return true,
        KeyCode::Tab => {
            app.login_field = (app.login_field + 1) % 2;
        }
        KeyCode::Enter => {
            // Login is handled synchronously in the main loop since we need
            // to update the session before spawning the worker
            app.login_status = "login_requested".to_string();
        }
        KeyCode::Backspace => {
            if app.login_field == 0 {
                app.login_username.pop();
            } else {
                app.login_password.pop();
            }
        }
        KeyCode::Char(c) => {
            if app.login_field == 0 {
                app.login_username.push(c);
            } else {
                app.login_password.push(c);
            }
        }
        _ => {}
    }
    false
}

// ── Run ─────────────────────────────────────────────────────────────────────

pub fn run(
    terminal: &mut DefaultTerminal,
    api: InstagramClient,
    store: ConfigStore,
    username: String,
) -> Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();
    let (evt_tx, evt_rx) = mpsc::channel::<WorkerEvent>();

    let mut app = App::new(username, cmd_tx);

    // Load disk cache
    if let Some(cache) = store.load_cache() {
        app.threads = cache.threads;
        for (tid, msgs) in cache.messages {
            app.message_cache.insert(tid, (msgs, std::time::Instant::now() - std::time::Duration::from_secs(999)));
        }
    }

    // Prefetch DMs (will replace stale cache)
    let _ = app.cmd_tx.send(WorkerCommand::FetchThreads);

    let save_store = ConfigStore::new().ok();
    std::thread::spawn(move || {
        worker_loop(api, store, cmd_rx, evt_tx);
    });

    let mut last_poll = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_secs(30);

    loop {
        terminal
            .draw(|frame| draw(frame, &mut app))
            .context("failed to draw frame")?;

        while let Ok(evt) = evt_rx.try_recv() {
            app.handle_worker_event(evt);
        }

        // Background poll for new messages
        if last_poll.elapsed() >= poll_interval {
            let _ = app.cmd_tx.send(WorkerCommand::PollThreads);
            last_poll = std::time::Instant::now();
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    let quit = handle_input(&mut app, key);
                    if quit {
                        // Save cache to disk on quit
                        if let Some(ref s) = save_store {
                            let cache = DmCache {
                                threads: app.threads.clone(),
                                messages: app.message_cache.iter()
                                    .map(|(k, (v, _))| (k.clone(), v.clone()))
                                    .collect(),
                            };
                            let _ = s.save_cache(&cache);
                        }
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_ctrl_key(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn test_app() -> App<'static> {
        let (cmd_tx, _cmd_rx) = mpsc::channel();
        App::new("testuser".to_string(), cmd_tx)
    }

    // ── Home screen ─────────────────────────────────────────────────────

    #[test]
    fn home_starts_on_home_screen() {
        let app = test_app();
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn home_n_goes_to_notes() {
        let mut app = test_app();
        handle_input(&mut app, make_key(KeyCode::Char('n')));
        assert_eq!(app.screen, Screen::Notes);
    }

    #[test]
    fn home_d_goes_to_dms() {
        let mut app = test_app();
        handle_input(&mut app, make_key(KeyCode::Char('d')));
        assert_eq!(app.screen, Screen::DMList);
    }

    #[test]
    fn home_q_returns_quit() {
        let mut app = test_app();
        let quit = handle_input(&mut app, make_key(KeyCode::Char('q')));
        assert!(quit);
    }

    #[test]
    fn home_esc_returns_quit() {
        let mut app = test_app();
        let quit = handle_input(&mut app, make_key(KeyCode::Esc));
        assert!(quit);
    }

    #[test]
    fn home_random_key_does_nothing() {
        let mut app = test_app();
        let quit = handle_input(&mut app, make_key(KeyCode::Char('x')));
        assert!(!quit);
        assert_eq!(app.screen, Screen::Home);
    }

    // ── Notes screen ────────────────────────────────────────────────────

    #[test]
    fn notes_esc_goes_home() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        handle_input(&mut app, make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn notes_ctrl_s_empty_sets_status() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        handle_input(&mut app, make_ctrl_key('s'));
        assert_eq!(app.status, "empty");
    }

    #[test]
    fn notes_ctrl_l_clears_textarea() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        handle_input(&mut app, make_key(KeyCode::Char('h')));
        handle_input(&mut app, make_key(KeyCode::Char('i')));
        assert!(!app.textarea.lines().join("").is_empty());
        handle_input(&mut app, make_ctrl_key('l'));
        assert!(app.textarea.lines().join("").is_empty());
    }

    #[test]
    fn notes_typing_adds_to_textarea() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        handle_input(&mut app, make_key(KeyCode::Char('a')));
        handle_input(&mut app, make_key(KeyCode::Char('b')));
        assert_eq!(app.textarea.lines().join(""), "ab");
    }

    #[test]
    fn notes_ctrl_s_too_long_sets_error() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        for _ in 0..61 {
            handle_input(&mut app, make_key(KeyCode::Char('x')));
        }
        handle_input(&mut app, make_ctrl_key('s'));
        assert!(app.status.contains("too long"));
    }

    // ── DM list screen ──────────────────────────────────────────────────

    #[test]
    fn dm_list_esc_goes_home() {
        let mut app = test_app();
        app.screen = Screen::DMList;
        handle_input(&mut app, make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn dm_list_j_k_navigation() {
        let mut app = test_app();
        app.screen = Screen::DMList;
        app.threads = vec![
            DirectThread {
                thread_id: "1".to_string(),
                thread_title: "Alice".to_string(),
                usernames: vec!["alice".to_string()],
                last_message: "hi".to_string(),
            },
            DirectThread {
                thread_id: "2".to_string(),
                thread_title: "Bob".to_string(),
                usernames: vec!["bob".to_string()],
                last_message: "yo".to_string(),
            },
        ];
        app.thread_list_state.select(Some(0));

        handle_input(&mut app, make_key(KeyCode::Char('j')));
        assert_eq!(app.thread_list_state.selected(), Some(1));

        handle_input(&mut app, make_key(KeyCode::Char('k')));
        assert_eq!(app.thread_list_state.selected(), Some(0));

        handle_input(&mut app, make_key(KeyCode::Char('k')));
        assert_eq!(app.thread_list_state.selected(), Some(0));
    }

    #[test]
    fn dm_list_j_doesnt_overflow() {
        let mut app = test_app();
        app.screen = Screen::DMList;
        app.threads = vec![DirectThread {
            thread_id: "1".to_string(),
            thread_title: "Alice".to_string(),
            usernames: vec![],
            last_message: String::new(),
        }];
        app.thread_list_state.select(Some(0));

        handle_input(&mut app, make_key(KeyCode::Char('j')));
        assert_eq!(app.thread_list_state.selected(), Some(0));
    }

    #[test]
    fn dm_list_enter_opens_thread() {
        let mut app = test_app();
        app.screen = Screen::DMList;
        app.threads = vec![DirectThread {
            thread_id: "thread_42".to_string(),
            thread_title: "Alice".to_string(),
            usernames: vec!["alice".to_string()],
            last_message: "hi".to_string(),
        }];
        app.thread_list_state.select(Some(0));

        handle_input(&mut app, make_key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::DMThread("thread_42".to_string()));
        assert_eq!(app.current_thread_id, "thread_42");
        assert_eq!(app.current_thread_title, "Alice");
    }

    // ── DM thread screen ────────────────────────────────────────────────

    #[test]
    fn dm_thread_esc_goes_to_list() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        handle_input(&mut app, make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::DMList);
    }

    #[test]
    fn dm_thread_typing_fills_input() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        handle_input(&mut app, make_key(KeyCode::Char('h')));
        handle_input(&mut app, make_key(KeyCode::Char('i')));
        assert_eq!(app.dm_input, "hi");
    }

    #[test]
    fn dm_thread_backspace_removes_char() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        app.dm_input = "hello".to_string();
        handle_input(&mut app, make_key(KeyCode::Backspace));
        assert_eq!(app.dm_input, "hell");
    }

    #[test]
    fn dm_thread_enter_empty_does_nothing() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        app.dm_input.clear();
        handle_input(&mut app, make_key(KeyCode::Enter));
        assert!(!app.status.contains("sending"));
    }

    #[test]
    fn dm_thread_scroll_up_down() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        assert_eq!(app.dm_scroll, 0);

        handle_input(&mut app, make_key(KeyCode::Up));
        assert_eq!(app.dm_scroll, 1);
        handle_input(&mut app, make_key(KeyCode::Up));
        assert_eq!(app.dm_scroll, 2);
        handle_input(&mut app, make_key(KeyCode::Down));
        assert_eq!(app.dm_scroll, 1);
        handle_input(&mut app, make_key(KeyCode::Down));
        handle_input(&mut app, make_key(KeyCode::Down));
        assert_eq!(app.dm_scroll, 0);
    }

    #[test]
    fn dm_thread_r_refreshes_only_when_input_empty() {
        let mut app = test_app();
        app.screen = Screen::DMThread("t1".to_string());
        app.current_thread_title = "Test".to_string();

        handle_input(&mut app, make_key(KeyCode::Char('r')));
        assert!(app.status.contains("loading"));

        app.status.clear();
        app.dm_input = "some text".to_string();
        handle_input(&mut app, make_key(KeyCode::Char('r')));
        assert_eq!(app.dm_input, "some textr");
        assert!(app.status.is_empty());
    }

    // ── Login screen ────────────────────────────────────────────────────

    #[test]
    fn login_esc_quits() {
        let mut app = test_app();
        app.screen = Screen::Login;
        let quit = handle_input(&mut app, make_key(KeyCode::Esc));
        assert!(quit);
    }

    #[test]
    fn login_tab_switches_field() {
        let mut app = test_app();
        app.screen = Screen::Login;
        assert_eq!(app.login_field, 0);
        handle_input(&mut app, make_key(KeyCode::Tab));
        assert_eq!(app.login_field, 1);
        handle_input(&mut app, make_key(KeyCode::Tab));
        assert_eq!(app.login_field, 0);
    }

    #[test]
    fn login_typing_fills_correct_field() {
        let mut app = test_app();
        app.screen = Screen::Login;
        handle_input(&mut app, make_key(KeyCode::Char('u')));
        assert_eq!(app.login_username, "u");
        assert!(app.login_password.is_empty());

        handle_input(&mut app, make_key(KeyCode::Tab));
        handle_input(&mut app, make_key(KeyCode::Char('p')));
        assert_eq!(app.login_username, "u");
        assert_eq!(app.login_password, "p");
    }

    #[test]
    fn login_backspace_removes_from_correct_field() {
        let mut app = test_app();
        app.screen = Screen::Login;
        app.login_username = "user".to_string();
        app.login_password = "pass".to_string();

        app.login_field = 0;
        handle_input(&mut app, make_key(KeyCode::Backspace));
        assert_eq!(app.login_username, "use");

        app.login_field = 1;
        handle_input(&mut app, make_key(KeyCode::Backspace));
        assert_eq!(app.login_password, "pas");
    }

    #[test]
    fn login_enter_sets_login_requested() {
        let mut app = test_app();
        app.screen = Screen::Login;
        handle_input(&mut app, make_key(KeyCode::Enter));
        assert_eq!(app.login_status, "login_requested");
    }

    // ── Worker event handling ───────────────────────────────────────────

    #[test]
    fn worker_event_note_published_clears_textarea() {
        let mut app = test_app();
        app.screen = Screen::Notes;
        handle_input(&mut app, make_key(KeyCode::Char('x')));

        app.handle_worker_event(WorkerEvent::NotePublished(Ok("note123".to_string())));
        assert!(app.status.contains("note123"));
        assert!(app.textarea.lines().join("").is_empty());
    }

    #[test]
    fn worker_event_note_error() {
        let mut app = test_app();
        app.handle_worker_event(WorkerEvent::NotePublished(Err(anyhow::anyhow!("boom"))));
        assert!(app.status.contains("boom"));
    }

    #[test]
    fn worker_event_threads_fetched() {
        let mut app = test_app();
        app.screen = Screen::DMList;
        let threads = vec![
            DirectThread { thread_id: "1".into(), thread_title: "A".into(), usernames: vec![], last_message: String::new() },
            DirectThread { thread_id: "2".into(), thread_title: "B".into(), usernames: vec![], last_message: String::new() },
        ];
        app.handle_worker_event(WorkerEvent::ThreadsFetched(Ok(threads)));
        assert_eq!(app.threads.len(), 2);
        assert!(app.status.contains("2 conversations"));
    }

    #[test]
    fn worker_event_messages_fetched() {
        let mut app = test_app();
        let msgs = vec![DirectMessage {
            user_id: "alice".into(), text: "hi".into(), timestamp: String::new(), is_sender: false,
        }];
        app.current_thread_id = "t1".into();
        app.handle_worker_event(WorkerEvent::MessagesFetched("t1".into(), Ok((msgs, "Alice".into()))));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.current_thread_title, "Alice");
    }

    #[test]
    fn worker_event_dm_sent_ok() {
        let mut app = test_app();
        app.current_thread_title = "Alice".into();
        app.dm_input = "old".into();
        app.handle_worker_event(WorkerEvent::DMSent(Ok(())));
        assert!(app.dm_input.is_empty());
        assert!(app.status.contains("sent!"));
    }

    #[test]
    fn worker_event_dm_sent_error() {
        let mut app = test_app();
        app.handle_worker_event(WorkerEvent::DMSent(Err(anyhow::anyhow!("network"))));
        assert!(app.status.contains("network"));
    }

    #[test]
    fn app_stores_username() {
        let app = test_app();
        assert_eq!(app.username, "testuser");
    }
}
