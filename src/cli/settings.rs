use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::config::Config;

enum SettingValue {
    Bool(bool),
    Number(u64, u64, u64), // value, min, max
}

struct SettingItem {
    label: &'static str,
    description: &'static str,
    value: SettingValue,
    apply: fn(&mut Config, &SettingValue),
}

struct App {
    items: Vec<SettingItem>,
    list_state: ListState,
    status: Option<(String, Color)>,
    dirty: bool,
}

impl App {
    fn from_config(config: &Config) -> Self {
        let items = vec![
            SettingItem {
                label: "Tailscale Binding",
                description: "Bind to Tailscale IP (requires tailscale)",
                value: SettingValue::Bool(config.bind == "tailscale"),
                apply: |c, v| {
                    if let SettingValue::Bool(on) = v {
                        c.bind = if *on { "tailscale".into() } else { "127.0.0.1".into() };
                    }
                },
            },
            SettingItem {
                label: "Yolo Mode",
                description: "Auto-approve all tool calls without confirmation",
                value: SettingValue::Bool(config.yolo_mode),
                apply: |c, v| {
                    if let SettingValue::Bool(on) = v {
                        c.yolo_mode = *on;
                    }
                },
            },
            SettingItem {
                label: "iTerm Integration",
                description: "Enable iTerm2-specific features (badges, marks)",
                value: SettingValue::Bool(config.iterm_enabled),
                apply: |c, v| {
                    if let SettingValue::Bool(on) = v {
                        c.iterm_enabled = *on;
                    }
                },
            },
            SettingItem {
                label: "Port",
                description: "Web UI port (requires restart)",
                value: SettingValue::Number(config.port as u64, 1024, 65535),
                apply: |c, v| {
                    if let SettingValue::Number(n, _, _) = v {
                        c.port = *n as u16;
                    }
                },
            },
            SettingItem {
                label: "Log Retention (days)",
                description: "Number of days to keep session logs",
                value: SettingValue::Number(config.log_retention_days as u64, 1, 365),
                apply: |c, v| {
                    if let SettingValue::Number(n, _, _) = v {
                        c.log_retention_days = *n as u32;
                    }
                },
            },
            SettingItem {
                label: "Max Log Lines",
                description: "Maximum lines stored per session log",
                value: SettingValue::Number(config.max_log_lines as u64, 100, 1_000_000),
                apply: |c, v| {
                    if let SettingValue::Number(n, _, _) = v {
                        c.max_log_lines = *n as usize;
                    }
                },
            },
        ];

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            items,
            list_state,
            status: None,
            dirty: false,
        }
    }

    fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    fn move_up(&mut self) {
        let i = self.selected();
        let prev = if i == 0 { self.items.len() - 1 } else { i - 1 };
        self.list_state.select(Some(prev));
        self.status = None;
    }

    fn move_down(&mut self) {
        let i = self.selected();
        let next = if i >= self.items.len() - 1 { 0 } else { i + 1 };
        self.list_state.select(Some(next));
        self.status = None;
    }

    fn toggle_bool(&mut self) {
        let i = self.selected();
        if let SettingValue::Bool(ref mut v) = self.items[i].value {
            *v = !*v;
            self.dirty = true;

            // Tailscale validation
            if i == 0 && *v {
                match std::process::Command::new("tailscale").arg("version").output() {
                    Ok(output) if output.status.success() => {}
                    _ => {
                        self.status =
                            Some(("tailscale not found — will fallback at runtime".into(), Color::Yellow));
                    }
                }
            }
        }
    }

    fn adjust_number(&mut self, delta: i64) {
        let i = self.selected();
        if let SettingValue::Number(ref mut val, min, max) = self.items[i].value {
            let new = (*val as i64 + delta).clamp(min as i64, max as i64) as u64;
            if new != *val {
                *val = new;
                self.dirty = true;
            }
        }
    }

    fn save(&mut self, config: &mut Config) -> Result<()> {
        for item in &self.items {
            (item.apply)(config, &item.value);
        }
        let path = Config::config_path();
        config.save(&path)?;
        self.dirty = false;
        self.status = Some((format!("Saved to {}", path.display()), Color::Green));
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

        // Main list
        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let marker = if idx == self.selected() { ">> " } else { "   " };
                let val_str = match &item.value {
                    SettingValue::Bool(true) => "● ON".to_string(),
                    SettingValue::Bool(false) => "○ OFF".to_string(),
                    SettingValue::Number(n, _, _) => n.to_string(),
                };
                let padding = 30usize.saturating_sub(item.label.len());
                let text = format!("{}{}{:>pad$}{}", marker, item.label, "", val_str, pad = padding);
                let style = if idx == self.selected() {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(text, style)))
            })
            .collect();

        let list = List::new(list_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Lineforge Settings "),
        );
        frame.render_stateful_widget(list, chunks[0], &mut self.list_state);

        // Description / status bar
        let (desc_text, desc_color) = if let Some((ref msg, color)) = self.status {
            (msg.clone(), color)
        } else {
            let item = &self.items[self.selected()];
            (item.description.to_string(), Color::DarkGray)
        };
        let desc = Paragraph::new(Line::from(Span::styled(
            format!(" {}", desc_text),
            Style::default().fg(desc_color),
        )))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(desc, chunks[1]);

        // Help bar
        let mut help_spans = vec![
            Span::styled(" j/k", Style::default().fg(Color::Cyan)),
            Span::raw(":nav  "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(":toggle  "),
            Span::styled("h/l", Style::default().fg(Color::Cyan)),
            Span::raw(":adjust  "),
            Span::styled("s", Style::default().fg(Color::Cyan)),
            Span::raw(":save  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(":quit"),
        ];
        if self.dirty {
            help_spans.push(Span::raw("  "));
            help_spans.push(Span::styled("[modified]", Style::default().fg(Color::Red)));
        }
        let help = Paragraph::new(Line::from(help_spans));
        frame.render_widget(help, chunks[2]);
    }
}

pub fn run() -> Result<()> {
    let mut config = Config::load(None)?;
    let mut app = App::from_config(&config);

    enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app, &mut config);

    disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    config: &mut Config,
) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if app.dirty {
                        app.save(config)?;
                    }
                    return Ok(());
                }
                KeyCode::Char('j') | KeyCode::Down => app.move_down(),
                KeyCode::Char('k') | KeyCode::Up => app.move_up(),
                KeyCode::Enter | KeyCode::Char(' ') => app.toggle_bool(),
                KeyCode::Char('h') | KeyCode::Left => app.adjust_number(-1),
                KeyCode::Char('l') | KeyCode::Right => app.adjust_number(1),
                KeyCode::Char('s') => {
                    app.save(config)?;
                }
                _ => {}
            }
        }
    }
}
