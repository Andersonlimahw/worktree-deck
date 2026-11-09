use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::error::AppError;
use crate::model::{RunMode, Worktree};
use crate::ops;
use crate::output;

const HELP: &str = "\
worktree-deck

Up/Down or j/k     Move the selection
Home/End or g/G    Jump to the first or last worktree
/                  Edit the filter
Enter              Apply the filter
Esc                Cancel filter, help, or confirmation
Backspace          Delete the previous character
r                  Refresh from git
x                  Remove the selected worktree
Type remove        Confirm a removal, then press Enter
?                  Toggle this help
q                  Quit
Ctrl+C             Quit

The main worktree, a locked worktree, and a worktree with staged,
unstaged, or untracked changes cannot be removed. Removal keeps the branch.
Create a worktree from the CLI so the source revision stays explicit:
  worktree-deck create --path <path> --new-branch <name> --start-point <rev> --yes
";

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, AppError> {
        enable_raw_mode()
            .map_err(|err| AppError::io(format!("failed to enable raw mode: {err}")))?;
        if let Err(err) = execute!(io::stdout(), EnterAlternateScreen, cursor::Hide) {
            restore_terminal();
            return Err(AppError::io(format!(
                "failed to enter the alternate screen: {err}"
            )));
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub(crate) fn run() -> Result<(), AppError> {
    let cwd = std::env::current_dir()
        .map_err(|err| AppError::io(format!("failed to read the current directory: {err}")))?;
    let _guard = TerminalGuard::enter()?;
    let previous = install_panic_hook();
    let result = run_loop(cwd);
    std::panic::set_hook(previous);
    result
}

fn run_loop(cwd: std::path::PathBuf) -> Result<(), AppError> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .map_err(|err| AppError::io(format!("failed to start the terminal ui: {err}")))?;
    let mut state = UiState::new(cwd);
    terminal
        .draw(|frame| draw(frame, &state))
        .map_err(|err| AppError::io(format!("failed to draw the terminal ui: {err}")))?;
    state.reload();
    event_loop(&mut terminal, &mut state)
}

fn install_panic_hook() -> Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        restore_terminal();
        eprintln!("error: worktree-deck panicked: {info}");
    }));
    previous
}

fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    let _ = disable_raw_mode();
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut UiState,
) -> Result<(), AppError> {
    loop {
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|err| AppError::io(format!("failed to draw the terminal ui: {err}")))?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|err| AppError::io(format!("failed to read the terminal: {err}")))?
        {
            continue;
        }
        let event = event::read()
            .map_err(|err| AppError::io(format!("failed to read the terminal: {err}")))?;
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if on_key(state, key.code, key.modifiers) {
                break;
            }
        }
    }
    Ok(())
}

struct UiState {
    cwd: std::path::PathBuf,
    worktrees: Vec<Worktree>,
    filter: String,
    filter_draft: String,
    editing_filter: bool,
    selected: usize,
    error: Option<String>,
    status: Option<String>,
    confirm: bool,
    confirm_draft: String,
    help: bool,
    loading: bool,
    failed: bool,
}

impl UiState {
    fn new(cwd: std::path::PathBuf) -> Self {
        Self {
            cwd,
            worktrees: Vec::new(),
            filter: String::new(),
            filter_draft: String::new(),
            editing_filter: false,
            selected: 0,
            error: None,
            status: None,
            confirm: false,
            confirm_draft: String::new(),
            help: false,
            loading: true,
            failed: false,
        }
    }

    fn filtered(&self) -> Vec<usize> {
        matching_indices(&self.worktrees, &self.filter)
    }

    fn selected_worktree(&self) -> Option<&Worktree> {
        let filtered = self.filtered();
        filtered
            .get(self.selected)
            .and_then(|index| self.worktrees.get(*index))
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(len - 1) as isize;
        let next = (current + delta).clamp(0, len as isize - 1);
        self.selected = next as usize;
    }

    fn select_edge(&mut self, first: bool) {
        let len = self.filtered().len();
        self.selected = if len == 0 || first { 0 } else { len - 1 };
    }

