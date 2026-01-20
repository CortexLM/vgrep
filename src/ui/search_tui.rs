use anyhow::{Context, Result};
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
use std::process::Command;

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

    fn open_file(&mut self, path: &std::path::Path, line: i32) -> Result<()> {
        // Suspend TUI
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        
        let mut command = Command::new(&editor);
        
        // Handle common editors that support +LINE syntax
        if editor.contains("vi") || editor.contains("nano") || editor.contains("emacs") {
             command.arg(format!("+{}", line));
        }
        
        let status = command
            .arg(path)
            .status()
            .context("Failed to open editor")?;

        if !status.success() {
             self.status_message = Some(format!("Editor exited with error: {}", status));
        }

        // Restore TUI
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        
        // Redraw immediately
        // We can't access terminal here directly to redraw, but the main loop will catch up on next iteration.
        // However, to avoid a flash/blank screen until next event, we might want to trigger a redraw if possible,
        // or just let the loop handle it.
        // Since we are returning control to run_loop which calls terminal.draw(), it should be fine.
        
        Ok(())
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
                                // Clone the necessary data to avoid borrowing self.results while calling open_file
                                let result_data = self.results.get(selected).map(|r| (r.path.clone(), r.start_line));
                                
                                if let Some((path, start_line)) = result_data {
                                    // Open file in default editor
                                    if let Err(e) = self.open_file(&path, start_line + 1) {
                                        self.status_message = Some(format!("Error opening file: {}", e));
                                    } else {
                                        self.status_message = Some(format!("Opened: {}", path.display()));
                                        // Force a clear/redraw is implicit as we loop back
                                        terminal.clear()?; 
                                    }
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
                        format!(" :{}:{}", r.start_line + 1, r.end_line + 1),
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
