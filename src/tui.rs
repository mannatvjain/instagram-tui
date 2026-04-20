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
use crate::config::{ConfigStore, SessionConfig};

// ── Worker ──────────────────────────────────────────────────────────────────

pub enum WorkerCommand {
    PublishNote(String),
    FetchThreads,
    FetchMessages(String), // thread_id
    SendDM(String, String), // thread_id, text
}

pub enum WorkerEvent {
    NotePublished(Result<String>),
    ThreadsFetched(Result<Vec<DirectThread>>),
    MessagesFetched(Result<(Vec<DirectMessage>, String)>),
    DMSent(Result<()>),
}

fn worker_loop(
    api: InstagramClient,
    _store: ConfigStore,
    session: SessionConfig,
    cmd_rx: mpsc::Receiver<WorkerCommand>,
    evt_tx: mpsc::Sender<WorkerEvent>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            WorkerCommand::PublishNote(text) => {
                let result = api.create_note(&session, &text);
                let _ = evt_tx.send(WorkerEvent::NotePublished(result));
            }
            WorkerCommand::FetchThreads => {
                let result = api.get_direct_threads(&session, 20);
                let _ = evt_tx.send(WorkerEvent::ThreadsFetched(result));
            }
            WorkerCommand::FetchMessages(thread_id) => {
                let result = api.get_thread_messages(&session, &thread_id, 20);
                let _ = evt_tx.send(WorkerEvent::MessagesFetched(result));
            }
            WorkerCommand::SendDM(thread_id, text) => {
                let result = api.send_dm(&session, &thread_id, &text);
                let _ = evt_tx.send(WorkerEvent::DMSent(result));
                // Auto-refresh after send
                let result = api.get_thread_messages(&session, &thread_id, 20);
                let _ = evt_tx.send(WorkerEvent::MessagesFetched(result));
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
    threads: Vec<DirectThread>,
    thread_list_state: ListState,
    messages: Vec<DirectMessage>,
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

            threads: Vec::new(),
            thread_list_state: ListState::default(),
            messages: Vec::new(),
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
            }
            WorkerEvent::ThreadsFetched(Err(e)) => {
                self.status = format!("error: {}", e);
            }
            WorkerEvent::MessagesFetched(Ok((msgs, title))) => {
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
            WorkerEvent::MessagesFetched(Err(e)) => {
                self.status = format!("error: {}", e);
            }
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
            let title = &t.thread_title;
            let preview = if t.last_message.len() > 60 {
                format!("{}...", &t.last_message[..60])
            } else {
                t.last_message.clone()
            };
            ListItem::new(vec![
                Line::from(Span::styled(
                    title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
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
    let status = Paragraph::new(format!("  {}", app.status))
        .style(Style::default().fg(Color::DarkGray));
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
                    app.messages.clear();
                    app.dm_input.clear();
                    app.status = format!("{}  loading...", thread.thread_title);
                    app.screen = Screen::DMThread(tid.clone());
                    let _ = app.cmd_tx.send(WorkerCommand::FetchMessages(tid));
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
    session: Option<SessionConfig>,
) -> Result<()> {
    let (cmd_tx, _cmd_rx) = mpsc::channel::<WorkerCommand>();

    let needs_login = session.is_none();
    let mut session = session.unwrap_or_default();
    let username = session.username.clone().unwrap_or_else(|| "unknown".to_string());

    let mut app = App::new(username, cmd_tx.clone());

    if needs_login {
        app.screen = Screen::Login;
    }

    // Login loop — runs before worker thread is spawned
    if needs_login {
        loop {
            terminal
                .draw(|frame| draw(frame, &mut app))
                .context("failed to draw frame")?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == event::KeyEventKind::Press {
                        let quit = handle_input(&mut app, key);
                        if quit {
                            return Ok(());
                        }
                    }
                }
            }

            if app.login_status == "login_requested" {
                app.login_status = "logging in...".to_string();
                terminal.draw(|frame| draw(frame, &mut app))?;

                match api.login(&app.login_username, &app.login_password) {
                    Ok(new_session) => {
                        let _ = store.save_session(&new_session);
                        app.username = new_session.username.clone().unwrap_or_default();
                        session = new_session;
                        app.screen = Screen::Home;
                        break;
                    }
                    Err(e) => {
                        app.login_status = format!("failed: {}", e);
                    }
                }
            }
        }
    }

    // Now spawn worker thread with valid session
    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();
    let (evt_tx, evt_rx) = mpsc::channel::<WorkerEvent>();
    app.cmd_tx = cmd_tx;

    // Prefetch DMs
    let _ = app.cmd_tx.send(WorkerCommand::FetchThreads);

    let worker_session = session.clone();
    std::thread::spawn(move || {
        worker_loop(api, store, worker_session, cmd_rx, evt_tx);
    });

    // Main loop
    loop {
        terminal
            .draw(|frame| draw(frame, &mut app))
            .context("failed to draw frame")?;

        while let Ok(evt) = evt_rx.try_recv() {
            app.handle_worker_event(evt);
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    let quit = handle_input(&mut app, key);
                    if quit {
                        return Ok(());
                    }
                }
            }
        }
    }
}