    fn reload(&mut self) {
        let previous = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone());
        match ops::list(&self.cwd) {
            Ok(worktrees) => {
                self.worktrees = worktrees;
                self.loading = false;
                self.failed = false;
                self.error = None;
                self.restore_selection(previous.as_deref());
            }
            Err(err) => {
                self.loading = false;
                self.failed = self.worktrees.is_empty();
                self.error = Some(err.message);
            }
        }
    }

    fn restore_selection(&mut self, previous: Option<&str>) {
        let filtered = self.filtered();
        if let Some(path) = previous {
            if let Some(position) = filtered
                .iter()
                .position(|index| self.worktrees[*index].path == path)
            {
                self.selected = position;
                return;
            }
        }
        if filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= filtered.len() {
            self.selected = filtered.len() - 1;
        }
    }

    fn begin_remove(&mut self) {
        let Some(worktree) = self.selected_worktree().cloned() else {
            self.error = Some("No worktree is selected.".to_string());
            return;
        };
        if let Some(block) = ops::removal_block(&worktree, Path::new(&worktree.path).is_dir()) {
            self.error = Some(ops::removal_message(block, &worktree));
            self.confirm = false;
            return;
        }
        self.confirm = true;
        self.confirm_draft.clear();
        self.help = false;
        self.editing_filter = false;
        self.error = None;
        self.status = None;
    }

    fn confirm_remove(&mut self) {
        if self.confirm_draft != "remove" {
            self.error = Some(
                "Confirmation did not match. Type remove and press Enter to confirm.".to_string(),
            );
            self.confirm = false;
            self.confirm_draft.clear();
            return;
        }
        let Some(path) = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
        else {
            self.confirm = false;
            return;
        };
        match ops::remove(
            &self.cwd,
            &path,
            RunMode {
                confirmed: true,
                dry_run: false,
            },
        ) {
            Ok(outcome) => {
                let kept = outcome.branch_kept.unwrap_or_else(|| "none".to_string());
                self.confirm = false;
                self.confirm_draft.clear();
                self.reload();
                self.status = Some(format!("Removed {}. Branch kept: {kept}.", outcome.path));
            }
            Err(err) => {
                self.confirm = false;
                self.confirm_draft.clear();
                self.reload();
                self.error = Some(err.message);
            }
        }
    }
}

fn on_key(state: &mut UiState, code: KeyCode, modifiers: KeyModifiers) -> bool {
    if modifiers.contains(KeyModifiers::CONTROL)
        && matches!(code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return true;
    }
    if state.help {
        match code {
            KeyCode::Esc | KeyCode::Char('?') => state.help = false,
            KeyCode::Char('q') => return true,
            _ => {}
        }
        return false;
    }
    if state.confirm {
        return on_confirm_key(state, code);
    }
    if state.editing_filter {
        return on_filter_key(state, code);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => true,
        KeyCode::Char('j') | KeyCode::Down => {
            state.move_selection(1);
            false
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.move_selection(-1);
            false
        }
        KeyCode::Char('g') | KeyCode::Home => {
            state.select_edge(true);
            false
        }
        KeyCode::Char('G') | KeyCode::End => {
            state.select_edge(false);
            false
        }
        KeyCode::Char('/') => {
            state.editing_filter = true;
            state.filter_draft.clone_from(&state.filter);
            state.confirm = false;
            false
        }
        KeyCode::Char('r') => {
            state.reload();
            false
        }
        KeyCode::Char('x') => {
            state.begin_remove();
            false
        }
        KeyCode::Char('?') => {
            state.help = true;
            false
        }
        _ => false,
    }
}

fn on_filter_key(state: &mut UiState, code: KeyCode) -> bool {
    match code {
        KeyCode::Enter => {
            state.filter.clone_from(&state.filter_draft);
            state.editing_filter = false;
            state.selected = 0;
            false
        }
        KeyCode::Esc => {
            state.editing_filter = false;
            false
        }
        KeyCode::Backspace => {
            state.filter_draft.pop();
            false
        }
        KeyCode::Char(ch) if !ch.is_control() => {
            state.filter_draft.push(ch);
            false
        }
        _ => false,
    }
}

fn on_confirm_key(state: &mut UiState, code: KeyCode) -> bool {
    match code {
        KeyCode::Enter => {
            state.confirm_remove();
            false
        }
        KeyCode::Esc => {
            state.confirm = false;
            state.confirm_draft.clear();
            state.status = Some("Removal cancelled.".to_string());
            false
        }
        KeyCode::Backspace => {
            state.confirm_draft.pop();
            false
        }
        KeyCode::Char(ch) if !ch.is_control() => {
            state.confirm_draft.push(ch);
            false
        }
        _ => false,
    }
}

fn draw(frame: &mut Frame, state: &UiState) {
    let area = frame.area();
    if state.help {
        frame.render_widget(
            Paragraph::new(HELP).block(Block::default().title(" Help ").borders(Borders::ALL)),
            area,
        );
        return;
    }
    if area.height < 4 || area.width < 12 {
        frame.render_widget(Paragraph::new("Terminal is too small. q quits."), area);
        return;
    }
    if state.loading {
        frame.render_widget(Paragraph::new("Loading worktrees..."), area);
        return;
    }
    if state.failed {
        let message = state.error.as_deref().unwrap_or("git could not be queried");
        frame.render_widget(
            Paragraph::new(format!("Error: {message}\n\nr  refresh\nq  quit")),
            area,
        );
        return;
    }

    let columns = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(Paragraph::new(header(state)), columns[0]);

    let body = Layout::default()
        .direction(if area.width < 80 {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(columns[1]);
    draw_list(frame, state, body[0]);
    draw_detail(frame, state, body[1]);
    frame.render_widget(
        Paragraph::new(message_line(state)).wrap(Wrap { trim: false }),
        columns[2],
    );
    frame.render_widget(Paragraph::new(footer(state)), columns[3]);
}

fn draw_list(frame: &mut Frame, state: &UiState, area: Rect) {
    let filtered = state.filtered();
    let inner_height = area.height.saturating_sub(2) as usize;
    let (start, end) = visible_window(filtered.len(), state.selected, inner_height);
    let lines = if filtered.is_empty() {
        vec![Line::from("No matching worktrees.")]
    } else {
        (start..end)
            .map(|row| {
                let worktree = &state.worktrees[filtered[row]];
                let marker = if row == state.selected { ">" } else { " " };
                let style = if row == state.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(
                    format!("{marker} {}", summary(worktree)),
                    style,
                ))
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title(" Worktrees ").borders(Borders::ALL)),
        area,
    );
}

fn draw_detail(frame: &mut Frame, state: &UiState, area: Rect) {
    let text = match state.selected_worktree() {
        Some(worktree) => output::format_worktree(worktree),
        None => "No worktree selected.".to_string(),
    };
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(" Details ").borders(Borders::ALL)),
        area,
    );
}

