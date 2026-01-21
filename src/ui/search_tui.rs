use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

use crate::core::{SearchEngine, SearchResult};

pub struct SearchTui {
    search_engine: SearchEngine,
    input: String,
    results: Vec<SearchResult>,
    list_state: ListState,
    mode: Mode,
    status_message: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Input,
    Results,
}

impl SearchTui {
    pub fn new(search_engine: SearchEngine) -> Result<Self> {
        Ok(Self {
            search_engine,
            input: String::new(),
            results: Vec::new(),
            list_state: ListState::default(),
            mode: Mode::Input,
            status_message: None,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.run_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match self.mode {
                    Mode::Input => match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Enter => {
                            if !self.input.is_empty() {
                                self.perform_search();
                            }
                        }
                        KeyCode::Char(c) => {
                            self.input.push(c);
                        }
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Tab => {
                            if !self.results.is_empty() {
                                self.mode = Mode::Results;
                                if self.list_state.selected().is_none() {
                                    self.list_state.select(Some(0));
                                }
                            }
                        }
                        _ => {}
                    },
                    Mode::Results => match key.code {
                        KeyCode::Esc | KeyCode::Tab => {
                            self.mode = Mode::Input;
                        }
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.previous();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.next();
                        }
                        KeyCode::Enter => {
                            if let Some(selected) = self.list_state.selected() {
                                if let Some(result) = self.results.get(selected) {
                                    // Open file in default editor (or just print path)
                                    self.status_message =
                                        Some(format!("Selected: {}", result.path.display()));
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    fn perform_search(&mut self) {
        self.status_message = Some("Searching...".to_string());

        match self.search_engine.search_interactive(&self.input, 20) {
            Ok(results) => {
                if results.is_empty() {
                    self.status_message = Some("No results found".to_string());
                } else {
                    self.status_message = Some(format!("Found {} results", results.len()));
                    self.mode = Mode::Results;
                    self.list_state.select(Some(0));
                }
                self.results = results;
            }
            Err(e) => {
                self.status_message = Some(format!("Error: {}", e));
                self.results.clear();
            }
        }
    }

    fn next(&mut self) {
        if self.results.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.results.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.results.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.results.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),  // Search input
                Constraint::Min(10),    // Results
                Constraint::Length(10), // Preview
                Constraint::Length(1),  // Status bar
            ])
            .split(f.area());

        self.render_input(f, chunks[0]);
        self.render_results(f, chunks[1]);
        self.render_preview(f, chunks[2]);
        self.render_status(f, chunks[3]);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let style = if self.mode == Mode::Input {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let input = Paragraph::new(self.input.as_str()).style(style).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search Query (Enter to search, Tab to results, Esc to quit)"),
        );

        f.render_widget(input, area);

        // Show cursor in input mode
        if self.mode == Mode::Input {
            f.set_cursor_position((area.x + self.input.len() as u16 + 1, area.y + 1));
        }
    }

    fn render_results(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .results
            .iter()
            .map(|r| {
                let cwd = std::env::current_dir().unwrap_or_default();
                let rel_path = r.path.strip_prefix(&cwd).unwrap_or(&r.path);

                let score_pct = r.score * 100.0;
                let line = Line::from(vec![
                    Span::styled(
                        format!("{:.1}%", score_pct),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("./{}", rel_path.display()),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" :{}:{}", r.start_line, r.end_line),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);

                ListItem::new(line)
            })
            .collect();

        let style = if self.mode == Mode::Results {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let results_list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Results (j/k or arrows to navigate)")
                    .style(style),
            )
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(results_list, area, &mut self.list_state);
    }

    fn render_preview(&self, f: &mut Frame, area: Rect) {
        let preview_text = if let Some(selected) = self.list_state.selected() {
            if let Some(result) = self.results.get(selected) {
                result.preview.clone().unwrap_or_default()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let preview = Paragraph::new(Text::raw(preview_text))
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Gray));

        f.render_widget(preview, area);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let status = self.status_message.as_deref().unwrap_or("Ready");

        let status_bar = Paragraph::new(status).style(Style::default().fg(Color::Cyan));

        f.render_widget(status_bar, area);
    }
}
