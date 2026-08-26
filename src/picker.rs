use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute, queue};
use std::io::{stdout, IsTerminal, Write};

pub struct Item {
    pub label: String,
    pub meta: String,
    /// Shown as a badge — used here to mark repos already on disk.
    pub note: Option<String>,
    pub selected: bool,
}

/// Restores the terminal even if the body panics or returns early. Without this a
/// crash mid-render leaves the shell in raw mode with no echo.
struct TermGuard;

impl TermGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, cursor::Hide)?;
        Ok(TermGuard)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

pub fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Every filter term must match somewhere in the label, so "sen py" narrows the way
/// people expect without needing to remember the exact order of words.
fn matches(label: &str, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let l = label.to_lowercase();
    filter
        .split_whitespace()
        .all(|t| l.contains(&t.to_lowercase()))
}

/// Pads to a fixed column so the metadata on the right lines up down the list.
fn pad_to(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(w - n))
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    if w <= 1 {
        return "…".into();
    }
    let mut out: String = s.chars().take(w - 1).collect();
    out.push('…');
    out
}

/// Multi-select list with type-to-filter. Returns the chosen indices, or None if
/// the user backed out.
pub fn pick(title: &str, hint: &str, items: &mut [Item]) -> Result<Option<Vec<usize>>> {
    if items.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let _guard = TermGuard::enter()?;

    let mut filter = String::new();
    let mut cursor_idx: usize = 0;
    let mut scroll: usize = 0;

    loop {
        let (cols, rows) = terminal::size().unwrap_or((100, 30));
        let cols = cols.max(40) as usize;
        let rows = rows.max(10) as usize;
        // header(2) + filter(1) + blank(1) + footer(2)
        let view_h = rows.saturating_sub(6).max(1);

        let visible: Vec<usize> = (0..items.len())
            .filter(|&i| matches(&items[i].label, &filter))
            .collect();
        if cursor_idx >= visible.len() {
            cursor_idx = visible.len().saturating_sub(1);
        }
        if cursor_idx < scroll {
            scroll = cursor_idx;
        }
        if cursor_idx >= scroll + view_h {
            scroll = cursor_idx + 1 - view_h;
        }
        if scroll > visible.len().saturating_sub(1) {
            scroll = 0;
        }

        let chosen = items.iter().filter(|i| i.selected).count();
        let mut out = stdout();
        queue!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

        // --- header ---
        queue!(
            out,
            SetAttribute(Attribute::Bold),
            Print(truncate(title, cols)),
            SetAttribute(Attribute::Reset),
            cursor::MoveToNextLine(1),
            SetForegroundColor(Color::DarkGrey),
            Print(truncate(hint, cols)),
            ResetColor,
            cursor::MoveToNextLine(1),
        )?;

        // --- filter line ---
        queue!(out, SetForegroundColor(Color::DarkGrey), Print("filter "), ResetColor)?;
        if filter.is_empty() {
            queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                Print("(type to narrow)"),
                ResetColor
            )?;
        } else {
            queue!(
                out,
                SetForegroundColor(Color::Cyan),
                Print(&filter),
                ResetColor,
                SetForegroundColor(Color::DarkGrey),
                Print("▏"),
                ResetColor
            )?;
        }
        let counts = format!("  {} shown · {chosen} selected in all", visible.len());
        queue!(
            out,
            SetForegroundColor(Color::DarkGrey),
            Print(truncate(&counts, cols.saturating_sub(20))),
            ResetColor,
            cursor::MoveToNextLine(2),
        )?;

        // --- list ---
        if visible.is_empty() {
            queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                Print("  nothing matches that filter"),
                ResetColor,
                cursor::MoveToNextLine(1)
            )?;
        }
        for row in 0..view_h {
            let vi = scroll + row;
            if vi >= visible.len() {
                break;
            }
            let i = visible[vi];
            let it = &items[i];
            let is_cursor = vi == cursor_idx;

            queue!(out, Print(if is_cursor { "❯ " } else { "  " }))?;
            if it.selected {
                queue!(
                    out,
                    SetForegroundColor(Color::Green),
                    Print("[x] "),
                    ResetColor
                )?;
            } else {
                queue!(
                    out,
                    SetForegroundColor(Color::DarkGrey),
                    Print("[ ] "),
                    ResetColor
                )?;
            }

            let badge = it.note.as_deref().unwrap_or("");
            // Widest badge any row can carry, so present and absent badges do not
            // shift the metadata column relative to each other.
            let badge_w = if items.iter().any(|i| i.note.is_some()) { 12 } else { 0 };
            let meta_w = it.meta.chars().count() + 2;
            // The name is the thing being chosen, so it keeps the space. Metadata is
            // dropped entirely on a narrow terminal rather than squeezing the name
            // down to an unreadable stub.
            let room = cols.saturating_sub(6 + badge_w);
            let show_meta = room > meta_w + 20;
            let name_w = if show_meta { room - meta_w } else { room }.max(10);

            if is_cursor {
                queue!(out, SetAttribute(Attribute::Bold))?;
            }
            let name = truncate(&it.label, name_w);
            let name = if show_meta { pad_to(&name, name_w) } else { name };
            queue!(out, Print(name), SetAttribute(Attribute::Reset))?;

            if badge_w > 0 {
                queue!(
                    out,
                    SetForegroundColor(Color::Cyan),
                    Print(pad_to(&format!("  {badge}"), badge_w)),
                    ResetColor
                )?;
            }
            if show_meta {
                queue!(
                    out,
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("  {}", it.meta)),
                    ResetColor
                )?;
            }
            queue!(out, cursor::MoveToNextLine(1))?;
        }

        // --- footer ---
        let more = if visible.len() > view_h {
            format!("  ({}–{} of {})", scroll + 1, (scroll + view_h).min(visible.len()), visible.len())
        } else {
            String::new()
        };
        queue!(
            out,
            cursor::MoveTo(0, (rows - 2) as u16),
            SetForegroundColor(Color::DarkGrey),
            Print(truncate(
                &format!(
                    "↑↓ move · space toggle · ^a all · ^x none · ^u clear filter · ⏎ confirm · esc cancel{more}"
                ),
                cols
            )),
            ResetColor
        )?;
        out.flush()?;

        // --- input ---
        match event::read()? {
            Event::Key(KeyEvent { code, modifiers, .. }) => {
                let ctrl = modifiers.contains(KeyModifiers::CONTROL);
                match code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c') if ctrl => return Ok(None),
                    KeyCode::Enter => {
                        let sel: Vec<usize> =
                            (0..items.len()).filter(|&i| items[i].selected).collect();
                        return Ok(Some(sel));
                    }
                    KeyCode::Up => cursor_idx = cursor_idx.saturating_sub(1),
                    KeyCode::Char('p') if ctrl => cursor_idx = cursor_idx.saturating_sub(1),
                    KeyCode::Down => {
                        if cursor_idx + 1 < visible.len() {
                            cursor_idx += 1;
                        }
                    }
                    KeyCode::Char('n') if ctrl => {
                        if cursor_idx + 1 < visible.len() {
                            cursor_idx += 1;
                        }
                    }
                    KeyCode::PageUp => cursor_idx = cursor_idx.saturating_sub(view_h),
                    KeyCode::PageDown => {
                        cursor_idx = (cursor_idx + view_h).min(visible.len().saturating_sub(1))
                    }
                    KeyCode::Home => cursor_idx = 0,
                    KeyCode::End => cursor_idx = visible.len().saturating_sub(1),
                    // Repo names never contain a space, so space is free to be the
                    // toggle rather than a filter character.
                    KeyCode::Char(' ') | KeyCode::Tab => {
                        if let Some(&i) = visible.get(cursor_idx) {
                            items[i].selected = !items[i].selected;
                            if cursor_idx + 1 < visible.len() {
                                cursor_idx += 1;
                            }
                        }
                    }
                    KeyCode::Char('a') if ctrl => {
                        for &i in &visible {
                            items[i].selected = true;
                        }
                    }
                    KeyCode::Char('x') if ctrl => {
                        for it in items.iter_mut() {
                            it.selected = false;
                        }
                    }
                    KeyCode::Char('u') if ctrl => {
                        filter.clear();
                        cursor_idx = 0;
                        scroll = 0;
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        cursor_idx = 0;
                        scroll = 0;
                    }
                    KeyCode::Char(c) if !ctrl => {
                        filter.push(c);
                        cursor_idx = 0;
                        scroll = 0;
                    }
                    _ => {}
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

/// A yes/no question on stderr, so it still works when stdout is redirected.
pub fn confirm(question: &str, default_yes: bool) -> Result<bool> {
    use std::io::BufRead;
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    eprint!("{question} {suffix} ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let a = line.trim().to_lowercase();
    Ok(match a.as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Free-text prompt with a default shown in brackets.
pub fn prompt(question: &str, default: &str) -> Result<String> {
    use std::io::BufRead;
    eprint!("{question} [{default}]: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let a = line.trim();
    Ok(if a.is_empty() { default.to_string() } else { a.to_string() })
}

pub fn ago(days: f64) -> String {
    if days > 9000.0 {
        return "never".into();
    }
    let h = days * 24.0;
    if h < 1.0 {
        format!("{}m ago", (h * 60.0).round().max(1.0) as i64)
    } else if h < 24.0 {
        format!("{}h ago", h.round() as i64)
    } else if days < 30.0 {
        format!("{}d ago", days.round() as i64)
    } else if days < 365.0 {
        format!("{}mo ago", (days / 30.0).round() as i64)
    } else {
        format!("{:.1}y ago", days / 365.0)
    }
}