fn header(state: &UiState) -> String {
    let shown = state.filtered().len();
    let filter = if state.editing_filter {
        format!("(editing) {}", state.filter_draft)
    } else if state.filter.is_empty() {
        "(none)".to_string()
    } else {
        state.filter.clone()
    };
    format!("worktree-deck  {shown} shown  filter: {filter}")
}

fn footer(state: &UiState) -> String {
    if state.confirm {
        format!(
            "Type remove and press Enter to confirm removal. Draft: {}  Esc cancels",
            state.confirm_draft
        )
    } else if state.editing_filter {
        "Enter applies the filter. Esc cancels. Backspace deletes.".to_string()
    } else {
        "j/k move   / filter   r refresh   x remove   ? help   q quit".to_string()
    }
}

fn message_line(state: &UiState) -> String {
    if let Some(error) = &state.error {
        format!("Error: {error}")
    } else if let Some(status) = &state.status {
        format!("Status: {status}")
    } else {
        "Ready.".to_string()
    }
}

fn summary(worktree: &Worktree) -> String {
    let role = if worktree.is_main { "main" } else { "linked" };
    let branch = worktree.branch.as_deref().unwrap_or(if worktree.detached {
        "(detached)"
    } else {
        "(none)"
    });
    format!(
        "{role}  {branch}  {}  {}",
        flag_summary(worktree),
        worktree.path
    )
}

fn flag_summary(worktree: &Worktree) -> String {
    let mut flags = Vec::new();
    if worktree.locked {
        flags.push("locked");
    }
    if worktree.prunable {
        flags.push("prunable");
    }
    match &worktree.changes {
        Some(changes) if changes.is_dirty() => flags.push("dirty"),
        None => flags.push("status-unavailable"),
        Some(_) => {}
    }
    if flags.is_empty() {
        "clean".to_string()
    } else {
        flags.join(",")
    }
}

pub(crate) fn matching_indices(worktrees: &[Worktree], filter: &str) -> Vec<usize> {
    let needle = filter.to_lowercase();
    worktrees
        .iter()
        .enumerate()
        .filter_map(|(index, worktree)| {
            if needle.is_empty() {
                return Some(index);
            }
            let branch = worktree.branch.as_deref().unwrap_or("detached");
            if worktree.path.to_lowercase().contains(&needle)
                || branch.to_lowercase().contains(&needle)
            {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn visible_window(len: usize, selected: usize, height: usize) -> (usize, usize) {
    if len == 0 || height == 0 {
        return (0, 0);
    }
    let selected = selected.min(len - 1);
    let mut start = selected.saturating_sub(height / 2);
    if start + height > len {
        start = len.saturating_sub(height);
    }
    (start, (start + height).min(len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChangeCounts;

    fn sample(path: &str, branch: &str) -> Worktree {
        Worktree {
            path: path.to_string(),
            head: "abc".to_string(),
            branch: Some(branch.to_string()),
            ref_name: Some(format!("refs/heads/{branch}")),
            detached: false,
            bare: false,
            is_main: false,
            locked: false,
            lock_reason: None,
            prunable: false,
            prunable_reason: None,
            changes: Some(ChangeCounts {
                staged: 0,
                unstaged: 0,
                untracked: 0,
            }),
        }
    }

    #[test]
    fn filter_matches_path_or_branch_without_case() {
        let worktrees = vec![sample("/tmp/Árvore", "Feature"), sample("/repo", "main")];
        assert_eq!(matching_indices(&worktrees, "feature"), vec![0]);
        assert_eq!(matching_indices(&worktrees, "ÁRVORE"), vec![0]);
        assert_eq!(matching_indices(&worktrees, ""), vec![0, 1]);
    }

    #[test]
    fn window_keeps_the_selection_visible() {
        assert_eq!(visible_window(10, 9, 3), (7, 10));
        assert_eq!(visible_window(10, 0, 3), (0, 3));
        assert_eq!(visible_window(2, 1, 5), (0, 2));
        assert_eq!(visible_window(0, 0, 4), (0, 0));
    }
}
